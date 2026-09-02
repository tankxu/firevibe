//! Opus 解码 + 送进虚拟声卡（BlackHole / VB-CABLE / PipeWire null sink）。
//!
//! 遥控器固定 16 kHz 单声道、20 ms 一帧（320 样本）、CELT-only WB。

use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

pub const OPUS_RATE: u32 = 16_000;
pub const OPUS_FRAME: usize = 320;

fn f32_to_bits(v: f32) -> u32 {
    v.to_bits()
}
fn bits_to_f32(v: u32) -> f32 {
    f32::from_bits(v)
}

struct Shared {
    ring: Mutex<VecDeque<f32>>,
    cap: usize,
    passing: AtomicBool,
    gain: AtomicU32,
    level: AtomicU32,
    dropped: AtomicU64,
    /// cpal 报了流错误（设备被拔/重配置）。置了就该重建。
    dead: AtomicBool,
    /// 输出回调累计消费的帧数。送流时它**不推进**=流停摆了（豆包收不到声，
    /// 而我们 push_pcm 照样在填缓冲、电平照动）。UI 靠它判「该不该重建」。
    out_frames: AtomicU64,
}

/// VoiceSink 是给 HID 读线程用的句柄；cpal 的 Stream 不是 Send，
/// 所以它单独活在自己的线程里，这边只共享环形缓冲和几个原子量。
pub struct VoiceSink {
    sh: Arc<Shared>,
    pub device_name: String,
    pub out_rate: u32,
    pub out_channels: u16,
    stop: Arc<AtomicBool>,
    // 重采样状态（只在推送线程用）
    rs_pos: Mutex<f64>,
    rs_prev: Mutex<f32>,
    rs_have: Mutex<bool>,
}

/// 排障：cpal 眼里的全部设备 + 它对输入/输出配置的判断。
/// 加这个是因为 output_devices() 会漏设备，得看清它到底怎么想的。
pub fn debug_devices() -> Vec<(String, String, String)> {
    let host = cpal::default_host();
    let mut v = Vec::new();
    if let Ok(it) = host.devices() {
        for d in it {
            let n = d.name().unwrap_or_else(|_| "?".into());
            let o = match d.default_output_config() {
                Ok(c) => format!("{}Hz/{}ch", c.sample_rate().0, c.channels()),
                Err(e) => format!("x {e}"),
            };
            let i = match d.default_input_config() {
                Ok(c) => format!("{}Hz/{}ch", c.sample_rate().0, c.channels()),
                Err(_) => "-".into(),
            };
            v.push((n, o, i));
        }
    }
    v
}

pub fn list_output_devices() -> Vec<String> {
    let host = cpal::default_host();
    // 同上：output_devices() 在 macOS 上漏掉纯输出设备，改成全量枚举再自己判断
    match host.devices() {
        Ok(it) => it
            .filter(|d| d.default_output_config().is_ok())
            .filter_map(|d| d.name().ok())
            .collect(),
        Err(_) => Vec::new(),
    }
}


/// 线性 RMS → 0..1 的电平表刻度。
///
/// 线性刻度对语音根本不能用：正常说话 RMS 只有 -27 dBFS ≈ 线性 0.045，
/// 22 格的表里点不亮 2 格，而且音节之间的停顿会直接掉到 0 —— 看着就是
/// 「抖两下又平了」。人耳是对数的，电平表也得是。
///
/// 映射：-55 dBFS → 空，-3 dBFS → 满。正常说话落在六成左右。
pub fn meter_norm(rms: f32) -> f32 {
    if rms <= 0.0 {
        return 0.0;
    }
    let db = 20.0 * rms.max(1e-5).log10();
    ((db + 55.0) / 52.0).clamp(0.0, 1.0)
}

/// 快起慢落。上升立刻跟手，下降拖一下，不然每 20ms 一帧的停顿都会画出来。
pub fn meter_smooth(prev: f32, now: f32) -> f32 {
    if now > prev {
        now
    } else {
        prev * 0.82 + now * 0.18
    }
}

