//! 把配置、HID 设备、语音、按键注入串起来。

use crate::config::{Action, ActionType, Config, VoiceMode};
use crate::device::*;
use crate::inject::{new_injector, Injector};
use crate::keys::{passthrough, Key};
use crate::voice::{VoiceSink, OPUS_FRAME, OPUS_RATE};
use anyhow::{anyhow, Context, Result};
use parking_lot::{Mutex, RwLock};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, AtomicUsize, Ordering};
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
    /// 开麦模型探测出结果了。UI 收到后要补一次红外表同步 ——
    /// 刚配对的遥控器在「连上」那一刻模型还是 Unknown，红外同步会被跳过，
    /// 不补的话要等下一次重连才写得进去。
    MicModelProbed,
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
    /// 还有几条 HID 线程活着（主读线程 + 各副 collection 的读线程）。
    /// `start()` 必须等它们**全部**退出再开新的 —— 同一个设备被两个线程
    /// 打开会把 CoreFoundation 的对象搞坏，崩在
    /// `IOHIDDeviceScheduleWithRunLoop` / `__CFCheckCFInfoPACSignature`。
    /// ⚠️ 计数在 `start()` 里**spawn 之前**就加上 —— 在线程体里才加的话，
    /// spawn 和真正跑起来之间有个窗口，下一次 start() 会以为没人活着。
    hid_threads: Arc<AtomicUsize>,
    /// 红外表写入在途锁。写一次要十几秒（还要等遥控器醒），而触发点有两个
    /// （编辑器保存 + 每次重连自动补写）—— 不锁的话两个 irblast 进程会同时
    /// 往同一组 GATT 特征里交错写分片，表被写坏**设备照样回 0x02 说成功**，
    /// 然后 CONTEXT_SWITCH 到一张坏表，按键行为从此不对。遥控器被写坏
    /// 大概率就是这么来的。
    ir_sync_busy: Arc<AtomicBool>,
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
                hid_threads: Arc::new(AtomicUsize::new(0)),
                ir_sync_busy: Arc::new(AtomicBool::new(false)),
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

    /// 语音链路建起来了吗。
    ///
    /// 界面**必须拿它当判据**去决定要不要重建，不能用一次性的标志位：
    /// `stop()` 里有 `stop_voice()`，而每次**重连尝试**都会先 `stop()` ——
    /// 遥控器一休眠掉线，sink 就被销毁了。界面那边如果只记着「建过了」，
    /// 就再也不会重建：之后按住说话，快捷键照发（那段在 sink 判断之外，
    /// 所以第三方工具照常弹出来），但**音频不进虚拟声卡、电平条不出、
    /// 系统输入也不切** —— 看起来像"能用但没声音"，极难往这儿想。
    pub fn has_voice(&self) -> bool {
        self.voice.lock().is_some()
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
                // 内置码（麦克风键 0xb1 = AC Search）**无条件吞**，不看连没连。
                // 遥控器休眠后第一下按键是「唤醒键」：事件到达时 app 还没抓到
                // 设备（重连要几百毫秒），connected 还是 false —— 原来这里直接
                // 放行，Spotlight 就弹出来了。这个码是遥控器专属（普通键盘
                // 不发 AC Search），一直拦没有误伤面。
                if crate::tap::BUILTIN_SUPPRESS.contains(&ev.code) {
                    return true;
                }
                // 遥控器没连着就别拦 —— 学来的码跟 Mac 自带键盘的功能键是
                // 同一套，一直拦着会把你自己键盘的键也吞了
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

    /// 按当前方案编一张键位表。返回（表的 hex, 配了码的键数）。
    /// 编译失败（比如某条码超长）返回一句能直接显示的话。
    fn build_ir_table_hex(&self) -> Result<(String, usize), String> {
        let cfg = self.cfg.read();
        let prof = cfg.profile();
        let mut codes: Vec<(crate::layout::Slot, Option<crate::ir::IrCode>)> = Vec::new();
        for slot in crate::irtable::IR_SLOTS {
            // 只看短按：表里没有「长按」这个概念，一个 scanId 一条码
            let code = prof
                .action(slot, false)
                .filter(|a| a.kind == crate::config::ActionType::IrBlast)
                .and_then(|a| crate::ir::IrCode::parse(&a.arg).ok());
            codes.push((slot, code));
        }
        let n = codes.iter().filter(|(_, c)| c.is_some()).count();
        let refs: Vec<_> = codes.iter().map(|(s, c)| (*s, c.as_ref())).collect();
        let table = crate::irtable::build(&refs).map_err(|e| format!("红外表编译失败：{e}"))?;
        Ok((crate::irtable::to_hex(&table), n))
    }

    /// 有没有一次红外写入正在进行（irblast 还没回来）
    pub fn ir_sync_in_flight(&self) -> bool {
        self.ir_sync_busy.load(Ordering::SeqCst)
    }

    /// 诊断钩子：`FIREVIBE_IR_WRITE=<蓝牙名片段>:<hex文件路径>` 时，启动后把
    /// 文件里的表**原样**写进遥控器（走 --mapping，不经过 irtable::build）。
    ///
    /// 为什么存在：排查「写了哪张表让固件出毛病」要做 A/B —— 比如把电视原表
    /// `tv_table.hex` 写回去对照。终端进程没有蓝牙授权（CoreBluetooth 对 shell
    /// 卡在 unauthorized），irblast 只能由 app 拉起（TCC 归责到 FireVibe.app），
    /// 所以从 app 里开这个口子。写入结果打到 stderr。
    pub fn maybe_debug_ir_write(&self) {
        let Ok(spec) = std::env::var("FIREVIBE_IR_WRITE") else {
            return;
        };
        let Some((name, path)) = spec.split_once(':') else {
            eprintln!("[irdbg] FIREVIBE_IR_WRITE 格式：<蓝牙名片段>:<hex文件路径>");
            return;
        };
        let hex = match std::fs::read_to_string(path) {
            Ok(s) => s.trim().to_string(),
            Err(e) => {
                eprintln!("[irdbg] 读不了 {path}: {e}");
                return;
            }
        };
        if self.ir_sync_busy.swap(true, Ordering::SeqCst) {
            eprintln!("[irdbg] 已有写入在途，跳过");
            return;
        }
        let busy = self.ir_sync_busy.clone();
        let name = name.to_string();
        std::thread::spawn(move || {
            struct Unlock(Arc<AtomicBool>);
            impl Drop for Unlock {
                fn drop(&mut self) {
                    self.0.store(false, Ordering::SeqCst);
                }
            }
            let _unlock = Unlock(busy);
            let Some(exe) = ir_blaster_path() else {
                eprintln!("[irdbg] 找不到 irblast");
                return;
            };
            eprintln!("[irdbg] 写入 {}（{} 字节表）—— 什么时候按遥控器都行，持续等 30 分钟", name, hex.len() / 2);
            // 循环重试：一轮等 60 秒，「等不到」（退出码 3）就再来一轮 ——
            // 用户不一定守在旁边，一次性的 120 秒窗口错过就白装了一趟。
            // 每轮新起 irblast（全新 CBCentralManager），比单个超长等待可靠。
            for round in 1..=30 {
                let out = std::process::Command::new(&exe)
                    .arg(&name)
                    .arg(&hex)
                    .args(["--mapping", "--uuid-rand", "--wait", "60"])
                    .output();
                match out {
                    Ok(o) if o.status.code() == Some(3) => {
                        eprintln!("[irdbg] 第 {round} 轮没等到遥控器，继续蹲");
                        continue;
                    }
                    Ok(o) => {
                        for l in String::from_utf8_lossy(&o.stderr).lines() {
                            eprintln!("[irdbg] {l}");
                        }
                        eprintln!(
                            "[irdbg] 退出码 {:?} stdout={}",
                            o.status.code(),
                            String::from_utf8_lossy(&o.stdout).trim()
                        );
                        return;
                    }
                    Err(e) => {
                        eprintln!("[irdbg] irblast 起不来: {e}");
                        return;
                    }
                }
            }
            eprintln!("[irdbg] 30 分钟都没等到遥控器，放弃 —— 重开 app 再试");
        });
    }

    /// 当前方案里的红外配置和遥控器里已写入的表**不一致**（有改动没写进去）。
    /// 顶栏的「写入红外」提示就看它。
    pub fn ir_table_pending(&self) -> bool {
        if !self.cfg.read().settings.mic_model.is_ptt() {
            return false; // 原厂走 blast，即配即用，没有「写入」这回事
        }
        let hash = self.cfg.read().settings.ir_table_hash.clone();
        match self.build_ir_table_hex() {
            // 编不出来（有超长码之类）也算「有改动」：让提示露出来，
            // 点了会得到具体报错，总比默默藏着强
            Err(_) => true,
            Ok((_, 0)) if hash.is_empty() => false, // 没配过也没写过，无事发生
            Ok((hex, _)) => hash != hex,
        }
    }

    /// 把当前方案里配的红外码**烧进仿品遥控器**（PID 0x0425 那条路）。
    ///
    /// 仿品没实现 blast，只能改它的持久化键位表 —— 所以红外不是「app 让它发」，
    /// 而是「按实体键它自己发」，电脑关着也照发。代价是只有 4 个键能挂
    /// （见 `irtable::scan_id`）、而且要先写进去，十几秒。
    ///
    /// **四行永远都写**，没配码的行只有 `BLE_KEYPRESS` —— 这样用户把动作删掉、
    /// 再点一次写入，就真的把那个键的红外清了，不会在遥控器里留着旧码。
    ///
    /// ⚠️ **只由用户手动触发**（顶栏的「写入红外」按钮）。以前保存动作、
    /// 连上遥控器都会自动写 —— 遥控器几秒睡一次、一按键就重连，写入窗口
    /// 又长达 90 秒，自动写等于把十几秒的 GATT 会话随机撒在使用过程里，
    /// 干扰正常使用还容易撞车。现在改动只点亮提示，写不写、什么时候写，用户说了算。
    ///
    /// 原厂遥控器（热麦克风那派）不走这条：它的 blast 是即时的，见 `ir_blast`。
    pub fn sync_ir_table(&self) -> Option<String> {
        if !self.cfg.read().settings.mic_model.is_ptt() {
            return None; // 原厂走 blast，不用烧表
        }
        let (hex, n) = match self.build_ir_table_hex() {
            Ok(v) => v,
            Err(e) => return Some(e),
        };
        let name = ir_device_name();
        if name.is_empty() {
            // 还没连上过，不知道该找哪台蓝牙外设。irblast 拿空串去 contains 匹配
            // 谁都配不上，白等 90 秒。
            return Some("还没连上过遥控器 —— 按一下它任意键、等它连上再写".into());
        }
        // 在途锁：写一次要十几秒（还要等遥控器醒），期间编辑器再保存 / 遥控器
        // 重连都会再触发一次。两个 irblast 同时往同一组 GATT 特征交错写分片，
        // 表被写坏设备**照样回 0x02**，然后 CONTEXT_SWITCH 到一张坏表 ——
        // 绝不能并发。在途时直接跳过：写完 hash 没更新的话，下一次重连会补。
        if self.ir_sync_busy.swap(true, Ordering::SeqCst) {
            return Some("上一次红外写入还在进行 —— 等它完成再点".into());
        }
        let busy = self.ir_sync_busy.clone();
        let tx = self.tx.clone();
        let cfg2 = self.cfg.clone();
        let hex2 = hex.clone();
        std::thread::spawn(move || {
            // 任何退出路径都要放锁（含 panic），否则红外同步从此永远被跳过
            struct Unlock(Arc<AtomicBool>);
            impl Drop for Unlock {
                fn drop(&mut self) {
                    self.0.store(false, Ordering::SeqCst);
                }
            }
            let _unlock = Unlock(busy);
            let Some(exe) = ir_blaster_path() else {
                let _ = tx.send(Event::Log("找不到 irblast，重新打包一次".into()));
                return;
            };
            let _ = tx.send(Event::Log(format!(
                "正在把 {n} 个红外码写进遥控器（十几秒，别关 app）…"
            )));
            // --mapping 写持久化键位表；--uuid-rand 因为表 id 随机就行（电视也是）；
            // --wait 让它等遥控器醒过来 —— 仿品空闲几十秒就睡，睡着时不广播，
            // 主机叫不醒它，只能等用户按键。
            let out = std::process::Command::new(exe)
                .arg(&name)
                .arg(&hex)
                .args(["--mapping", "--uuid-rand", "--wait", "90"])
                .output();
            let msg = match out {
                Ok(o) if o.status.success() => {
                    {
                        let mut c = cfg2.write();
                        // 清空之后把指纹也清掉：遥控器回到「我们没动过」的状态，
                        // 下次连上就不该再自动写了
                        c.settings.ir_table_hash = if n == 0 { String::new() } else { hex2 };
                        let _ = c.save();
                    }
                    if n == 0 {
                        "遥控器上的红外码已清空".to_string()
                    } else {
                        format!("{n} 个红外码已写进遥控器，按实体键就发（电脑关着也发）")
                    }
                }
                Ok(o) => {
                    let err = String::from_utf8_lossy(&o.stderr);
                    let last = err.lines().last().unwrap_or("").to_string();
                    if last.contains("等不到遥控器") {
                        "遥控器睡着了，写不进去 —— 按一下它任意键再试".to_string()
                    } else {
                        format!("红外表写入失败：{last}")
                    }
                }
                Err(e) => format!("红外表写入起不来：{e}"),
            };
            // 也进 stderr：只发 Event::Log 的话成功消息会被界面的过滤器丢掉，
            // 用户和排障都看不到「到底写没写进去」——今晚在这上面白等过一轮
            eprintln!("[firevibe] {msg}");
            let _ = tx.send(Event::Log(msg));
        });
        Some(if n == 0 {
            "正在清空遥控器上的红外码…".into()
        } else {
            format!("正在把 {n} 个红外码写进遥控器…")
        })
    }

    pub fn start(&self) -> Result<()> {
        // ⚠️ 停止标志必须在这儿复位。它是**跨连接存活**的（`Arc<AtomicBool>` 挂在
        // Runtime 上，不是每次 start 新建），而 `start_runtime` 永远是
        // 「先 stop 再 start」—— 不复位的话新起的读线程第一圈就 `break`。
        //
        // 症状极其像"设备坏了"：打开成功、Connected 事件照发、hidremap 照下、
        // 电量也读得到，就是**一个 HID 报文都收不到** —— 软遥控器不亮、
        // 按键没反应、语音没音频。而 firectl 单独跑却一切正常（它不走这条路）。
        //
        // ⚠️ 但复位**必须等旧读线程真的退出之后**。直接置 false 的话，旧线程
        // 可能压根没看到停止信号 —— 于是两个读线程同时开着同一个设备，
        // 而 hidapi 在 macOS 上会给每个设备起自己的 run loop，撞在一起就把
        // CoreFoundation 的对象写坏，崩在 `IOHIDDeviceScheduleWithRunLoop`
        // → `__CFCheckCFInfoPACSignature`（Trace/BPT trap）。
        self.stop.store(true, Ordering::Relaxed);
        for _ in 0..100 {
            if self.hid_threads.load(Ordering::Relaxed) == 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if self.hid_threads.load(Ordering::Relaxed) != 0 {
            // ⚠️ 等不到就**失败返回**，绝不硬闯。以前这里直接往下走，旧线程还
            // 开着设备、新线程又开一遍 —— hidapi 在 macOS 上给每个设备起自己的
            // run loop，撞在一起把 CF 对象写坏，直接 abort（不是 panic，是进程没了）。
            // 返回 HID_ERROR 的话 UI 300ms 后会自动重试，下一轮多半就干净了。
            self.stop.store(true, Ordering::Relaxed);
            return Err(anyhow!("HID_ERROR: 上一条连接还没退干净，稍后自动重试"));
        }
        self.stop.store(false, Ordering::Relaxed);
        let exclusive = self.cfg.read().exclusive;
        let api = hidapi::HidApi::new().context("hidapi 初始化失败")?;
        #[cfg(target_os = "macos")]
        api.set_open_exclusive(exclusive);
        // 错误分类用 ASCII 前缀，别让界面去匹配中文 ——
        // 原来消息里永远带「输入监控」四个字，结果「设备没连上」也被显示成权限问题。
        let (vid, pid) = self.cfg.read().device_ids();
        // ⚠️ **必须认准 vendor collection**，不能用 `api.open(vid, pid)`。
        //
        // macOS 把这支遥控器拆成三个 top-level collection：
        //   0x0001/0x06 键盘 · 0x000c/0x01 消费类 · **0x00ff/0x00 厂商**
        // `open(vid, pid)` 拿的是**枚举出来的第一个**（实测是键盘那个），
        // 而按键报文 0x02、音频报文 0xF0 都只从**厂商**那个 collection 出来。
        // 开错了就是「设备连上了、映射也下发了、语音链路也建好了，
        // 但一个报文都收不到」—— 界面上一切正常，按键却毫无反应。
        //
        // 枚举顺序不保证稳定，所以这不是"偶尔"，是"看运气"：重新配对之后
        // 顺序变了就从能用变成不能用。firectl 的诊断命令当初就是特意挑
        // 0x00ff 的（见 cli/src/main.rs 里那句「认准 vendor collection」），
        // 但这个知识一直没搬进来。
        // ⚠️ 以前这里只挑**一个** collection 打开（写死 0x00ff 厂商），但实测
        // 「按键报文从哪个 collection 出来」会随枚举顺序 / 配对状态 / 是否写过
        // 键位表而变 —— 押错一个就是「设备连上了、一个报文都收不到」，还查不出
        // 原因。所以现在**全部打开**：主 collection（优先厂商 0x00ff，命令和音频
        // 从它走）留在主读线程，其余的各起一条只转发报文的副线程，全部报文汇到
        // 主循环按 report id 处理 —— report id 本来就够区分，不用赌路由。
        // 按键状态集 `active` 是幂等的，就算同一报文从两条路各来一份也不会重复触发。
        // `FIREVIBE_HID_USAGE_PAGE` 仍可覆盖主 collection（十六进制；`any`=第一个）。
        let want_page: Option<u16> = match std::env::var("FIREVIBE_HID_USAGE_PAGE") {
            Ok(v) if v.eq_ignore_ascii_case("any") => None,
            Ok(v) => u16::from_str_radix(v.trim_start_matches("0x"), 16).ok(),
            Err(_) => Some(0x00ff),
        };
        let all: Vec<(std::ffi::CString, u16, u16)> = api
            .device_list()
            .filter(|d| d.vendor_id() == vid && d.product_id() == pid)
            .map(|d| (d.path().to_owned(), d.usage_page(), d.usage()))
            .collect();
        // ⚠️ 「没枚举到」要**直接返回**，不能穿过下面那个分类器。
        // 穿过去的话消息会变成 `HID_ERROR: HID_NOT_FOUND: …`（前缀套两层），
        // 而界面是靠 `starts_with("HID_NOT_FOUND")` 决定要不要显示
        // 「重试 / 重新配对」两个按钮的 —— 判断一失效，用户就**没有配对入口**了。
        // 分类器只认英文关键字，中文消息永远命不中，所以这里必须自己给前缀。
        if all.is_empty() {
            return Err(anyhow!("HID_NOT_FOUND: 没找到 {vid:#06x}/{pid:#06x}"));
        }
        let primary_idx = want_page
            .and_then(|pg| all.iter().position(|(_, p, _)| *p == pg))
            .unwrap_or(0);
        {
            let (_, pg, us) = &all[primary_idx];
            eprintln!(
                "[firevibe] 枚举到 {} 个 collection，主 collection usage_page 0x{pg:04x} usage 0x{us:02x}",
                all.len()
            );
        }
        let path = all[primary_idx].0.clone();
        let dev = api
            .open_path(&path)
            .map_err(|e| {
            let raw = e.to_string();
            // 0xE00002C1 (kIOReturnNotPrivileged / "privilege violation") 是
            // `IOHIDDeviceOpen` 在**没有「输入监控」权限**时的返回码 —— 枚举不需要
            // 权限，所以设备列得出来、就是打不开。以前它落进 HID_ERROR，界面只显示
            // 一句看不懂的英文，不提示去开授权，白查了很久。
            // （另一个码 0xE00002E2 not permitted 出现在 SetReport 上，见文档。）
            let kind = if raw.contains("not permitted")
                || raw.contains("0xE00002E2")
                || raw.contains("privilege violation")
                || raw.contains("0xE00002C1")
            {
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
        // 硬件层映射在这儿同步补（本函数总在后台线程跑）：遥控器休眠唤醒的
        // 那一下按键紧跟在连接建立后面，映射晚一步它就以原始键码漏进系统。
        // 以前是 UI 收到连接结果后才下发，多绕两轮 pump（几百毫秒），必输。
        // 在这儿也只是"尽量赢"—— 抢不过的那一下由 tap 的无条件拦截兜底。
        if let Some(m) = self.sync_hid_remap() {
            eprintln!("[firevibe] {m}");
        }
        self.log(if exclusive {
            "已独占打开设备 —— 系统收不到原始按键，映射不会重复触发"
        } else {
            "共享模式打开 —— 系统同时会收到原始按键"
        });

        // 其余 collection：各起一条副读线程，只把 (report id, 载荷) 转发给主循环。
        // 线程计数在 spawn **之前**加，Drop guard 减 —— 见 hid_threads 的注释。
        struct ThreadCount(Arc<AtomicUsize>);
        impl Drop for ThreadCount {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::SeqCst);
            }
        }
        let (fwd_tx, fwd_rx) = channel::<(u8, Vec<u8>)>();
        // 排障开关：FIREVIBE_HID_SINGLE=1 时只开主 collection（v0.2.1 的打开方式）。
        // 用来隔离「多 collection 全开」对遥控器行为的影响 —— 比如它的
        // 闲置断链计时器是否因此变了脾气。
        let single = std::env::var_os("FIREVIBE_HID_SINGLE").is_some();
        if single {
            eprintln!("[firevibe] FIREVIBE_HID_SINGLE=1：只开主 collection");
        }
        for (i, (p, pg, us)) in all.iter().enumerate() {
            if i == primary_idx || single {
                continue;
            }
            let sec = match api.open_path(p) {
                Ok(d) => d,
                Err(e) => {
                    // 开不成不算致命 —— 主 collection 还在。只记一笔。
                    eprintln!("[firevibe] 副 collection 0x{pg:04x}/0x{us:02x} 打不开：{e}");
                    continue;
                }
            };
            eprintln!("[firevibe] 副 collection usage_page 0x{pg:04x} usage 0x{us:02x} 已打开");
            let stop2 = self.stop.clone();
            let tx2 = fwd_tx.clone();
            self.hid_threads.fetch_add(1, Ordering::SeqCst);
            let count = ThreadCount(self.hid_threads.clone());
            let spawned = std::thread::Builder::new()
                .name(format!("firevibe-hid-{pg:04x}"))
                .spawn(move || {
                    let _count = count;
                    let mut buf = [0u8; 128];
                    loop {
                        if stop2.load(Ordering::Relaxed) {
                            break;
                        }
                        match sec.read_timeout(&mut buf, 200) {
                            Ok(0) => {}
                            Ok(n) => {
                                if tx2.send((buf[0], buf[1..n].to_vec())).is_err() {
                                    break; // 主循环没了
                                }
                            }
                            Err(_) => break, // 设备掉了，主线程那边会走断开流程
                        }
                    }
                });
            if spawned.is_err() {
                // spawn 失败时闭包（连同里面的计数 guard）被原地丢弃，
                // Drop 会把计数减回来，不会卡住下一次 start()
                eprintln!("[firevibe] 副读线程起不来");
            }
        }
        drop(fwd_tx); // 主循环握着 fwd_rx；发送端只留副线程那几份

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
        // 计数在 spawn 之前加 —— 线程体里才加的话，spawn 到跑起来之间
        // 下一次 start() 会以为没人活着，又开一遍同一个设备。
        self.hid_threads.fetch_add(1, Ordering::SeqCst);
        let main_count = ThreadCount(self.hid_threads.clone());

        let spawn_res = std::thread::Builder::new()
            .name("firevibe-hid".into())
            .spawn(move || {
                // 用 guard 清计数，保证任何退出路径（含 `?`、panic）都会清掉，
                // 否则 `start()` 会一直等一个已经死了的线程
                // ⚠️ 退出时必须**同时**清掉 `connected`。它以前只在「读报错」那条路上
                // 被清，读线程从别的路径退出（停止标志、探测失败、panic）就留着 true。
                // 而界面的重连是 `!connected()` 才触发的 —— 标志不清，
                // **300ms 重试永远不跑**，app 从此不再抓着设备。
                //
                // 后果比听起来严重：这台遥控器**只要 app 握着 HID 句柄就不休眠**，
                // 一旦不抓了，它几秒就掉线、而且再也回不来（没人去连它）。
                // 表现就是「刚配好能用几秒，然后彻底掉，重启 app 才好」。
                struct Alive(ThreadCount, Arc<Status>);
                impl Drop for Alive {
                    fn drop(&mut self) {
                        self.1.connected.store(false, Ordering::Relaxed);
                        // 退出以前是完全静默的：日志里只看到「连上了」，看不到「又断了」
                        eprintln!("[firevibe] HID 读线程退出");
                    }
                }
                let _alive = Alive(main_count, status.clone());
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

                // ── 开麦模型探测（在主循环里开窗口，不再单独阻塞 1.5 秒）──
                // 判据：**没人碰遥控器的时候发 MIC_ON，出不出流**。
                // 出流 = 热麦克风（它压根不看按键）；不出流 = PTT（只有按住才出）。
                // 结果存进配置，换设备时会被清掉重探（见 UI 的 pick_device）。
                //
                // ⚠️ 以前这是个独立的 1.5 秒小循环：既不查停止标志（start() 等半秒
                // 就硬闯 → 两个读线程开同一个设备 → CF 崩溃），期间的按键还全被
                // 吃掉（只数音频帧，别的报文读了就扔）。放进主循环两个毛病都没了。
                //
                // ⚠️ 判成 PTT 之后**照样继续发 MIC_ON/keepalive** —— 对 PTT 无害，
                // 而万一判错（比如探测时遥控器正好睡着了）也不会把热麦克风弄瘫。
                // 判型只用来关掉那个没意义的自愈关麦、和在界面上提醒绑定方式。
                let mut probe_until: Option<Instant> =
                    if cfg.read().settings.mic_model == crate::config::MicModel::Unknown {
                        let _ = dev.write(&MIC_ON);
                        Some(Instant::now() + Duration::from_millis(1500))
                    } else {
                        None
                    };
                let mut probe_frames = 0u32;

                // 实验开关：FIREVIBE_KEEPALIVE=<秒> 时每隔几秒发一条无害命令
                //（关麦，两派遥控器闲置时都等于空操作），试试 ATT 流量能不能
                // 拦住仿品「几秒就休眠断链」。没设就完全不跑。若验证有效再做成配置。
                let keepalive: Option<Duration> = std::env::var("FIREVIBE_KEEPALIVE")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
                    .filter(|s| *s > 0)
                    .map(Duration::from_secs);
                let mut last_alive = Instant::now();

                // 隐式按住会话（见 implied_session）：Some = 会话进行中，
                // 时间戳 = 最后一帧音频 + 400ms，到点视为实体键已松开。
                let mut implied: Option<(crate::config::Action, Instant)> = None;
                // 连按下报文带音频一起到的场景里，别抢在按下报文前面开会话：
                // 攒满 3 帧（约 60ms）还没见到按下才算数
                let mut implied_warmup: u8 = 0;

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
                    // 探测窗口到点：收针、判型、落盘
                    if probe_until.is_some_and(|t| Instant::now() >= t) {
                        probe_until = None;
                        let _ = dev.write(&MIC_OFF);
                        let model = if probe_frames > 3 {
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
                            "开麦模型：{}（静默发 MIC_ON 收到 {probe_frames} 帧）",
                            match model {
                                crate::config::MicModel::Hot => "热麦克风 —— 发命令就一直出流",
                                crate::config::MicModel::Ptt =>
                                    "按住才出流 —— 麦克风键要绑「按住」模式",
                                _ => "未知",
                            }
                        )));
                        // 通知 UI：连上那一刻模型还是 Unknown，
                        // 顶栏「写入红外」的判断这时才有效（UI 收到后会刷新提示）
                        let _ = tx.send(Event::MicModelProbed);
                    }
                    // 隐式按住会话收针：音频停了 400ms = 实体麦克风键已松开
                    // （PTT 遥控器松键即停流，这个判据很硬）
                    if implied.as_ref().is_some_and(|(_, t)| Instant::now() >= *t) {
                        let (a, _) = implied.take().unwrap();
                        implied_warmup = 0;
                        let r = implied_session(
                            &cfg, &status, &inj, &dictating, &tx, &voice, &prev_input, &a, false,
                        );
                        eprintln!("[firevibe] 音频停了 —— 隐式按住会话结束（{r}）");
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
                    // 实验保活（见上面 keepalive 的注释）。开麦期间不需要 ——
                    // 上面那条 MIC_ON keepalive 本身就是流量。
                    if let Some(iv) = keepalive {
                        if !mic_now && last_alive.elapsed() >= iv {
                            let _ = dev.write(&MIC_OFF);
                            last_alive = Instant::now();
                        }
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

                    // 本圈的报文：主 collection 直读一条 + 副线程转发来的全部。
                    // 超时从 200ms 收到 50ms —— 副 collection 的按键要等主读超时
                    // 才被捞出来，200ms 的按键延迟按着能感觉到。
                    let mut reports: Vec<(u8, Vec<u8>)> = Vec::new();
                    match dev.read_timeout(&mut buf, 50) {
                        Ok(0) => {}
                        Ok(n) => reports.push((buf[0], buf[1..n].to_vec())),
                        Err(e) => {
                            status.connected.store(false, Ordering::Relaxed);
                            let _ = tx.send(Event::Disconnected(e.to_string()));
                            break;
                        }
                    }
                    while let Ok(r) = fwd_rx.try_recv() {
                        reports.push(r);
                    }
                    if reports.is_empty() {
                        continue;
                    }
                    // 键报文排在音频前面处理：同一批里按下和首帧音频一起到时，
                    // 先看到按下，就不会误开隐式会话
                    reports.sort_by_key(|(rid, _)| u8::from(*rid == RID_AUDIO));
                    for (rid, payload) in reports {
                    let payload = &payload[..];
                    *seen_rids.lock().entry(rid).or_insert(0) += 1;
                    let raw_on = raw_all.load(Ordering::Relaxed);
                    if raw_on {
                        let _ = tx.send(Event::Raw {
                            report_id: rid,
                            data: payload.to_vec(),
                        });
                    }

                    match rid {
                        // 探测窗口期内的音频只归探测计数：这是我们自己发 MIC_ON
                        // 引出来的流，不该惊动自愈逻辑和界面的帧计数
                        RID_AUDIO if probe_until.is_some() => {
                            probe_frames += 1;
                        }
                        RID_AUDIO => {
                            status.audio_frames.fetch_add(1, Ordering::Relaxed);
                            // 隐式按住会话（见 implied_session 的注释）
                            if let Some((_, t)) = implied.as_mut() {
                                *t = Instant::now() + Duration::from_millis(400);
                            } else if cfg.read().settings.mic_model.is_ptt()
                                && !learn.load(Ordering::Relaxed)
                            {
                                let mic_seen = cfg
                                    .read()
                                    .slot_key(crate::layout::Slot::Mic)
                                    .map(|k| active.contains(&k) || held.contains_key(&k))
                                    .unwrap_or(false);
                                let busy = voice.lock().as_ref().map(|s| s.passing()).unwrap_or(false)
                                    || dictating.lock().is_some()
                                    || recording.lock().is_some();
                                if mic_seen || busy {
                                    implied_warmup = 0;
                                } else {
                                    implied_warmup = implied_warmup.saturating_add(1);
                                    if implied_warmup >= 3 {
                                        let act = cfg
                                            .read()
                                            .profile()
                                            .action(crate::layout::Slot::Mic, true)
                                            .filter(|a| match a.kind {
                                                ActionType::VoicePtt => true,
                                                ActionType::VoiceHotkey
                                                | ActionType::VoiceDictate => a.arg == "hold",
                                                _ => false,
                                            });
                                        if let Some(a) = act {
                                            let r = implied_session(
                                                &cfg, &status, &inj, &dictating, &tx, &voice,
                                                &prev_input, &a, true,
                                            );
                                            eprintln!(
                                                "[firevibe] 音频先到、没看到按下 —— 隐式开启按住会话（{r}）"
                                            );
                                            implied = Some((
                                                a,
                                                Instant::now() + Duration::from_millis(400),
                                            ));
                                        }
                                        implied_warmup = 0;
                                    }
                                }
                            }
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
                    } // for reports
                }
                let _ = dev.write(&MIC_OFF);
                pressed.lock().clear();
            });
        spawn_res?;
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
                let r = self.run_action_at(&act, true, Some(slot));
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
            return self.run_action_at(&act, true, Some(slot));
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
        self.run_action_at(act, down, None)
    }

    /// 和 [`Self::run_action`] 一样，但告诉它这个动作挂在哪个键上。
    /// 仿品遥控器的红外要按 scanId 找行，不知道是哪个键就没法把话说准。
    pub fn run_action_at(
        &self,
        act: &Action,
        down: bool,
        slot: Option<crate::layout::Slot>,
    ) -> String {
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
            ActionType::IrBlast => {
                let ptt = self.cfg.read().settings.mic_model.is_ptt();
                return ir_blast(&self.tx, act, ptt, slot);
                // slot 为 None 时（CLI 直接跑动作）只能说通用的那句
            }
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
        self.stop_light();
        self.status.mic_on.store(false, Ordering::Relaxed);
        self.stop_voice();
    }

    /// 只让 HID 读线程退出，**不动映射、不拆语音链路**。重连尝试用这个。
    ///
    /// 为什么不能在重连时用完整的 `stop()`：这台遥控器空闲几秒就掉线，
    /// 重连每 300ms 试一次，于是
    /// · `hidremap::clear()` 每秒被调三次 —— 白跑三个 hidutil 进程，
    ///   而且清掉和补上之间有窗口，正好按到麦克风键就弹 Spotlight；
    /// · `stop_voice()` 把 cpal 的输出流反复拆了又建 —— 纯浪费，
    ///   而且一旦界面那边没重建就彻底哑掉（见 `has_voice`）。
    ///
    /// 两样都是**跨连接存活**的东西：映射靠 `sync_hid_remap()` 幂等重下，
    /// sink 和 HID 连接本来就没关系。
    pub fn stop_light(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// 隐式按住会话：遥控器休眠后第一下就是按住说话时，「按下」事件发生在
/// 我们打开设备之前，永远看不到 —— 但这台 PTT 遥控器**只有实体麦克风键
/// 按住时才吐音频**，所以「音频来了而我们没见过按下」= 用户此刻正按着。
/// 用这个事实替他把会话开起来，第一下按住就能直接说话，不用再按第二下。
///
/// 不走 dispatch：它的 via_hw 分支假设「硬件按下事件已经进了系统」，而隐式
/// 场景里那一下要么被 tap 吞了、要么以原始键码漏走了，第三方工具什么都没
/// 收到，必须补合成按键（豆包实测认合成修饰键）。就算 hidremap 赢了竞速、
/// 硬件事件也到了，重复一份合成 down/up 对修饰键无害。
fn implied_session(
    cfg: &Arc<RwLock<Config>>,
    status: &Arc<Status>,
    inj: &Arc<dyn Injector>,
    dictating: &Arc<Mutex<Option<crate::stt::Recorder>>>,
    tx: &Sender<Event>,
    voice: &Arc<Mutex<Option<Arc<VoiceSink>>>>,
    prev: &Arc<Mutex<Option<u32>>>,
    act: &Action,
    down: bool,
) -> String {
    match act.kind {
        ActionType::VoicePtt => {
            let Some(sink) = voice.lock().clone() else {
                return "语音未启动".into();
            };
            gate_voice(cfg, status, &sink, prev, down, false);
            if down { "开始送流".into() } else { "停止送流".into() }
        }
        ActionType::VoiceHotkey => {
            if let Some(sink) = voice.lock().clone() {
                gate_voice(cfg, status, &sink, prev, down, down);
            }
            // 走硬件层映射的键**绝不补合成**：全局映射常驻事件系统，唤醒的
            // 那一下已经以硬件 rightoption 的身份进了系统（豆包这类只认硬件
            // 来源的工具就靠它）。再合成一份不但没用（合成的它不认），
            // 还会把它的热键状态机搞乱。这里只负责音频闸门。
            let via_hw = act.mods.is_empty()
                && crate::hidremap::usage_of(&act.key).is_some()
                && cfg.read().mic_remap_key().as_deref() == Some(act.key.as_str());
            if via_hw {
                return if down {
                    format!("第三方语音输入 · {}（硬件层）", act.key)
                } else {
                    "松开".into()
                };
            }
            if down {
                mark_hold(&act.key);
                let _ = inj.key_down(&act.key, &act.mods);
                format!("第三方语音输入 · {}（合成）", act.key)
            } else {
                hold_long_enough(&act.key);
                let _ = inj.key_up(&act.key, &act.mods);
                "松开".into()
            }
        }
        ActionType::VoiceDictate => gate_dictation(cfg, status, inj, dictating, tx, down),
        _ => String::new(),
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
        // 落盘节流：这段跑在 HID 读线程里，原来**每按一键**做一次
        // 「临时文件 + fsync + rename」——按键延迟里凭空多几毫秒盘刷，
        // 还磨 SSD。统计丢最近 10 秒无所谓（配置的其它保存路径也会顺带带上）。
        static LAST_SAVE: AtomicU64 = AtomicU64::new(0);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let last = LAST_SAVE.load(Ordering::Relaxed);
        if now.saturating_sub(last) >= 10 && LAST_SAVE.compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
            let _ = c.save();
        }
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
        ActionType::IrBlast => ir_blast(tx, &act, cfg.read().settings.mic_model.is_ptt(), Some(slot)),
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

/// 让遥控器打一发红外。**两种遥控器走的路完全相反**：
///
/// - **原厂 0x0421（热麦克风）**：实现了 blast —— 这里现编码、现写进
///   `FE151503`，遥控器立刻打一发。即时、不留痕、任意键都能绑。
/// - **仿品 0x0425（PTT）**：**没有** blast。红外是事先烧进它的键位表的
///   （见 [`Runtime::sync_ir_table`]），按实体键时**遥控器自己就发了** ——
///   等这个函数被调到，红外早已经出去了。所以这里什么都不做，只回一句话。
///
/// 这个区别不是实现偷懒，是固件决定的。仿品上试过 blast 的全套流程，
/// 设备回 `0x02` 说成功但什么都不发（见 NOTES.md）。
fn ir_blast(
    tx: &Sender<Event>,
    act: &Action,
    ptt: bool,
    slot: Option<crate::layout::Slot>,
) -> String {
    if ptt {
        // 仿品：红外已经由遥控器自己发出去了。这里只负责把话说清楚。
        if let Some(s) = slot {
            if !crate::irtable::supports_ir(s) {
                let m = "这个键在仿品遥控器上挂不了红外 —— 只有开关机 / 音量± / 静音四个键可以"
                    .to_string();
                let _ = tx.send(Event::Log(m.clone()));
                return m;
            }
        }
        return match crate::ir::IrCode::parse(&act.arg) {
            Ok(c) => format!("红外由遥控器自己发出 · {}", c.summary()),
            Err(e) => {
                let m = format!("红外码有问题：{e}");
                let _ = tx.send(Event::Log(m.clone()));
                m
            }
        };
    }
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
