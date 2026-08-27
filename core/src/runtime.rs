//! 把配置、HID 设备、语音、按键注入串起来。

use crate::config::{Action, ActionType, Config, VoiceMode};
use crate::device::*;
use crate::inject::{new_injector, Injector};
use crate::keys::{passthrough, Key};
use crate::voice::{VoiceSink, OPUS_FRAME, OPUS_RATE};
use anyhow::{anyhow, Context, Result};
use parking_lot::{Mutex, RwLock};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub enum Event {
    /// 按键事件 + 处理结果描述
    Key {
        key: Key,
        down: bool,
        result: String,
    },
    /// 未识别的 report（学习/调试）
    Raw {
        report_id: u8,
        data: Vec<u8>,
    },
    /// 学习模式下捕获到的键
    Learned(Key),
    /// 原始按键边沿（trace_keys 开时每个 down/up 都发，用来诊断长按瞬断）
    KeyEdge {
        key: Key,
        down: bool,
    },
    Log(String),
    Connected {
        product: String,
        serial: String,
    },
    Disconnected(String),
}

pub struct Status {
    pub connected: AtomicBool,
    pub battery: AtomicI32,
    pub mic_on: AtomicBool,
    pub audio_frames: AtomicU64,
}

impl Default for Status {
    fn default() -> Self {
        Self {
            connected: AtomicBool::new(false),
            battery: AtomicI32::new(0),
            mic_on: AtomicBool::new(false),
            audio_frames: AtomicU64::new(0),
        }
    }
}

pub struct Runtime {
    pub cfg: Arc<RwLock<Config>>,
    pub status: Arc<Status>,
    pub inj: Arc<dyn Injector>,
    pub voice: Arc<Mutex<Option<Arc<VoiceSink>>>>,
    pub learn: Arc<AtomicBool>,
    /// 当前按下的键，UI 用来让图上按钮跟着亮
    pub pressed: Arc<Mutex<HashSet<Key>>>,
    /// 自动切输入设备时记下的原设备，说完切回去
    pub prev_input: Arc<Mutex<Option<u32>>>,
    /// tap 看到的最近的非字符事件 (时刻, 键码)，用来跟 HID 按键做时间关联
    pub recent_ev: Arc<Mutex<std::collections::VecDeque<(Instant, i64, i64)>>>,
    /// 遥控器最近一次发出键报告的时间。屏蔽只在「这一下确实来自遥控器」时生效 ——
    /// 键码跟 Mac 自带键盘是同一套，光看键码必然把用户自己的键一起吞掉。
    pub last_hid_key: Arc<Mutex<Option<Instant>>>,
    /// 关掉后：开局不补关麦、自愈也不发关麦。诊断麦克风时要用 ——
    /// 不然分不清「设备不吐流」和「我们自己把它关掉了」。
    pub auto_mic_off: Arc<AtomicBool>,
    /// 打开后：**每一条**报文都往外发 `Event::Raw`（平时只有 vendor 报文才发）。
    /// 换遥控器时要看原始字节才能判断按键到底发了什么。
    pub raw_all: Arc<AtomicBool>,
    /// 打开后：每个派生出来的 down/up 边沿都发 `Event::KeyEdge`（诊断长按用）
    pub trace_keys: Arc<AtomicBool>,
    /// 待下发的 OUTPUT 报文。读线程每圈取一次 —— 设备句柄在那个线程里，
    /// 外面拿不到，只能这样递进去。
    pub pending_writes: Arc<Mutex<Vec<Vec<u8>>>>,
    /// 见过哪些 report id、各多少条。换一款遥控器时靠它判断
    /// 「语音通路是不是我们认识的那条」—— 有 0xF0 才说明音频流对得上。
    pub seen_rids: Arc<Mutex<std::collections::BTreeMap<u8, u64>>>,
    /// 遥控器现在是否还按着键。macOS 会给按住的键**自动重复**发事件，
    /// 而遥控器只发一次 HID 报告 —— 光靠时间窗口会让重复事件全部漏出去
    /// （麦克风键漏一个就把 Spotlight 拉起来）。按住期间一律吞。
    pub hid_key_held: Arc<AtomicBool>,
    /// 听写录音缓冲。Some 表示正在录 —— 读线程会把解码后的 PCM 也塞进来。
    pub dictating: Arc<Mutex<Option<crate::stt::Recorder>>>,
    /// 正在录音到文件（按一下开始、再按一下停止）
    pub recording: Arc<Mutex<Option<crate::recorder::Rec>>>,
    /// 学到的要吞掉的键码
    pub learned: Arc<Mutex<Vec<i64>>>,
    tap: Arc<Mutex<Option<crate::tap::Tap>>>,
    pub descriptor: Arc<Mutex<Vec<u8>>>,
    stop: Arc<AtomicBool>,
    tx: Sender<Event>,
}

impl Runtime {
    pub fn new(cfg: Config) -> (Self, Receiver<Event>) {
        let (tx, rx) = channel();
        // 上次记下的电量先摆上去，别让界面空着等下一次上报
        let status = Status::default();
        if let Some(b) = cfg.settings.last_battery {
            status.battery.store(b, Ordering::Relaxed);
        }
        (
            Self {
                cfg: Arc::new(RwLock::new(cfg)),
                status: Arc::new(status),
                inj: Arc::from(new_injector()),
                voice: Arc::new(Mutex::new(None)),
                learn: Arc::new(AtomicBool::new(false)),
                pressed: Arc::new(Mutex::new(HashSet::new())),
                prev_input: Arc::new(Mutex::new(None)),
                recent_ev: Arc::new(Mutex::new(std::collections::VecDeque::new())),
                last_hid_key: Arc::new(Mutex::new(None)),
                hid_key_held: Arc::new(AtomicBool::new(false)),
                auto_mic_off: Arc::new(AtomicBool::new(true)),
                raw_all: Arc::new(AtomicBool::new(false)),
                trace_keys: Arc::new(AtomicBool::new(false)),
                pending_writes: Arc::new(Mutex::new(Vec::new())),
                seen_rids: Arc::new(Mutex::new(std::collections::BTreeMap::new())),
                dictating: Arc::new(Mutex::new(None)),
                recording: Arc::new(Mutex::new(None)),
                learned: Arc::new(Mutex::new(Vec::new())),
                tap: Arc::new(Mutex::new(None)),
                descriptor: Arc::new(Mutex::new(Vec::new())),
                stop: Arc::new(AtomicBool::new(false)),
                tx,
            },
            rx,
        )
    }

    pub fn log(&self, s: impl Into<String>) {
        let _ = self.tx.send(Event::Log(s.into()));
    }

    /// 建好语音链路但**不开麦**。
    ///
    /// 开麦是热的 —— 发一次 MIC_ON 遥控器就一直吐流，蓝灯一直闪，还费电。
    /// 所以只在真的要说话时才开（见 `gate_voice`），平时只把 sink 准备好。
    pub fn ensure_voice(&self) -> Result<()> {
        if self.voice.lock().is_some() {
            return Ok(());
        }
        let (dev, gain) = {
            let c = self.cfg.read();
            (c.voice.device.clone(), c.voice.gain)
        };
        let sink = Arc::new(VoiceSink::start(&dev, gain)?);
        self.log(format!(
            "语音链路就绪 -> {} @ {} Hz / {} 声道（麦克风按需开）",
            sink.device_name, sink.out_rate, sink.out_channels
        ));
        *self.voice.lock() = Some(sink);
        Ok(())
    }

    pub fn start_voice(&self) -> Result<()> {
        self.stop_voice();
        let (dev, gain, mode) = {
            let c = self.cfg.read();
            (c.voice.device.clone(), c.voice.gain, c.voice.mode)
        };
        let sink = Arc::new(VoiceSink::start(&dev, gain)?);
        if mode == VoiceMode::Always {
            sink.set_passing(true);
        }
        self.log(format!(
            "语音已启动 -> {} @ {} Hz / {} 声道",
            sink.device_name, sink.out_rate, sink.out_channels
        ));
        *self.voice.lock() = Some(sink);
        self.set_mic(true);
        Ok(())
    }

    pub fn stop_voice(&self) {
        *self.voice.lock() = None;
        self.set_mic(false);
    }

    /// 开始 / 停止说话。一次做齐三件事：开麦、把音频送进虚拟声卡、
    /// 把系统默认输入切到虚拟声卡（松开后还原）。界面上的「测试输入」用它。
    pub fn set_talking(&self, on: bool) -> bool {
        let Some(sink) = self.voice.lock().clone() else {
            return false;
        };
        gate_voice(&self.cfg, &self.status, &sink, &self.prev_input, on, false);
        true
    }

    /// 界面上的听写开关（测试面板用）
    pub fn set_dictating(&self, on: bool) -> String {
        gate_dictation(
            &self.cfg,
            &self.status,
            &self.inj,
            &self.dictating,
            &self.tx,
            on,
        )
    }

    /// 按键测绘模式。开着时每次按下只上报 `Event::Learned(key)`，不执行任何动作 ——
    /// 换一款遥控器时用它重新认键。
    pub fn set_learn(&self, on: bool) {
        self.learn.store(on, Ordering::Relaxed);
    }

    /// 实时电平（0~1），界面画电平条用
    pub fn level(&self) -> f32 {
        if let Some(r) = self.dictating.lock().as_ref() {
            return r.level();
        }
        self.voice.lock().as_ref().map(|s| s.level()).unwrap_or(0.0)
    }

    pub fn voice_sink(&self) -> Option<Arc<VoiceSink>> {
        self.voice.lock().clone()
    }