impl VoiceSink {
    pub fn start(device_prefix: &str, gain: f32) -> Result<Self> {
        let host = cpal::default_host();
        let want = device_prefix.to_lowercase();
        // ⚠️ 别用 host.output_devices() —— 实测它在 macOS 上只返回**带输入流**的设备，
        // 纯输出设备（MacBook 扬声器、DELL、我们自己那块 FireVibe Bridge）一个都不出现。
        // 直接枚举全部设备按名字找，能不能输出交给 default_output_config 判断。
        let dev = host
            .devices()
            .context("枚举音频设备失败")?
            .filter(|d| d.default_output_config().is_ok())
            .find(|d| {
                d.name()
                    .map(|n| n.to_lowercase().starts_with(&want))
                    .unwrap_or(false)
            })
            .ok_or_else(|| anyhow!("找不到名字以 {device_prefix:?} 开头的输出设备"))?;

        let name = dev.name().unwrap_or_else(|_| "?".into());
        let def = dev.default_output_config().context("取默认输出配置失败")?;
        // ⚠️ **优先把输出流开成 16 kHz**（= OPUS_RATE，遥控器音频的原始采样率）。
        // 为什么：豆包这类语音工具按 16 kHz 读这块虚拟声卡，会把整条链路的消费节奏
        // 拉到 16 kHz；而设备 default_output_config 报的是 48 kHz。若按 48 kHz 上采样
        // 往里灌，就是「灌 48000/秒、只消费 16000/秒」，缓冲几秒就爆、Doubao 听到的
        // 永远是几秒前的音频（实测缓冲顶到 4 秒上限、每秒丢 3 万帧）。
        // 开成 16 kHz 后 ratio=1、不上采样，cpal 回调也按 16 kHz 要数据，产销平衡。
        // 设备硬件真实速率交给 CoreAudio 内部转换，不归我们管。
        let cfg = dev
            .supported_output_configs()
            .ok()
            .and_then(|mut it| {
                it.find(|r| {
                    r.min_sample_rate().0 <= OPUS_RATE && OPUS_RATE <= r.max_sample_rate().0
                })
                .map(|r| r.with_sample_rate(cpal::SampleRate(OPUS_RATE)))
            })
            .unwrap_or_else(|| def.clone());
        let out_rate = cfg.sample_rate().0;
        let out_ch = cfg.channels();
        eprintln!("[voice] 输出流 {out_rate} Hz / {out_ch} 声道（设备默认 {} Hz）", def.sample_rate().0);

        // 缓冲上限压到约 250ms —— 溢出丢**最旧**的（见 push_pcm），保证延迟封顶。
        // 之前是 4 秒 + 丢新的：一旦产销错配，延迟直接攒到 4 秒还留着旧音频。
        let cap = (out_rate as usize / 4).max(OPUS_RATE as usize / 4);
        let sh = Arc::new(Shared {
            ring: Mutex::new(VecDeque::with_capacity(cap)),
            cap,
            passing: AtomicBool::new(false),
            gain: AtomicU32::new(f32_to_bits(gain)),
            level: AtomicU32::new(0),
            dropped: AtomicU64::new(0),
            dead: AtomicBool::new(false),
            out_frames: AtomicU64::new(0),
        });
        let stop = Arc::new(AtomicBool::new(false));

        // cpal Stream 不是 Send，放到专属线程里活着
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
        {
            let sh = sh.clone();
            let stop = stop.clone();
            let cfg = cfg.clone();
            std::thread::Builder::new()
                .name("firevibe-audio".into())
                .spawn(move || {
                    let sh_err = sh.clone();
                    let build = || -> Result<cpal::Stream> {
                        let ch = out_ch as usize;
                        let s = dev.build_output_stream(
                            &cfg.config(),
                            move |out: &mut [f32], _: &cpal::OutputCallbackInfo| {
                                let mut ring = sh.ring.lock();
                                let mut n = 0u64;
                                for frame in out.chunks_mut(ch) {
                                    let v = ring.pop_front().unwrap_or(0.0);
                                    // 单声道复制到所有声道，不依赖驱动的声道映射
                                    for s in frame.iter_mut() {
                                        *s = v;
                                    }
                                    n += 1;
                                }
                                sh.out_frames.fetch_add(n, Ordering::Relaxed);
                            },
                            move |e| {
                                eprintln!("音频流错误: {e}");
                                sh_err.dead.store(true, Ordering::Relaxed);
                            },
                            None,
                        )?;
                        s.play()?;
                        Ok(s)
                    };
                    match build() {
                        Ok(stream) => {
                            let _ = ready_tx.send(Ok(()));
                            while !stop.load(Ordering::Relaxed) {
                                std::thread::sleep(std::time::Duration::from_millis(50));
                            }
                            drop(stream);
                        }
                        Err(e) => {
                            let _ = ready_tx.send(Err(e.to_string()));
                        }
                    }
                })?;
        }
        match ready_rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(anyhow!("打开音频流({name}): {e}")),
            Err(_) => return Err(anyhow!("打开音频流超时")),
        }

        Ok(Self {
            sh,
            device_name: name,
            out_rate,
            out_channels: out_ch,
            stop,
            rs_pos: Mutex::new(0.0),
            rs_prev: Mutex::new(0.0),
            rs_have: Mutex::new(false),
        })
    }

    pub fn set_passing(&self, b: bool) {
        self.sh.passing.store(b, Ordering::Relaxed);
    }
    pub fn passing(&self) -> bool {
        self.sh.passing.load(Ordering::Relaxed)
    }
    pub fn set_gain(&self, g: f32) {
        self.sh.gain.store(f32_to_bits(g), Ordering::Relaxed);
    }
    pub fn gain(&self) -> f32 {
        bits_to_f32(self.sh.gain.load(Ordering::Relaxed))
    }
    pub fn level(&self) -> f32 {
        bits_to_f32(self.sh.level.load(Ordering::Relaxed))
    }
    pub fn dropped(&self) -> u64 {
        self.sh.dropped.load(Ordering::Relaxed)
    }
    /// cpal 报过流错误吗（设备被拔/重配置）
    pub fn dead(&self) -> bool {
        self.sh.dead.load(Ordering::Relaxed)
    }
    /// 输出回调累计消费的帧数。送流时它不涨=流停摆了，该重建。
    pub fn out_frames(&self) -> u64 {
        self.sh.out_frames.load(Ordering::Relaxed)
    }

    /// 推一帧解码后的 16-bit 单声道 PCM（16 kHz）
    pub fn push_pcm(&self, pcm: &[i16]) {
        if !self.passing() || pcm.is_empty() {
            return;
        }
        let g = self.gain();
        let ratio = self.out_rate as f64 / OPUS_RATE as f64;

        // 电平给 UI —— 算的是**加完增益之后**的值，也就是真正送出去的那份。
        // 分贝刻度 + 快起慢落，见 meter_norm / meter_smooth 的说明。
        let sum: f64 = pcm
            .iter()
            .map(|&s| {
                let x = s as f64 / 32768.0 * g as f64;
                x * x
            })
            .sum();
        let rms = (sum / pcm.len() as f64).sqrt() as f32;
        let prev = bits_to_f32(self.sh.level.load(Ordering::Relaxed));
        let lv = meter_smooth(prev, meter_norm(rms));
        self.sh.level.store(f32_to_bits(lv), Ordering::Relaxed);

        let mut ring = self.sh.ring.lock();
        let mut pos = self.rs_pos.lock();
        let mut prev = self.rs_prev.lock();
        let mut have = self.rs_have.lock();
        for &s in pcm {
            let cur = s as f32 / 32768.0 * g;
            if !*have {
                *prev = cur;
                *have = true;
                *pos = 0.0;
            }
            while *pos < 1.0 {
                let v = *prev + (cur - *prev) * (*pos as f32);
                ring.push_back(v);
                // 溢出丢**最旧**的:延迟必须封顶。留着旧音频只会让 Doubao 一直
                // 听几秒前的话(之前丢新的、留旧的 4 秒,就是这个 bug)。
                while ring.len() > self.sh.cap {
                    ring.pop_front();
                    self.sh.dropped.fetch_add(1, Ordering::Relaxed);
                }
                *pos += 1.0 / ratio;
            }
            *pos -= 1.0;
            *prev = cur;
        }
        // 诊断：FIREVIBE_AUDIO_DEBUG=1 时每约 1 秒打一次环形缓冲深度(=下游延迟)。
        // 深度大 → 延迟在 ring→虚拟声卡这段；深度一直很浅 → 延迟在上游(遥控器/HID)。
        if std::env::var_os("FIREVIBE_AUDIO_DEBUG").is_some() {
            use std::sync::atomic::AtomicU32;
            static N: AtomicU32 = AtomicU32::new(0);
            if N.fetch_add(1, Ordering::Relaxed) % 50 == 0 {
                let depth = ring.len();
                eprintln!(
                    "[audio] 缓冲深度 {depth} 采样 ≈ {}ms，累计丢帧 {}",
                    depth as u32 * 1000 / self.out_rate,
                    self.sh.dropped.load(Ordering::Relaxed)
                );
            }
        }
    }
}