    /// 启动 HID 读线程。语音由调用方决定何时 start_voice()。
    /// 起事件 tap，把遥控器按键在系统那边的默认行为吞掉。
    ///
    /// 为什么这么绕：独占打开 HID 要 root，所以系统和我们会同时收到遥控器的键。
    /// 麦克风键 usage 是 Consumer 0x0221 (AC Search)，macOS 自己就弹 Spotlight，
    /// 而且跟 ⌘Space 那个符号热键无关（实测把它 disable 了照样弹），只能在事件层拦。
    ///
    /// 判定分两层：
    /// 1. **键码**要在屏蔽表里（内置 0xb1 麦克风键，加上学到的）；
    /// 2. **必须确实来自遥控器** —— 键码跟 Mac 自带键盘是同一套（方向键、搜索键
    ///    都是），光看键码就吞会把用户自己键盘上的键一起吃掉。定性信号是遥控器的
    ///    HID 流：真是它按的就一定有键报告。两条通路并行、谁先到不确定，所以
    ///    回调里允许最多等 15ms（tap 回调可以短暂阻塞，这点延迟感知不到）。
    ///
    /// 别用 `kCGKeyboardEventKeyboardType` 区分设备 —— 实测每个事件都不一样，
    /// 不是稳定的设备 id。
    #[cfg(target_os = "macos")]
    pub fn start_tap(&self) -> Result<()> {
        use crate::tap;
        if self.tap.lock().is_some() {
            return Ok(());
        }
        if !self.cfg.read().settings.suppress_os_keys {
            return Ok(());
        }
        // 内置已知码 + 用户这边学到的，合起来
        let mut codes: Vec<i64> = tap::BUILTIN_SUPPRESS.to_vec();
        for c in &self.cfg.read().settings.suppress_codes {
            if !codes.contains(c) {
                codes.push(*c);
            }
        }
        *self.learned.lock() = codes;

        let learned = self.learned.clone();
        let recent = self.recent_ev.clone();
        let status = self.status.clone();
        let last_hid = self.last_hid_key.clone();
        let held_flag = self.hid_key_held.clone();
        let last_say: Arc<Mutex<Option<(i64, bool)>>> = Arc::new(Mutex::new(None));
        let t = tap::spawn(
            &[tap::EV_KEY_DOWN, tap::EV_KEY_UP, tap::EV_SYSTEM_DEFINED],
            false, // 要拦
            Box::new(move |ev| {
                // 只认真实硬件发的事件：我们自己（和别的 app）注入的按键
                // 既不学也不吞，否则会把自己发的键屏蔽掉
                if !tap::is_hardware(ev) || !tap::is_non_character(ev) {
                    return false;
                }
                // 遥控器没连着就别拦 —— 这些码跟 Mac 自带键盘的功能键是同一套，
                // 一直拦着会把你自己键盘的搜索键也吞了
                if !status.connected.load(Ordering::Relaxed) {
                    return false;
                }
                // 记进环形缓冲供 HID 线程回看（只留最近 1 秒）
                {
                    let mut r = recent.lock();
                    let now = Instant::now();
                    r.push_back((now, ev.code, ev.kb_type));
                    while r
                        .front()
                        .map(|(t, _, _)| now.duration_since(*t) > Duration::from_secs(1))
                        == Some(true)
                    {
                        r.pop_front();
                    }
                }
                // 键码对不上就直接放行
                if !learned.lock().contains(&ev.code) {
                    return false;
                }
                // 到这儿说明「这个码我们想接管」，但**还不知道是谁按的**。
                // 键码跟 Mac 自带键盘是同一套（方向键、搜索键都是），
                // 光看键码就吞会把用户自己键盘上的键一起吃掉 —— 这是不可接受的。
                // 唯一可靠的定性信号是遥控器的 HID 流：真是遥控器按的，
                // 它一定会发键报告。两条通路并行，谁先到不确定，所以这里
                // 允许**最多等 15ms**（tap 回调可以短暂阻塞，这点延迟感知不到）。
                // 别用 kCGKeyboardEventKeyboardType 去区分设备 ——
                // 实测那个字段每个事件都不一样，不是稳定的设备 id。
                let fresh =
                    |win: Duration| last_hid.lock().map(|t| t.elapsed() < win).unwrap_or(false);
                // 按住期间 macOS 会疯狂重复发事件，同样的判定只打一次日志
                let dbg = std::env::var_os("FIREVIBE_TAP_DEBUG").is_some()
                    && last_say.lock().replace((ev.code, true)) != Some((ev.code, true));
                // 遥控器还按着 → 这些都是 macOS 的自动重复，全吞
                if held_flag.load(Ordering::Relaxed) {
                    if dbg {
                        eprintln!("[tap] 0x{:x} 吞掉（遥控器按着，吞自动重复）", ev.code);
                    }
                    return true;
                }
                if fresh(Duration::from_millis(120)) {
                    if dbg {
                        eprintln!("[tap] 0x{:x} 吞掉（HID 已到，等了 0ms）", ev.code);
                    }
                    return true;
                }
                for i in 1..=8 {
                    std::thread::sleep(Duration::from_millis(2));
                    if fresh(Duration::from_millis(120)) {
                        if dbg {
                            eprintln!("[tap] 0x{:x} 吞掉（等了 {}ms）", ev.code, i * 2);
                        }
                        return true;
                    }
                }
                if dbg {
                    eprintln!("[tap] 0x{:x} 放行（16ms 内没有遥控器 HID 活动）", ev.code);
                }
                false
            }),
            None,
        )?;
        *self.tap.lock() = Some(t);
        self.log(format!(
            "已接管系统默认行为，屏蔽键码 {:?}（仅遥控器连接时生效）",
            self.learned
                .lock()
                .iter()
                .map(|c| format!("0x{c:x}"))
                .collect::<Vec<_>>()
        ));
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    pub fn start_tap(&self) -> Result<()> {
        Ok(())
    }

    /// 按配置下发 / 清掉 HID 设备层映射。
    ///
    /// 每次启动都调一次：设了就重下（幂等，顺带盖掉上次异常退出的残留），
    /// 没设就清掉。这是进程外的系统状态，不主动收拾会留给用户一颗变成修饰键的按钮。
    pub fn sync_hid_remap(&self) -> Option<String> {
        // ⚠️ hidremap 有自己的一份 VID/PID（默认是出厂那台 0x0421），映射只对匹配的
        // 设备生效。以前只有「配对新遥控器」的流程会 set_ids —— 启动时按配置换过
        // 遥控器的，映射就一直下发给一台没连的设备，麦克风键没被接管、Spotlight 照弹。
        // 在这里对齐，所有调用点就都对了。
        let (vid, pid) = self.cfg.read().device_ids();
        crate::hidremap::set_ids(vid, pid);
        let want = self.cfg.read().mic_remap_key();
        match want {
            Some(k) if !k.is_empty() => match crate::hidremap::apply(&k) {
                Ok(()) => Some(format!("麦克风键已在硬件层映射成 {k}")),
                Err(e) => Some(format!("硬件层映射失败: {e}")),
            },
            _ => {
                crate::hidremap::clear();
                None
            }
        }
    }

    pub fn start(&self) -> Result<()> {
        let exclusive = self.cfg.read().exclusive;
        let api = hidapi::HidApi::new().context("hidapi 初始化失败")?;
        #[cfg(target_os = "macos")]
        api.set_open_exclusive(exclusive);
        // 错误分类用 ASCII 前缀，别让界面去匹配中文 ——
        // 原来消息里永远带「输入监控」四个字，结果「设备没连上」也被显示成权限问题。
        let (vid, pid) = self.cfg.read().device_ids();
        let dev = api.open(vid, pid).map_err(|e| {
            let raw = e.to_string();
            let kind = if raw.contains("not permitted") || raw.contains("0xE00002E2") {
                "HID_NOT_PERMITTED"
            } else if raw.contains("No HID devices") || raw.contains("not found") {
                "HID_NOT_FOUND"
            } else {
                "HID_ERROR"
            };
            anyhow!("{kind}: {raw}")
        })?;

        let product = dev.get_product_string().ok().flatten().unwrap_or_default();
        let serial = dev
            .get_serial_number_string()
            .ok()
            .flatten()
            .unwrap_or_default();
        let mut dbuf = vec![0u8; 4096];
        if let Ok(n) = dev.get_report_descriptor(&mut dbuf) {
            *self.descriptor.lock() = dbuf[..n].to_vec();
        }
        self.status.connected.store(true, Ordering::Relaxed);
        let _ = self.tx.send(Event::Connected {
            product: product.clone(),
            serial,
        });
        self.log(if exclusive {
            "已独占打开设备 —— 系统收不到原始按键，映射不会重复触发"
        } else {
            "共享模式打开 —— 系统同时会收到原始按键"
        });

        let cfg = self.cfg.clone();
        let status = self.status.clone();
        let inj = self.inj.clone();
        let voice = self.voice.clone();
        let learn = self.learn.clone();
        let pressed = self.pressed.clone();
        let prev_input = self.prev_input.clone();
        let recent_ev = self.recent_ev.clone();
        let learned_codes = self.learned.clone();
        let last_hid_key = self.last_hid_key.clone();
        let hid_key_held = self.hid_key_held.clone();
        let seen_rids = self.seen_rids.clone();
        let raw_all = self.raw_all.clone();
        let auto_mic_off = self.auto_mic_off.clone();
        let trace_keys = self.trace_keys.clone();
        let pending_writes = self.pending_writes.clone();
        let dictating = self.dictating.clone();
        let recording = self.recording.clone();
        let stop = self.stop.clone();
        let tx = self.tx.clone();

        std::thread::Builder::new()
            .name("firevibe-hid".into())
            .spawn(move || {
                let mut dec = match opus::Decoder::new(OPUS_RATE, opus::Channels::Mono) {
                    Ok(d) => d,
                    Err(e) => {
                        let _ = tx.send(Event::Log(format!("opus 解码器建不起来: {e}")));
                        return;
                    }
                };
                let mut pcm = vec![0i16; OPUS_FRAME * 6];
                let mut buf = [0u8; 128];
                let mut active: HashSet<Key> = HashSet::new();
                // 开局补一发关麦：上次进程若被强杀，遥控器会一直热着、50 帧/秒吐音频，
                // 一两天吃掉 30% 电；而 `mic_now != mic_was` 两边都 false 时不触发，不补关不掉。
                // （关麦命令有效，前提是本进程有「输入监控」授权 —— 见 device.rs。）
                let _ = dev.write(&MIC_OFF);

                // ── 探一次开麦模型 ──
                // 判据：**没人碰遥控器的时候发 MIC_ON，出不出流**。
                // 出流 = 热麦克风（它压根不看按键）；不出流 = PTT（只有按住才出）。
                // 结果存进配置，换设备时会被清掉重探（见 UI 的 pick_device）。
                //
                // ⚠️ 判成 PTT 之后**照样继续发 MIC_ON/keepalive** —— 对 PTT 无害，
                // 而万一判错（比如探测时遥控器正好睡着了）也不会把热麦克风弄瘫。
                // 判型只用来关掉那个没意义的自愈关麦、和在界面上提醒绑定方式。
                if cfg.read().settings.mic_model == crate::config::MicModel::Unknown {
                    let base = status.audio_frames.load(Ordering::Relaxed);
                    let _ = dev.write(&MIC_ON);
                    let t0 = Instant::now();
                    let mut n = 0u32;
                    let mut b = [0u8; 128];
                    while t0.elapsed() < Duration::from_millis(1500) {
                        if let Ok(len) = dev.read_timeout(&mut b, 50) {
                            if len > 0 && b[0] == RID_AUDIO {
                                n += 1;
                            }
                        }
                    }
                    let _ = dev.write(&MIC_OFF);
                    let model = if n > 3 {
                        crate::config::MicModel::Hot
                    } else {
                        crate::config::MicModel::Ptt
                    };
                    {
                        let mut c = cfg.write();
                        c.settings.mic_model = model;
                        let _ = c.save();
                    }
                    let _ = tx.send(Event::Log(format!(
                        "开麦模型：{}（静默发 MIC_ON 收到 {n} 帧）",
                        match model {
                            crate::config::MicModel::Hot => "热麦克风 —— 发命令就一直出流",
                            crate::config::MicModel::Ptt =>
                                "按住才出流 —— 麦克风键要绑「按住」模式",
                            _ => "未知",
                        }
                    )));
                    status.audio_frames.store(base, Ordering::Relaxed);
                }

                let mut mic_was = false;
                let mut last_ka = Instant::now();
                let mut idle_frames = status.audio_frames.load(Ordering::Relaxed);
                let mut idle_at = Instant::now();
                // 长按状态机：按下时间 + 是否已触发长按
                let mut held: HashMap<Key, (Instant, bool)> = HashMap::new();

                loop {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    // 外面递进来的 OUTPUT 报文（--probe-all 的三轮对照用来试开麦）
                    {
                        let todo: Vec<Vec<u8>> = std::mem::take(&mut *pending_writes.lock());
                        for b in todo {
                            let hex: String =
                                b.iter().map(|x| format!("{x:02X} ")).collect();
                            let msg = match dev.write(&b) {
                                Ok(n) => format!("写 {hex}→ 成功 {n} 字节"),
                                Err(e) => format!("写 {hex}→ 失败: {e}"),
                            };
                            eprintln!("[cmd] {msg}");
                            let _ = tx.send(Event::Log(msg));
                        }
                    }
                    // 开关麦：标志一变立刻下发（读超时 200ms，所以延迟 <=200ms）
                    let mic_now = status.mic_on.load(Ordering::Relaxed);
                    if mic_now != mic_was {
                        let cmd: &[u8] = if mic_now { &MIC_ON } else { &MIC_OFF };
                        match dev.write(cmd) {
                            Ok(n) => eprintln!("[mic] {}麦 {cmd:02X?} → 成功 {n} 字节", if mic_now { "开" } else { "关" }),
                            Err(e) => eprintln!("[mic] {}麦 {cmd:02X?} → 失败 {e}", if mic_now { "开" } else { "关" }),
                        }
                        mic_was = mic_now;
                        last_ka = Instant::now();
                    }
                    // keepalive：麦克风开着时每秒重发一次 MIC_ON。
                    // 不补的话说着说着流会断。
                    if mic_now && last_ka.elapsed() >= Duration::from_secs(1) {
                        let _ = dev.write(&MIC_ON);
                        last_ka = Instant::now();
                    }
                    // 自愈：没开麦却还在收音频 → 设备侧还热着（上次强杀、或没收到命令），补关麦。
                    // PTT 遥控器上这是误触发 —— 它本来就是「按住才出流」，没开麦时收到音频
                    // 完全正常，补关麦既没用又每 2 秒来一次。auto_mic_off 是诊断时的开关
                    // （probe-all 会关掉它），以前只存不读，等于没接上。
                    let self_heal = auto_mic_off.load(Ordering::Relaxed)
                        && !cfg.read().settings.mic_model.is_ptt();
                    if self_heal && !mic_now && idle_at.elapsed() >= Duration::from_secs(2) {
                        let now_frames = status.audio_frames.load(Ordering::Relaxed);
                        if now_frames > idle_frames + 5 {
                            let _ = dev.write(&MIC_OFF);
                        }
                        idle_frames = now_frames;
                        idle_at = Instant::now();
                    }
                    // 长按到时：立刻触发长按动作，并标记以抑制本次短按
                    {
                        let thresh = Duration::from_millis(cfg.read().long_press_ms());
                        let mut fire: Vec<Key> = Vec::new();
                        for (k, (t0, fired)) in held.iter_mut() {
                            if !*fired && t0.elapsed() >= thresh {
                                *fired = true;
                                fire.push(*k);
                            }
                        }
                        for k in fire {
                            let r = dispatch(
                                &cfg,
                                &status,
                                &inj,
                                &dictating,
                                &recording,
                                &tx,
                                &voice,
                                &prev_input,
                                k,
                                true,
                                true,
                            );
                            if !r.is_empty() {
                                // 只在动作真的执行了才学 —— 否则会把「没配动作」的键也
                                // 学进屏蔽表，结果系统默认行为被吞、又没有替代动作，纯亏
                                if let Some(l) =
                                    learn_suppress_codes(&cfg, &recent_ev, &learned_codes)
                                {
                                    let _ = tx
                                        .send(Event::Log(format!("已学到要屏蔽的系统键码: {l:?}")));
                                }
                                let _ = tx.send(Event::Key {
                                    key: k,
                                    down: true,
                                    result: r,
                                });
                            }
                        }
                    }

                    let n = match dev.read_timeout(&mut buf, 200) {
                        Ok(n) => n,
                        Err(e) => {
                            status.connected.store(false, Ordering::Relaxed);
                            let _ = tx.send(Event::Disconnected(e.to_string()));
                            break;
                        }
                    };
                    if n == 0 {
                        continue;
                    }
                    let rid = buf[0];
                    let payload = &buf[1..n];
                    *seen_rids.lock().entry(rid).or_insert(0) += 1;
                    let raw_on = raw_all.load(Ordering::Relaxed);
                    if raw_on {
                        let _ = tx.send(Event::Raw {
                            report_id: rid,
                            data: payload.to_vec(),
                        });
                    }

                    match rid {
                        RID_AUDIO => {
                            status.audio_frames.fetch_add(1, Ordering::Relaxed);
                            let sink = voice.lock().clone();
                            let passing = sink.as_ref().map(|s| s.passing()).unwrap_or(false);
                            // 听写不经过虚拟声卡，passing 是假的 ——
                            // 以前这里只在 passing 为真时解码，于是听写永远收不到采样，
                            // 一松手就是「说得太短」。两条路要各自判断。
                            let taking = dictating.lock().is_some();
                            let recing = recording.lock().is_some();
                            if passing || taking || recing {
                                if let Ok(got) = dec.decode(payload, &mut pcm, false) {
                                    if passing {
                                        if let Some(sink) = &sink {
                                            sink.push_pcm(&pcm[..got]);
                                        }
                                    }
                                    if let Some(r) = dictating.lock().as_mut() {
                                        r.push(&pcm[..got]);
                                    }
                                    {
                                        let mut g = recording.lock();
                                        if let Some(r) = g.as_mut() {
                                            r.push(&pcm[..got]);
                                            if r.samples_len() % 16_000 < 320 {
                                                eprintln!("[rec] 已写 {:.1}s", r.seconds());
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        RID_BATTERY => {
                            if let Some(&b) = payload.first() {
                                record_battery(&status, &cfg, b as i32, "上报");
                            }
                        }
                        RID_KEYBOARD | RID_CONSUMER | RID_VENDOR_EF => {
                            // 尽早打时间戳：tap 那边靠这个判断「这一下是遥控器按的」，
                            // 越靠近物理按下越准。放在解析之前。
                            *last_hid_key.lock() = Some(Instant::now());
                            let (now, page): (HashSet<Key>, u16) = match rid {
                                RID_KEYBOARD => (
                                    parse_keyboard(payload).into_iter().collect(),
                                    crate::keys::PAGE_KEYBOARD,
                                ),
                                RID_CONSUMER => (
                                    parse_consumer(payload).into_iter().collect(),
                                    crate::keys::PAGE_CONSUMER,
                                ),
                                // 0xEF：四个 App 快捷键
                                _ => (
                                    parse_vendor(payload).into_iter().collect(),
                                    crate::keys::PAGE_VENDOR,
                                ),
                            };
                            let mut events: Vec<(Key, bool)> = Vec::new();
                            for k in now.iter() {
                                if active.insert(*k) {
                                    events.push((*k, true));
                                }
                            }
                            let released: Vec<Key> = active
                                .iter()
                                .filter(|k| k.page == page && !now.contains(*k))
                                .copied()
                                .collect();
                            for k in released {
                                active.remove(&k);
                                events.push((k, false));
                            }
                            // tap 靠这个标志吞掉 macOS 的自动重复事件
                            hid_key_held.store(!active.is_empty(), Ordering::Relaxed);

                            for (k, down) in events {
                                // 纯观测：诊断长按用，原样上报每个边沿，不影响任何逻辑
                                if trace_keys.load(Ordering::Relaxed) {
                                    let _ = tx.send(Event::KeyEdge { key: k, down });
                                }
                                {
                                    let mut p = pressed.lock();
                                    if down {
                                        p.insert(k);
                                    } else {
                                        p.remove(&k);
                                    }
                                }
                                if learn.load(Ordering::Relaxed) {
                                    if down {
                                        let _ = tx.send(Event::Learned(k));
                                    }
                                    continue;
                                }
                                if down {
                                    // 配了长按才起定时器；没配就等松手直接走短按
                                    let slot = cfg.read().key_slot(k);
                                    let (has_long, at_once) = slot
                                        .map(|s| {
                                            let c = cfg.read();
                                            (
                                                c.profile().has_long(s),
                                                c.profile().long_fires_on_press(s),
                                            )
                                        })
                                        .unwrap_or((false, false));
                                    // 短按是空的 -> 没有要区分的东西，等阈值纯是延迟。
                                    // PTT 遥控器的麦克风键就靠这条：长按=按住说话，
                                    // 按下立刻开闸，开头那截话才不会丢。
                                    if has_long && at_once {
                                        held.insert(k, (Instant::now(), true)); // 标记已触发
                                        let r = dispatch(
                                            &cfg,
                                            &status,
                                            &inj,
                                            &dictating,
                                            &recording,
                                            &tx,
                                            &voice,
                                            &prev_input,
                                            k,
                                            true,
                                            true, // long
                                        );
                                        if !r.is_empty() {
                                            if let Some(l) = learn_suppress_codes(
                                                &cfg,
                                                &recent_ev,
                                                &learned_codes,
                                            ) {
                                                let _ = tx.send(Event::Log(format!(
                                                    "已学到要屏蔽的系统键码: {l:?}"
                                                )));
                                            }
                                            let _ = tx.send(Event::Key {
                                                key: k,
                                                down: true,
                                                result: r,
                                            });
                                        }
                                        continue;
                                    }
                                    if has_long {
                                        held.insert(k, (Instant::now(), false));
                                        continue; // 按下先不动作，等长按到时或松手
                                    }
                                    // 无长按配置：按下即触发短按（响应更快）
                                    let r = dispatch(
                                        &cfg,
                                        &status,
                                        &inj,
                                        &dictating,
                                        &recording,
                                        &tx,
                                        &voice,
                                        &prev_input,
                                        k,
                                        true,
                                        false,
                                    );
                                    if !r.is_empty() {
                                        // 只在动作真的执行了才学 —— 否则会把「没配动作」的键也
                                        // 学进屏蔽表，结果系统默认行为被吞、又没有替代动作，纯亏
                                        if let Some(l) =
                                            learn_suppress_codes(&cfg, &recent_ev, &learned_codes)
                                        {
                                            let _ = tx.send(Event::Log(format!(
                                                "已学到要屏蔽的系统键码: {l:?}"
                                            )));
                                        }
                                        let _ = tx.send(Event::Key {
                                            key: k,
                                            down: true,
                                            result: r,
                                        });
                                    }
                                } else {
                                    match held.remove(&k) {
                                        // 长按已触发 -> 松手只给长按动作发 release（按住说话要停流）
                                        Some((_, true)) => {
                                            let r = dispatch(
                                                &cfg,
                                                &status,
                                                &inj,
                                                &dictating,
                                                &recording,
                                                &tx,
                                                &voice,
                                                &prev_input,
                                                k,
                                                false,
                                                true,
                                            );
                                            if !r.is_empty() {
                                                // 只在动作真的执行了才学 —— 否则会把「没配动作」的键也
                                                // 学进屏蔽表，结果系统默认行为被吞、又没有替代动作，纯亏
                                                if let Some(l) = learn_suppress_codes(
                                                    &cfg,
                                                    &recent_ev,
                                                    &learned_codes,
                                                ) {
                                                    let _ = tx.send(Event::Log(format!(
                                                        "已学到要屏蔽的系统键码: {l:?}"
                                                    )));
                                                }
                                                let _ = tx.send(Event::Key {
                                                    key: k,
                                                    down: false,
                                                    result: r,
                                                });
                                            }
                                        }
                                        // 没到长按阈值就松手 -> 短按
                                        Some((_, false)) => {
                                            let r = dispatch(
                                                &cfg,
                                                &status,
                                                &inj,
                                                &dictating,
                                                &recording,
                                                &tx,
                                                &voice,
                                                &prev_input,
                                                k,
                                                true,
                                                false,
                                            );
                                            if !r.is_empty() {
                                                // 只在动作真的执行了才学 —— 否则会把「没配动作」的键也
                                                // 学进屏蔽表，结果系统默认行为被吞、又没有替代动作，纯亏
                                                if let Some(l) = learn_suppress_codes(
                                                    &cfg,
                                                    &recent_ev,
                                                    &learned_codes,
                                                ) {
                                                    let _ = tx.send(Event::Log(format!(
                                                        "已学到要屏蔽的系统键码: {l:?}"
                                                    )));
                                                }
                                                let _ = tx.send(Event::Key {
                                                    key: k,
                                                    down: true,
                                                    result: r,
                                                });
                                            }
                                        }
                                        // 没起定时器（无长按配置）-> 按下时已触发，松手补 release
                                        None => {
                                            let r = dispatch(
                                                &cfg,
                                                &status,
                                                &inj,
                                                &dictating,
                                                &recording,
                                                &tx,
                                                &voice,
                                                &prev_input,
                                                k,
                                                false,
                                                false,
                                            );
                                            if !r.is_empty() {
                                                // 只在动作真的执行了才学 —— 否则会把「没配动作」的键也
                                                // 学进屏蔽表，结果系统默认行为被吞、又没有替代动作，纯亏
                                                if let Some(l) = learn_suppress_codes(
                                                    &cfg,
                                                    &recent_ev,
                                                    &learned_codes,
                                                ) {
                                                    let _ = tx.send(Event::Log(format!(
                                                        "已学到要屏蔽的系统键码: {l:?}"
                                                    )));
                                                }
                                                let _ = tx.send(Event::Key {
                                                    key: k,
                                                    down: false,
                                                    result: r,
                                                });
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        RID_VENDOR_F1 => {
                            if !raw_on {
                                let _ = tx.send(Event::Raw {
                                    report_id: rid,
                                    data: payload.to_vec(),
                                });
                            }
                        }
                        _ => {
                            if !raw_on {
                                let _ = tx.send(Event::Raw {
                                    report_id: rid,
                                    data: payload.to_vec(),
                                });
                            }
                        }
                    }
                }
                let _ = dev.write(&MIC_OFF);
                pressed.lock().clear();
            })?;
        Ok(())
    }

    /// 开/关遥控器麦克风。设备句柄归读线程所有，这里翻标志，
    /// 读线程在下一次循环（<=200ms）立刻下发命令。
    /// 注意麦克风是「热」的：开了就一直吐流，与按键无关。
    pub fn set_mic(&self, on: bool) {
        self.status.mic_on.store(on, Ordering::Relaxed);
    }

    /// 从界面主动触发某个位置的动作（点击软遥控器 / 「测试」按钮）。
    /// `long=false` 走短按配置，`true` 走长按配置。
    /// 按住说话这类需要按下/松开配对的动作，在这里等价于「切换」。
    pub fn trigger_slot(&self, slot: crate::layout::Slot, long: bool) -> String {
        let (disabled, act, key) = {
            let c = self.cfg.read();
            (
                c.profile().is_disabled(slot),
                c.profile().action(slot, long),
                c.slot_key(slot),
            )
        };
        if disabled {
            return "已禁用".into();
        }
        if let Some(act) = act {
            // 「测试一次」只发按下、没有松开。按住类动作因此会一直挂着 ——
            // 录音会开一个永不结束的会话，按住说话会把麦克风永久开着（费电）。
            // 给它们补一次 2 秒后的松开，等于「试 2 秒」。
            const TRY: Duration = Duration::from_secs(2);
            if act.kind == ActionType::Record {
                let r = run_record(&self.status, &self.recording, &self.tx, true);
                let (st, rec, tx) =
                    (self.status.clone(), self.recording.clone(), self.tx.clone());
                std::thread::spawn(move || {
                    std::thread::sleep(TRY);
                    let _ = run_record(&st, &rec, &tx, false);
                });
                return if r.is_empty() { "试录 2 秒".into() } else { format!("{r} · 2 秒") };
            }
            if act.kind == ActionType::VoicePtt {
                let r = self.run_action(&act, true);
                if let Some(sink) = self.voice.lock().clone() {
                    let (cfg, st, prev) =
                        (self.cfg.clone(), self.status.clone(), self.prev_input.clone());
                    std::thread::spawn(move || {
                        std::thread::sleep(TRY);
                        gate_voice(&cfg, &st, &sink, &prev, false, false);
                    });
                }
                return format!("{r} · 2 秒");
            }
            return self.run_action(&act, true);
        }
        // 没配就走默认直通：方向键 / OK / 返回 / 音量 / 静音 / 播放 / 快进快退
        // 按了就该有反应，不该要求先配一遍
        let Some(k) = key else {
            return "未设置".into();
        };
        match passthrough(k) {
            Some(name) => match self.inj.key_stroke(name, &[]) {
                Ok(_) => format!("默认 · {name}"),
                Err(e) => format!("失败: {e}"),
            },
            None => "未设置".into(),
        }
    }

    /// 执行一个动作。`down` 只对按住说话有意义（true=开始送流）。
    pub fn run_action(&self, act: &Action, down: bool) -> String {
        match act.kind {
            ActionType::None => "未设置".into(),
            ActionType::VoicePtt => {
                let Some(sink) = self.voice.lock().clone() else {
                    return "语音未启动".into();
                };
                gate_voice(
                    &self.cfg,
                    &self.status,
                    &sink,
                    &self.prev_input,
                    down,
                    false,
                );
                if down {
                    "开始送流".into()
                } else {
                    "停止送流".into()
                }
            }
            ActionType::VoiceToggle => {
                if !down {
                    return String::new(); // toggle 只在按下时翻转
                }
                let Some(sink) = self.voice.lock().clone() else {
                    return "语音未启动".into();
                };
                let on = !sink.passing();
                gate_voice(&self.cfg, &self.status, &sink, &self.prev_input, on, false);
                if on {
                    "开始送流".into()
                } else {
                    "停止送流".into()
                }
            }
            // 听写要吃松开事件（松手才去识别），必须在 `!down` 短路之前。
            // 长按 = 按住说话；短按 = 点一下开始、再点一下结束。
            ActionType::Record => {
                return run_record(&self.status, &self.recording, &self.tx, down);
            }
            ActionType::IrBlast => return ir_blast(&self.tx, act),
            ActionType::VoiceDictate => {
                let long = act.arg == "hold";
                if !long && !down {
                    return String::new();
                }
                let on = if long {
                    down
                } else {
                    self.dictating.lock().is_none()
                };
                gate_dictation(
                    &self.cfg,
                    &self.status,
                    &self.inj,
                    &self.dictating,
                    &self.tx,
                    on,
                )
            }
            // 「按住」模式要分 down/up 两次调用，所以它必须在 `!down` 短路之前
            ActionType::VoiceHotkey => {
                let hold = act.arg == "hold";
                // 这个动作的完整语义是「喂第三方工具」：发快捷键 + 把遥控器音频
                // 送进虚拟声卡 + 把系统默认输入切到虚拟声卡。
                // 只发快捷键的话工具虽然起来了，但听的还是你原来的麦克风。
                if let Some(sink) = self.voice.lock().clone() {
                    let on = if hold { down } else { !sink.passing() };
                    if hold || down {
                        gate_voice(&self.cfg, &self.status, &sink, &self.prev_input, on, true);
                    }
                }
                let r = if hold {
                    if down {
                        mark_hold(&act.key);
                        self.inj.key_down(&act.key, &act.mods)
                    } else {
                        hold_long_enough(&act.key);
                        self.inj.key_up(&act.key, &act.mods)
                    }
                } else if down {
                    if act.arg == "double" {
                        double_stroke(&self.inj, &act.key, &act.mods)
                    } else {
                        self.inj.key_stroke(&act.key, &act.mods)
                    }
                } else if act.arg == "double" {
                    self.inj.key_stroke(&act.key, &act.mods)
                } else {
                    return String::new();
                };
                match r {
                    Ok(_) => {
                        if hold && !down {
                            "松开".into()
                        } else {
                            act.describe()
                        }
                    }
                    Err(e) => format!("失败: {e}"),
                }
            }
            _ if !down => String::new(),
            ActionType::Key => match self.inj.key_stroke(&act.key, &act.mods) {
                Ok(_) => act.describe(),
                Err(e) => format!("失败: {e}"),
            },
            ActionType::Text => match self.inj.type_text(&act.arg) {
                Ok(_) => act.describe(),
                Err(e) => format!("失败: {e}"),
            },
            ActionType::OpenApp => {
                spawn_open_app(&act.arg);
                act.describe()
            }
            ActionType::AppleScript => {
                spawn_applescript(&act.arg);
                act.describe()
            }
            ActionType::Http => {
                spawn_http(act, self.tx.clone());
                return act.describe();
            }
            ActionType::Shell => {
                spawn_shell(&act.arg);
                act.describe()
            }
        }
    }

    /// 启动时补救：上次进程被硬杀、系统默认输入还留在虚拟声卡上，
    /// 这里把它切回去。
    pub fn recover_input(&self) {
        let (id, want) = {
            let c = self.cfg.read();
            (c.settings.prev_input_id, c.voice.device.to_lowercase())
        };
        let Some(id) = id else { return };
        let cfg = self.cfg.clone();
        std::thread::spawn(move || {
            // 只有当前确实停在虚拟声卡上才动，别把用户自己选的设备改掉
            let stuck = crate::audio::default_input()
                .map(|d| d.name.to_lowercase().contains(&want))
                .unwrap_or(false);
            if stuck {
                let _ = crate::audio::set_default_input(id);
            }
            let mut g = cfg.write();
            g.settings.prev_input_id = None;
            let _ = g.save();
        });
    }

    /// 停下来时把系统输入设备还原 —— 别让它停在虚拟声卡上，
    /// 否则会议、系统听写全都听不到人声
    pub fn restore_input(&self) {
        if let Some(id) = self.prev_input.lock().take() {
            std::thread::spawn(move || {
                let _ = crate::audio::set_default_input(id);
            });
        }
    }

    /// 递一条 OUTPUT 报文给设备（第一个字节是 report id）。
    /// ⚠️ 只用来发**已知语义**的命令 —— 对不明设备盲扫厂商 opcode
    /// 可能干出不可逆的事（GATT 那边就有 WIPE）。
    pub fn send_report(&self, bytes: Vec<u8>) {
        self.pending_writes.lock().push(bytes);
    }

    pub fn stop(&self) {
        // 映射是系统状态，退出必须清 —— 留着遥控器那颗键会一直是修饰键
        crate::hidremap::clear();
        self.restore_input();
        self.stop.store(true, Ordering::Relaxed);
        self.status.mic_on.store(false, Ordering::Relaxed);
        self.stop_voice();
    }
}

fn dispatch(
    cfg: &Arc<RwLock<Config>>,
    status: &Arc<Status>,
    inj: &Arc<dyn Injector>,
    dictating: &Arc<Mutex<Option<crate::stt::Recorder>>>,
    recording: &Arc<Mutex<Option<crate::recorder::Rec>>>,
    tx: &Sender<Event>,
    voice: &Arc<Mutex<Option<Arc<VoiceSink>>>>,
    prev: &Arc<Mutex<Option<u32>>>,
    k: Key,
    down: bool,
    long: bool,
) -> String {
    // 被禁用的键彻底不响应（也不走直通）
    if let Some(s) = cfg.read().key_slot(k) {
        if cfg.read().profile().is_disabled(s) {
            return String::new();
        }
    }
    // usage -> 图上位置 -> 当前 profile 里该触发方式配的动作
    let found = cfg.read().action_for(k, long);
    let Some((slot, act)) = found else {
        // 该位置没配动作（或 usage 还没绑到任何位置）-> 自动直通
        return match passthrough(k) {
            Some(name) if down => match inj.key_stroke(name, &[]) {
                Ok(_) => format!("直通 -> {name}"),
                Err(e) => format!("直通失败: {e}"),
            },
            Some(_) => String::new(),
            None if down => "未配置".into(),
            None => String::new(),
        };
    };

    // 统一记一次使用统计：只在按下时记，每个动作一次，不管后面走哪条分支。
    // VoiceToggle/Dictate 的「再点一下停止」也是一次 down，会重复记 —— 可接受
    //（统计的是「触发次数」，开一次+关一次算两次触发，语义上也说得过去）。
    if down && act.kind != ActionType::None {
        let is_voice = matches!(
            act.kind,
            ActionType::VoicePtt
                | ActionType::VoiceToggle
                | ActionType::VoiceDictate
                | ActionType::VoiceHotkey
        );
        let slot_id = slot.id();
        let action_dbg = format!("{:?}", act.kind);
        let mut c = cfg.write();
        c.stats.record(slot_id, &action_dbg, is_voice, 0.0);
        let _ = c.save();
    }

    // 按住说话要处理按下和松开两个方向
    if act.kind == ActionType::VoicePtt {
        let Some(sink) = voice.lock().clone() else {
            return "按住说话：语音未启动".into();
        };
        gate_voice(cfg, status, &sink, prev, down, false);
        return if down {
            "开始送流".into()
        } else {
            "停止送流".into()
        };
    }
    if act.kind == ActionType::VoiceToggle {
        if !down {
            return String::new();
        }
        let Some(sink) = voice.lock().clone() else {
            return "语音未启动".into();
        };
        let on = !sink.passing();
        gate_voice(cfg, status, &sink, prev, on, false);
        return if on {
            "开始送流".into()
        } else {
            "停止送流".into()
        };
    }

    // 听写同样要吃松开事件。长按 = 按住说话；短按 = 点一下开始、再点一下结束。
    if act.kind == ActionType::VoiceDictate {
        let hold = act.arg == "hold";
        if !hold && !down {
            return String::new();
        }
        let on = if hold {
            down
        } else {
            dictating.lock().is_none()
        };
        return gate_dictation(cfg, status, inj, dictating, tx, on);
    }
    // 外部语音 app 的「按住」模式同样要吃松开事件，必须在 !down 短路之前
    if act.kind == ActionType::VoiceHotkey {
        // 纯修饰键走的是 HID 设备层映射（按下时系统已经收到真硬件事件了），
        // 这里再合成一次就是两下。只把音频那半边做掉。
        let via_hw = act.mods.is_empty()
            && crate::hidremap::usage_of(&act.key).is_some()
            && cfg.read().mic_remap_key().as_deref() == Some(act.key.as_str());
        let hold = act.arg == "hold";
        // 同上：发快捷键的同时把音频送进虚拟声卡、把系统默认输入切过去
        if let Some(sink) = voice.lock().clone() {
            let on = if hold { down } else { !sink.passing() };
            if hold || down {
                gate_voice(cfg, status, &sink, prev, on, true);
            }
        }
        if via_hw {
            // 音频照旧送进虚拟声卡，按键交给设备层映射
            return if hold && !down {
                "松开".into()
            } else {
                format!("第三方语音输入 · {}（硬件层）", act.key)
            };
        }
        let r = if hold {
            if down {
                mark_hold(&act.key);
                inj.key_down(&act.key, &act.mods)
            } else {
                // 松开前保证至少按住了 MIN_HOLD —— 豆包按「按住时长」判断是
                // 「按住说话」还是「单击」，太短会被当成单击，于是它开了个
                // 长录音会话却不收音。参考实现也是这么兜的。
                hold_long_enough(&act.key);
                inj.key_up(&act.key, &act.mods)
            }
        } else if down {
            // double：双击开、单击关 —— 豆包的默认设置就是这种
            if act.arg == "double" {
                double_stroke(inj, &act.key, &act.mods)
            } else {
                inj.key_stroke(&act.key, &act.mods)
            }
        } else if act.arg == "double" {
            // 双击模式的收尾是单击一次
            inj.key_stroke(&act.key, &act.mods)
        } else {
            return String::new();
        };
        return match r {
            Ok(_) if hold && !down => "松开".into(),
            Ok(_) => act.describe(),
            Err(e) => format!("失败: {e}"),
        };
    }

    // 录音是按住语义：按下开始、**松手保存**。所以它必须在 `!down` 短路之前 ——
    // 以前排在后面，松开事件被吃掉，录音根本停不下来（再按一次会撞上
    // 「已经在录就别重开」直接返回，计时器一直涨）。
    if act.kind == ActionType::Record {
        return run_record(status, recording, tx, down);
    }

    if !down {
        return String::new(); // 其余动作只在按下时触发
    }

    match act.kind {
        ActionType::None => "未设置".into(),
        // 上面已经提前返回了，这条只是让 match 穷尽
        ActionType::Record => String::new(),
        ActionType::IrBlast => ir_blast(tx, &act),
        ActionType::Key => match inj.key_stroke(&act.key, &act.mods) {
            Ok(_) => act.describe(),
            Err(e) => format!("失败: {e}"),
        },
        ActionType::Text => match inj.type_text(&act.arg) {
            Ok(_) => act.describe(),
            Err(e) => format!("失败: {e}"),
        },
        ActionType::OpenApp => {
            spawn_open_app(&act.arg);
            act.describe()
        }
        ActionType::AppleScript => {
            spawn_applescript(&act.arg);
            act.describe()
        }
        ActionType::Shell => {
            spawn_shell(&act.arg);
            act.describe()
        }
        ActionType::Http => {
            spawn_http(&act, tx.clone());
            act.describe()
        }
        ActionType::VoicePtt
        | ActionType::VoiceToggle
        | ActionType::VoiceHotkey
        | ActionType::VoiceDictate => unreachable!(),
    }
}

/// HID 线程处理了一个键 —— 回看最近 200ms 里系统那边产生的非字符事件，
/// 把它们的键码学下来存进配置，之后 tap 就能无条件吞掉。
///
/// 为什么要「回看」而不是「往后开窗口」：系统那条 HID→事件通路和我们的
/// hidapi 读取是并行的，系统完全可能更快 —— 等我们处理完再开窗口已经晚了。
fn learn_suppress_codes(
    cfg: &Arc<RwLock<Config>>,
    recent: &Arc<Mutex<std::collections::VecDeque<(Instant, i64, i64)>>>,
    learned: &Arc<Mutex<Vec<i64>>>,
) -> Option<Vec<i64>> {
    if !cfg.read().settings.suppress_os_keys {
        return None;
    }
    let now = Instant::now();
    let hits: Vec<(i64, i64)> = recent
        .lock()
        .iter()
        .filter(|(t, _, _)| now.duration_since(*t) < Duration::from_millis(200))
        .map(|(_, c, kb)| (*c, *kb))
        .collect();
    // 内置那些（麦克风键 0xb1）开箱就屏蔽，不用学、也别往配置里写 ——
    // 写进去只会让人以为「这功能得自己教一遍」。学习只兜内置没覆盖到的键。
    let mut fresh: Vec<i64> = Vec::new();
    {
        let mut l = learned.lock();
        for (c, _) in &hits {
            if !l.contains(c) {
                l.push(*c);
                if !crate::tap::BUILTIN_SUPPRESS.contains(c) {
                    fresh.push(*c);
                }
            }
        }
    }
    if fresh.is_empty() {
        return None;
    }
    let keep: Vec<i64> = learned
        .lock()
        .iter()
        .copied()
        .filter(|c| !crate::tap::BUILTIN_SUPPRESS.contains(c))
        .collect();
    let mut g = cfg.write();
    g.settings.suppress_codes = keep;
    g.settings.suppress_kb_types.clear(); // 这条路走不通，顺手清掉历史噪音
    let _ = g.save();
    Some(fresh)
}

/// 听写开关。开 = 开麦并开始攒 PCM；关 = 写 WAV 去识别、把文字打进当前焦点。
///
/// **不碰系统输入设备、也不需要虚拟声卡** —— 直接吃解码后的 PCM，
/// 这是自带识别相对「喂第三方工具」最大的好处。
/// 识别是阻塞的，整段丢后台线程，结果走 Event::Log 报出来。
fn gate_dictation(
    cfg: &Arc<RwLock<Config>>,
    status: &Arc<Status>,
    inj: &Arc<dyn Injector>,
    dictating: &Arc<Mutex<Option<crate::stt::Recorder>>>,
    tx: &Sender<Event>,
    on: bool,
) -> String {
    if on {
        if !crate::stt::authorized() {
            return format!("没有语音识别权限（{}）", crate::stt::auth_status());
        }
        // 记住「按下麦克风那一刻谁在前台」。识别是异步的，中间悬浮窗开关、
        // 系统麦克风键的默认行为都可能把前台抢走，到打字时就找不到输入框了。
        let mut rec = crate::stt::Recorder::new();
        rec.front = crate::frontapp::front();
        if let Some(f) = &rec.front {
            eprintln!("[stt] 按下时前台 = {} (pid {})", f.name, f.pid);
        }
        *dictating.lock() = Some(rec);
        status.mic_on.store(true, Ordering::Relaxed);
        return "开始听写".into();
    }
    let Some(rec) = dictating.lock().take() else {
        return String::new();
    };
    status.mic_on.store(false, Ordering::Relaxed);
    if rec.seconds() < 0.25 {
        return "说得太短".into();
    }
    let (locale, auto_enter) = {
        let c = cfg.read();
        (c.settings.stt_locale.clone(), c.settings.stt_auto_enter)
    };
    let inj = inj.clone();
    let tx = tx.clone();
    let secs = rec.seconds();
    let front = rec.front.clone();
    std::thread::spawn(move || {
        let path = match rec.write_wav(crate::voice::OPUS_RATE) {
            Ok(p) => p,
            Err(e) => {
                let _ = tx.send(Event::Log(format!("听写失败：写临时文件出错 {e}")));
                return;
            }
        };
        eprintln!("[stt] wav={} locale={locale} {:.1}s", path.display(), secs);
        let t0 = std::time::Instant::now();
        let out = crate::stt::transcribe_file(&path, &locale, true);
        eprintln!(
            "[stt] 耗时 {:.1}s 结果 {:?}",
            t0.elapsed().as_secs_f32(),
            out
        );
        match out {
            Ok(text) if text.trim().is_empty() => {
                let _ = tx.send(Event::Log("没识别出内容".into()));
            }
            Ok(text) => {
                let _ = tx.send(Event::Log(format!("听写（{secs:.1}s）：{text}")));
                // 字要落到「按下麦克风时」那个 app 里。前台被抢走过就切回去，
                // 不然 type_text 会成功返回，字却进了别的窗口（或者哪儿都没进）。
                let now = crate::frontapp::front();
                eprintln!(
                    "[stt] 打字前前台 = {:?}，目标 = {:?}",
                    now.as_ref().map(|f| f.name.clone()),
                    front.as_ref().map(|f| f.name.clone())
                );
                if let Some(want) = &front {
                    if want.pid == crate::frontapp::self_pid() {
                        let _ = tx.send(Event::Log(format!(
                            "识别到「{text}」，但按下时前台是 FireVibe 自己，没地方打字"
                        )));
                        let _ = std::fs::remove_file(&path);
                        return;
                    }
                    if now.as_ref().map(|f| f.pid) != Some(want.pid) {
                        let ok = crate::frontapp::activate(want.pid);
                        std::thread::sleep(std::time::Duration::from_millis(180));
                        eprintln!(
                            "[stt] 切回 {} → {ok}，现在前台 = {:?}",
                            want.name,
                            crate::frontapp::front().map(|f| f.name)
                        );
                    }
                }
                let r = inj.type_text(&text);
                eprintln!("[stt] 打字 {:?}", r);
                if let Err(e) = r {
                    let _ = tx.send(Event::Log(format!("打字失败: {e}")));
                } else if auto_enter {
                    let _ = inj.key_stroke("return", &[]);
                }
            }
            Err(e) => {
                let _ = tx.send(Event::Log(format!("听写失败: {e}")));
            }
        }
        let _ = std::fs::remove_file(&path);
    });
    format!("识别中（{secs:.1}s）…")
}

/// 说话开关。顺带按设置自动切换系统默认输入设备。
///
/// 为什么要切：靠输入法/外部 app 做识别时，它们听的是**系统默认输入**。
/// 实测切换本身 3~13ms，所以按下就切、松手切回是可行的。
/// 切换必须在后台线程做 —— CoreAudio 调用会跑 run loop，
/// 在 gpui 的 update 里同步调会炸（见 audio.rs 顶部注释）。
fn gate_voice(
    cfg: &Arc<RwLock<Config>>,
    status: &Arc<Status>,
    sink: &Arc<VoiceSink>,
    prev: &Arc<Mutex<Option<u32>>>,
    on: bool,
    // true = 在当前线程把设备切完再返回。喂第三方工具必须这样：
    // 它收到快捷键就开麦，切换晚一步它就绑错设备。
    wait: bool,
) {
    sink.set_passing(on);
    // 开麦是热的：发一次 MIC_ON 就一直吐流。所以跟着说话状态开关，
    // 别一直开着 —— 蓝灯一直闪、还费遥控器的电。
    status.mic_on.store(on, Ordering::Relaxed);
    let (auto, want) = {
        let c = cfg.read();
        (c.settings.auto_switch_input, c.voice.device.to_lowercase())
    };
    if !auto {
        return;
    }
    let prev = prev.clone();
    let cfg = cfg.clone();
    let work = move || {
        if on {
            let cur = crate::audio::default_input();
            let already = cur
                .as_ref()
                .map(|d| d.name.to_lowercase().contains(&want))
                .unwrap_or(false);
            if already {
                return; // 已经是虚拟声卡，别多折腾
            }
            let Some(target) = crate::audio::input_devices()
                .into_iter()
                .find(|d| d.name.to_lowercase().contains(&want))
            else {
                return;
            };
            let prev_id = cur.map(|d| d.id);
            *prev.lock() = prev_id;
            // 落盘：进程被硬杀时下次启动才能还原（见 recover_input）
            {
                let mut g = cfg.write();
                if g.settings.prev_input_id != prev_id {
                    g.settings.prev_input_id = prev_id;
                    let _ = g.save();
                }
            }
            let ok =
                crate::audio::set_default_input_and_wait(target.id, Duration::from_millis(250));
            if !ok {
                eprintln!("[voice] 切到 {} 没在 250ms 内生效", target.name);
            }
        } else {
            // 晚 400ms 再切回去：消费方可能还在读缓冲，立刻换设备会把尾音截掉
            std::thread::sleep(Duration::from_millis(400));
            if let Some(id) = prev.lock().take() {
                let _ = crate::audio::set_default_input(id);
            }
            let mut g = cfg.write();
            if g.settings.prev_input_id.is_some() {
                g.settings.prev_input_id = None;
                let _ = g.save();
            }
        }
    };
    // 开麦要同步等切换落地（第三方工具在抢设备）；关麦本来就要延迟 400ms，丢后台
    if wait && on {
        work();
    } else {
        std::thread::spawn(work);
    }
}

/// 打开应用。参数可以是 bundle id（含点号）、应用名、或路径。
fn spawn_open_app(target: &str) {
    let t = target.to_string();
    if t.is_empty() {
        return;
    }
    std::thread::spawn(move || {
        #[cfg(target_os = "macos")]
        {
            // 带点号且不像路径 -> 当 bundle id 用 -b，更稳（不受改名影响）
            let looks_bundle = t.contains('.') && !t.contains('/') && !t.ends_with(".app");
            let r = if looks_bundle {
                std::process::Command::new("open").args(["-b", &t]).spawn()
            } else {
                std::process::Command::new("open").args(["-a", &t]).spawn()
            };
            // bundle id 打不开时退回按名字试一次
            if r.is_err() && looks_bundle {
                let _ = std::process::Command::new("open").args(["-a", &t]).spawn();
            }
        }
        #[cfg(target_os = "linux")]
        {
            let _ = std::process::Command::new("gtk-launch").arg(&t).spawn();
        }
        #[cfg(target_os = "windows")]
        {
            let _ = std::process::Command::new("cmd")
                .args(["/C", "start", "", &t])
                .spawn();
        }
    });
}

/// 执行 AppleScript（仅 macOS；其他平台忽略）
fn spawn_applescript(script: &str) {
    let s = script.to_string();
    if s.is_empty() {
        return;
    }
    std::thread::spawn(move || {
        #[cfg(target_os = "macos")]
        {
            let _ = std::process::Command::new("osascript")
                .args(["-e", &s])
                .spawn();
        }
        #[cfg(not(target_os = "macos"))]
        let _ = &s;
    });
}

fn spawn_shell(cmd: &str) {
    let c = cmd.to_string();
    if c.is_empty() {
        return;
    }
    std::thread::spawn(move || {
        #[cfg(windows)]
        let _ = std::process::Command::new("cmd").args(["/C", &c]).spawn();
        #[cfg(not(windows))]
        let _ = std::process::Command::new("/bin/sh")
            .args(["-c", &c])
            .spawn();
    });
}

/// 发一个 HTTP 请求。用 `curl` 直接传**参数向量**（不过 shell），所以 URL、
/// 请求体里的引号/空格/换行都不会被拆断（用户拿 shell+curl 就栽在换行上）。
/// `--retry` / `--max-time` 是 curl 原生的。结果（HTTP 状态码或错误）回报到 Event::Log。
///
/// macOS 自带 curl 的异步解析器在部分仅发布 IPv4 的 mDNS 设备上会卡在双栈查询：
/// 系统解析器和 ping 都能解析 `.local`，curl 默认模式却一直等到超时。局域网里的这类
/// 地址强制走 IPv4；普通域名和显式 IPv6 地址仍保留 curl 的默认行为。
fn spawn_http(act: &Action, tx: Sender<Event>) {
    let url = act.arg.trim().to_string();
    if url.is_empty() {
        let _ = tx.send(Event::Log("HTTP 动作没填 URL".into()));
        return;
    }
    let method = if act.method.is_empty() {
        "GET".to_string()
    } else {
        act.method.to_uppercase()
    };
    let timeout_ms = if act.timeout_ms == 0 { 2000 } else { act.timeout_ms };
    let retries = act.retries;
    let body = act.body.clone();
    std::thread::spawn(move || {
        let mut cmd = std::process::Command::new("/usr/bin/curl");
        cmd.arg("-sS") // 安静，但保留错误信息
            .arg("-o")
            .arg("/dev/null")
            .arg("-w")
            .arg("%{http_code}")
            .arg("-X")
            .arg(&method)
            .arg("--max-time")
            .arg(format!("{:.2}", timeout_ms as f64 / 1000.0))
            .arg("--retry")
            .arg(retries.to_string());
        if is_mdns_url(&url) {
            cmd.arg("--ipv4");
        }
        if method == "POST" && !body.is_empty() {
            cmd.arg("-d").arg(&body);
        }
        cmd.arg(&url);
        match cmd.output() {
            Ok(o) if o.status.success() => {
                let code = String::from_utf8_lossy(&o.stdout);
                let _ = tx.send(Event::Log(format!("HTTP {method} → {}", code.trim())));
            }
            Ok(o) => {
                let err = String::from_utf8_lossy(&o.stderr);
                let _ = tx.send(Event::Log(format!("HTTP 请求失败：{}", err.trim())));
            }
            Err(e) => {
                let _ = tx.send(Event::Log(format!("HTTP 起不来（curl 缺失？）：{e}")));
            }
        }
    });
}

/// URL 的主机名是不是 Bonjour/mDNS 的 `.local` 名称。
///
/// 只检查 authority 里的 hostname，避免把路径、查询参数或 userinfo 中碰巧出现的
/// `.local` 当成主机名；末尾带 DNS 根点的 `device.local.` 也接受。
fn is_mdns_url(url: &str) -> bool {
    let Some((scheme, rest)) = url.trim().split_once("://") else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return false;
    }
    let authority = rest
        .split(&['/', '?', '#'][..])
        .next()
        .unwrap_or_default();
    let host_port = authority.rsplit_once('@').map(|(_, h)| h).unwrap_or(authority);
    // 方括号表示显式 IPv6 literal，不是 mDNS hostname。
    if host_port.starts_with('[') {
        return false;
    }
    let host = host_port
        .split_once(':')
        .map(|(h, _)| h)
        .unwrap_or(host_port)
        .trim_end_matches('.');
    !host.is_empty() && host.to_ascii_lowercase().ends_with(".local")
}

#[cfg(test)]
mod http_tests {
    use super::is_mdns_url;

    #[test]
    fn detects_mdns_http_hosts() {
        assert!(is_mdns_url("http://ble-remote.local/api/status"));
        assert!(is_mdns_url("HTTPS://BLE-REMOTE.LOCAL:8443/path"));
        assert!(is_mdns_url("http://user:pass@device.local./path"));
    }

    #[test]
    fn leaves_non_mdns_and_ipv6_urls_alone() {
        assert!(!is_mdns_url("http://example.com/local"));
        assert!(!is_mdns_url("http://device.local.example/path"));
        assert!(!is_mdns_url("http://local/path"));
        assert!(!is_mdns_url("http://[fe80::1]/path"));
        assert!(!is_mdns_url("ftp://device.local/file"));
        assert!(!is_mdns_url("not a url"));
    }
}

/// 供 UI 用：预设的 AppleScript 例子
pub fn applescript_presets() -> &'static [(&'static str, &'static str)] {
    &[
        ("锁屏", "tell application \"System Events\" to keystroke \"q\" using {control down, command down}"),
        ("睡眠", "tell application \"System Events\" to sleep"),
        ("显示桌面", "tell application \"System Events\" to key code 103"),
        ("静音切换", "set volume output muted (not (output muted of (get volume settings)))"),
        ("Music 播放/暂停", "tell application \"Music\" to playpause"),
        ("复制 Safari URL", "tell application \"Safari\" to set the clipboard to URL of current tab of window 1"),
        ("新建 Chrome 窗口", "tell application \"Google Chrome\" to make new window"),
        ("截图到剪贴板", "do shell script \"screencapture -c -i\""),
    ]
}

/// 供 UI 用：常见应用预设
pub fn app_presets() -> &'static [(&'static str, &'static str)] {
    &[
        ("Siri", "com.apple.siri.launcher"),
        ("Claude", "com.anthropic.claudefordesktop"),
        ("ChatGPT", "com.openai.codex"),
        ("Chrome", "com.google.Chrome"),
        ("Safari", "com.apple.Safari"),
        ("终端", "com.apple.Terminal"),
        ("访达", "com.apple.finder"),
        ("系统设置", "com.apple.systempreferences"),
        ("firevibe", "com.tankxu.firevibe"),
    ]
}

/// 让 UI 能直接构造动作
pub fn make_action(kind: ActionType, key: &str, mods: Vec<String>, arg: &str) -> Action {
    Action {
        kind,
        key: key.into(),
        mods,
        arg: arg.into(),
        ..Default::default()
    }
}

/// 「按住说话」型的第三方工具靠按住时长判断，太短会被当成单击。
/// 记下每个键的按下时刻，松开前补足最短时长。
const MIN_HOLD: Duration = Duration::from_millis(1000);

fn hold_marks() -> &'static Mutex<HashMap<String, Instant>> {
    static M: std::sync::OnceLock<Mutex<HashMap<String, Instant>>> = std::sync::OnceLock::new();
    M.get_or_init(|| Mutex::new(HashMap::new()))
}

fn mark_hold(key: &str) {
    hold_marks().lock().insert(key.to_string(), Instant::now());
}

fn hold_long_enough(key: &str) {
    if let Some(t0) = hold_marks().lock().remove(key) {
        let held = t0.elapsed();
        if held < MIN_HOLD {
            std::thread::sleep(MIN_HOLD - held);
        }
    }
}

/// 双击：豆包默认「双击触发键开始长录音、单击结束」
fn double_stroke(inj: &Arc<dyn Injector>, key: &str, mods: &[String]) -> Result<()> {
    inj.key_stroke(key, mods)?;
    std::thread::sleep(Duration::from_millis(80));
    inj.key_stroke(key, mods)
}

/// 记下电量。变了才落盘 —— 下次启动界面上立刻有值，不用干等遥控器上报。
fn record_battery(status: &Arc<Status>, cfg: &Arc<RwLock<Config>>, b: i32, how: &str) {
    if !(1..=100).contains(&b) {
        return; // 0 或越界当无效，别把界面刷成 0%
    }
    let was = status.battery.swap(b, Ordering::Relaxed);
    if was != b {
        eprintln!("[batt] {how} {b}%");
        let mut g = cfg.write();
        if g.settings.last_battery != Some(b) {
            g.settings.last_battery = Some(b);
            let _ = g.save();
        }
    }
}

// ⚠️ 电量**没法主动读**。三条路都试过了：
//   1. GetReport(Input, 0x03) → IOHIDDeviceGetReport 报
//      0xE00002F0「data was not found」。这台设备的 BLE HOGP 通路不给读，
//      只肯自己推。
//   2. IORegistry 的 BatteryPercent → 只有 Apple 自家设备才发布，
//      遥控器的 IOHIDDevice 节点上没有任何 battery/percent 属性。
//   3. 剩下唯一可能是 BLE GATT 标准电池服务（0x180F / 特征 0x2A19）。
//      笔记里说 macOS 对 app 隐藏 HID 服务 0x1812，但电池服务不是 HID 服务、
//      通常可见 —— 没验，要写 CoreBluetooth。
// 所以现在只被动收 0x03 上报，并把最后一次值落盘（last_battery），
// 这样重启后界面上立刻有值，不用干等。

/// 录音：**按住录、松手存**。
///
/// 为什么不是「按一下开始、再按一下停止」：遥控器只在**实体麦克风键按住**期间
/// 才吐音频流 —— 软件开麦无效（四种有出处的写法都试过，写入成功但音频帧为 0）。
/// 所以录音只能跟着按住的那段时间走，配在麦克风键上才有意义。
///
/// 录的是遥控器麦克风解码后的 PCM（和听写共用同一路），不碰系统输入设备。
/// irblast 小进程的位置。和 battprobe 一样在可执行文件旁边；
/// `FIREVIBE_IRBLAST` 可以指到别处，方便改了 helper 不用整包重签。
fn ir_blaster_path() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("FIREVIBE_IRBLAST") {
        let p = std::path::PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    let exe = std::env::current_exe().ok()?;
    let p = exe.with_file_name("irblast");
    p.is_file().then_some(p)
}

/// 发红外要按名字找蓝牙外设。用电量那边同一个目标名，换遥控器时会跟着更新。
fn ir_device_name() -> String {
    crate::battery::target_name()
}

/// 让遥控器打一发红外。
///
/// 现在只做到**校验 + 编译**：把用户配的码解析出来、按固件的格式编译成载荷，
/// 错在哪儿立刻告诉他。真正的发射要走 BLE GATT 的 KeyMap 服务
/// （`FE151500` / `FE151503` BLAST），得像 battprobe 那样起个独立的
/// CoreBluetooth 小进程 —— 那部分还没写（见 CLAUDE.md「红外发射」）。
///
/// 这样排的好处：码配得对不对现在就能验，不用等发射通道。
fn ir_blast(tx: &Sender<Event>, act: &Action) -> String {
    match crate::ir::IrCode::parse(&act.arg) {
        Err(e) => {
            let m = format!("红外码有问题：{e}");
            let _ = tx.send(Event::Log(m.clone()));
            m
        }
        Ok(code) => match code.compile_payload() {
            Err(e) => {
                let m = format!("红外码编译失败：{e}");
                let _ = tx.send(Event::Log(m.clone()));
                m
            }
            Ok(_) => match code.compile_blast(0) {
                Err(e) => {
                    let m = format!("红外表编译失败：{e}");
                    let _ = tx.send(Event::Log(m.clone()));
                    m
                }
                Ok(table) => {
                    // 交给独立小进程走 GATT（在本进程里建 CBCentralManager 不回调，
                    // 和 battprobe 同一个坑）。异步跑，别卡住按键分发。
                    let name = ir_device_name();
                    let hex: String = table.iter().map(|b| format!("{b:02x}")).collect();
                    let tx2 = tx.clone();
                    let sum = code.summary();
                    std::thread::spawn(move || {
                        let Some(exe) = ir_blaster_path() else {
                            let _ = tx2.send(Event::Log("找不到 irblast，重新打包一次".into()));
                            return;
                        };
                        match std::process::Command::new(exe).arg(&name).arg(&hex).output() {
                            Ok(o) if o.status.success() => {
                                let _ = tx2.send(Event::Log(format!("红外已发射 · {sum}")));
                            }
                            Ok(o) => {
                                let err = String::from_utf8_lossy(&o.stderr);
                                let last = err.lines().last().unwrap_or("").to_string();
                                let _ = tx2.send(Event::Log(format!(
                                    "红外发射失败（退出码 {:?}）：{last}",
                                    o.status.code()
                                )));
                            }
                            Err(e) => {
                                let _ = tx2.send(Event::Log(format!("红外发射起不来：{e}")));
                            }
                        }
                    });
                    format!("红外发射 · {}", code.summary())
                }
            },
        },
    }
}

fn run_record(
    status: &Arc<Status>,
    recording: &Arc<Mutex<Option<crate::recorder::Rec>>>,
    tx: &Sender<Event>,
    down: bool,
) -> String {
    // 松手 → 收尾保存
    if !down {
        let taken = recording.lock().take();
        let Some(r) = taken else {
            return String::new();
        };
        status.mic_on.store(false, Ordering::Relaxed);
        return match r.finish() {
            Ok((path, secs)) if secs < 0.2 => {
                // 什么都没录到：多半是没按住实体麦克风键，删掉空文件别留垃圾
                let _ = std::fs::remove_file(&path);
                let _ = tx.send(Event::Log(
                    "没录到声音 —— 录音要按住遥控器的麦克风键才有音频".into(),
                ));
                "没录到声音".into()
            }
            Ok((path, secs)) => {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                let _ = tx.send(Event::Log(format!(
                    "录音已保存到「下载」：{name}（{secs:.1}s）"
                )));
                format!("录音结束 · {secs:.1}s")
            }
            Err(e) => {
                let _ = tx.send(Event::Log(format!("录音保存失败: {e}")));
                "录音保存失败".into()
            }
        };
    }
    // 按下 → 开始录（已经在录就别重开）
    if recording.lock().is_some() {
        return String::new();
    }
    match crate::recorder::Rec::start(OPUS_RATE) {
        Ok(r) => {
            *recording.lock() = Some(r);
            status.mic_on.store(true, Ordering::Relaxed);
            "开始录音".into()
        }
        Err(e) => {
            let _ = tx.send(Event::Log(format!("录音启动失败: {e}")));
            "录音启动失败".into()
        }
    }
}