impl Drop for VoiceSink {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

// ---------------- 虚拟声卡检测 ----------------

#[derive(Clone, Debug, PartialEq)]
pub enum LoopbackStatus {
    /// 还没查（枚举 CoreAudio 要跑 run loop，不能在 UI 构造期同步做）
    Unknown,
    /// 驱动装了，CoreAudio 也认到了
    Ready { name: String },
    /// 驱动文件在，但音频服务还没加载（需要重启 coreaudiod）
    InstalledNotLoaded,
    /// 完全没装
    Missing,
}

impl LoopbackStatus {
    pub fn label(&self) -> String {
        match self {
            LoopbackStatus::Unknown => "检测中".into(),
            LoopbackStatus::Ready { name } => format!("{name} 就绪"),
            LoopbackStatus::InstalledNotLoaded => "驱动已装，音频服务未加载".into(),
            LoopbackStatus::Missing => "未安装".into(),
        }
    }
    pub fn hint(&self) -> &'static str {
        match self {
            LoopbackStatus::Unknown | LoopbackStatus::Ready { .. } => "",
            LoopbackStatus::InstalledNotLoaded => {
                "执行 sudo killall coreaudiod 让 CoreAudio 重新加载"
            }
            LoopbackStatus::Missing => "执行 brew install blackhole-2ch 安装虚拟声卡",
        }
    }
    pub fn is_ready(&self) -> bool {
        matches!(self, LoopbackStatus::Ready { .. })
    }
    /// 后台还没查出结果
    pub fn is_unknown(&self) -> bool {
        matches!(self, LoopbackStatus::Unknown)
    }
}

/// 检查虚拟声卡（BlackHole / VB-CABLE / null-sink）状态
pub fn loopback_status(prefix: &str) -> LoopbackStatus {
    let want = prefix.to_lowercase();
    if let Some(name) = list_output_devices()
        .into_iter()
        .find(|d| d.to_lowercase().starts_with(&want))
    {
        return LoopbackStatus::Ready { name };
    }
    #[cfg(target_os = "macos")]
    {
        // 驱动文件在但 CoreAudio 没认到 -> 需要重启 coreaudiod
        if let Ok(rd) = std::fs::read_dir("/Library/Audio/Plug-Ins/HAL") {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().to_lowercase();
                // 只认我们自己那块的驱动文件夹（FireVibeMic.driver）——
                // 别把机器上已有的真 BlackHole 当成「我们的装了但没加载」。
                if n.starts_with(&want) || n.contains("firevibemic") {
                    return LoopbackStatus::InstalledNotLoaded;
                }
            }
        }
    }
    LoopbackStatus::Missing
}

/// 虚拟声卡自检：往声卡**输出**端写一段测试音，同时从它的**输入**端录回来。
///
/// 为什么需要这个：测试面板里的电平读的是我们自己的 `VoiceSink`，
/// 只证明「遥控器音频解码出来了」，完全不证明「音频真的进了虚拟声卡」。
/// 豆包这类工具听的是系统默认输入，也就是声卡的输入端 —— 那一端有没有信号，
/// 只有真录一遍才知道。
///
/// 返回 (发出去的 RMS, 录回来的 RMS)。
pub fn loopback_selftest(device_prefix: &str, secs: f32) -> Result<(f32, f32)> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    let sink = VoiceSink::start(device_prefix, 1.0)?;
    sink.set_passing(true);

    let host = cpal::default_host();
    let want = device_prefix.to_lowercase();
    let dev = host
        .input_devices()
        .context("枚举输入设备失败")?
        .find(|d| {
            d.name()
                .map(|n| n.to_lowercase().starts_with(&want))
                .unwrap_or(false)
        })
        .ok_or_else(|| anyhow!("找不到名字以 {device_prefix:?} 开头的输入设备"))?;
    let icfg = dev.default_input_config().context("取默认输入配置失败")?;

    let acc = Arc::new(Mutex::new((0.0f64, 0usize)));
    let acc2 = acc.clone();
    // cpal Stream 不是 Send，录音流也得关在自己的线程里
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    std::thread::spawn(move || {
        let build = || -> Result<cpal::Stream> {
            let s = dev.build_input_stream(
                &icfg.config(),
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    let mut g = acc2.lock();
                    for v in data {
                        g.0 += (*v as f64) * (*v as f64);
                        g.1 += 1;
                    }
                },
                |e| eprintln!("[loopback] 录音流出错: {e}"),
                None,
            )?;
            s.play()?;
            Ok(s)
        };
        match build() {
            Ok(s) => {
                let _ = ready_tx.send(Ok(()));
                while !stop2.load(Ordering::Relaxed) {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                drop(s);
            }
            Err(e) => {
                let _ = ready_tx.send(Err(e.to_string()));
            }
        }
    });
    match ready_rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(anyhow!("打不开输入端: {e}")),
        Err(_) => return Err(anyhow!("打开输入端超时")),
    }

    // 440Hz 正弦，按遥控器的采样率（16k）一帧一帧喂，跟真实链路一致
    let n = (OPUS_RATE as f32 * secs) as usize;
    let mut sent = 0.0f64;
    let mut buf = vec![0i16; OPUS_FRAME];
    let mut phase = 0.0f32;
    let step = std::f32::consts::TAU * 440.0 / OPUS_RATE as f32;
    let mut done = 0usize;
    while done < n {
        for s in buf.iter_mut() {
            let v = (phase.sin() * 0.35 * i16::MAX as f32) as i16;
            phase += step;
            *s = v;
            sent += (v as f64 / i16::MAX as f64).powi(2);
        }
        sink.push_pcm(&buf);
        done += buf.len();
        // 实时喂，别一次灌爆环形缓冲
        std::thread::sleep(std::time::Duration::from_millis(
            (buf.len() as u64 * 1000) / OPUS_RATE as u64,
        ));
    }
    std::thread::sleep(std::time::Duration::from_millis(300));
    stop.store(true, Ordering::Relaxed);
    sink.set_passing(false);

    let (sq, cnt) = *acc.lock();
    let heard = if cnt == 0 {
        0.0
    } else {
        (sq / cnt as f64).sqrt() as f32
    };
    let sent_rms = (sent / done as f64).sqrt() as f32;
    Ok((sent_rms, heard))
}
