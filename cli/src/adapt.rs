//! 遥控器适配 —— 换一款遥控器时跑这个，三步走完直接写进配置。
//!
//! 为什么放 CLI 不放界面：这是个一次性的、强交互的向导（选设备、逐键按、
//! 看报文），塞进界面既占地方又不好做。

use firevibe_core::{
    config::Config,
    device::HidDev,
    layout::Slot,
    runtime::{Event, Runtime},
};
use std::io::Write;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

fn ask(prompt: &str) -> String {
    print!("{prompt}");
    let _ = std::io::stdout().flush();
    let mut s = String::new();
    let _ = std::io::stdin().read_line(&mut s);
    s.trim().to_string()
}

pub fn run() -> anyhow::Result<()> {
    println!();
    println!("╭─ FireVibe 遥控器适配 ─────────────────────────────╮");
    println!("│ 换一款遥控器时跑这个。全程回车即可跳过当前步骤。  │");
    println!("╰──────────────────────────────────────────────────╯");

    step1_pick_device()?;
    step2_map_keys()?;
    Ok(())
}

/// 第一步：从已连接的 HID 设备里挑出遥控器，写进配置
fn step1_pick_device() -> anyhow::Result<()> {
    let cfg = Config::load();
    let (cur_vid, cur_pid) = cfg.device_ids();
    println!("\n【第一步】选择你的遥控器");
    println!("现在用的标识：0x{cur_vid:04x} / 0x{cur_pid:04x}");
    println!("（先确认遥控器已经在 系统设置 › 蓝牙 里连上了）\n");

    let devs = firevibe_core::device::list_hid();
    if devs.is_empty() {
        println!("  没扫到任何 HID 设备。先去蓝牙里把遥控器连上，再重跑。");
        return Ok(());
    }
    for (i, d) in devs.iter().enumerate() {
        let mark = if d.vid == cur_vid && d.pid == cur_pid {
            "  ← 正在使用"
        } else {
            ""
        };
        let vendor = if d.vendor.is_empty() { "厂商未知" } else { &d.vendor };
        println!("  {:>2}) {:<30} {}  {vendor}{mark}", i + 1, d.label(), d.ids());
    }

    let ans = ask("\n输入序号选择（直接回车＝不改）: ");
    if ans.is_empty() {
        println!("  保持不变。");
        return Ok(());
    }
    let Some(d) = ans.parse::<usize>().ok().and_then(|n| devs.get(n.wrapping_sub(1))) else {
        println!("  序号不对，保持不变。");
        return Ok(());
    };
    save_device(d)?;
    println!("  已选「{}」（0x{:04x} / 0x{:04x}），写进配置了。", d.label(), d.vid, d.pid);
    println!("  重开 FireVibe 就会连它。");
    Ok(())
}

fn save_device(d: &HidDev) -> anyhow::Result<()> {
    let mut cfg = Config::load();
    cfg.settings.device_vid = Some(format!("0x{:04x}", d.vid));
    cfg.settings.device_pid = Some(format!("0x{:04x}", d.pid));
    cfg.save()?;
    firevibe_core::hidremap::set_ids(d.vid, d.pid);
    Ok(())
}

/// 第二步：逐键按一遍，直接写进配置（不像 --map 那样只打印源码）
fn step2_map_keys() -> anyhow::Result<()> {
    println!("\n【第二步】重新认键");
    println!("只在「连上了但按键不对」时才需要。");
    if ask("现在做吗？(y / 回车跳过): ").to_lowercase() != "y" {
        println!("  跳过。");
        return step3_watch_reports();
    }

    let mut cfg = Config::load();
    cfg.voice.enabled = false;
    let (rt, rx) = Runtime::new(cfg);
    if let Err(e) = rt.start() {
        println!("  打不开遥控器：{e:#}");
        println!("  先做完第一步、并确认蓝牙已连上。");
        return Ok(());
    }
    rt.set_learn(true);

    println!("\n会依次提示按哪个键，按下即记录并前进。");
    println!("遥控器上没有的键等 8 秒自动跳过；Ctrl-C 随时退出（已记的会保存）。");
    println!("{}", "─".repeat(58));

    let mut got = 0usize;
    for (i, slot) in Slot::ALL.into_iter().enumerate() {
        print!("  [{:>2}/{}] 请按「{}」… ", i + 1, Slot::ALL.len(), slot.label());
        let _ = std::io::stdout().flush();
        let t0 = Instant::now();
        let mut done = false;
        while t0.elapsed() < Duration::from_secs(8) {
            if let Ok(Event::Learned(k)) = rx.recv_timeout(Duration::from_millis(120)) {
                println!("记下 {k}");
                rt.cfg.write().set_slot(slot, k);
                got += 1;
                done = true;
                break;
            }
        }
        if !done {
            println!("跳过");
        }
        // 清掉这一轮攒下的重复事件，免得下一个键被上一次的按键顶掉
        while rx.try_recv().is_ok() {}
    }

    let _ = rt.cfg.read().save();
    rt.set_learn(false);
    println!("{}", "─".repeat(58));
    println!("  记下了 {got} / {} 个键，已写进配置。", Slot::ALL.len());
    let r = step3_watch_reports_with(&rt, &rx);
    rt.stop();
    r
}

fn step3_watch_reports() -> anyhow::Result<()> {
    let mut cfg = Config::load();
    cfg.voice.enabled = false;
    let (rt, rx) = Runtime::new(cfg);
    if rt.start().is_err() {
        println!("\n【第三步】跳过（遥控器没连上）");
        return Ok(());
    }
    let r = step3_watch_reports_with(&rt, &rx);
    rt.stop();
    r
}

/// 第三步：看它到底发了什么报文 —— 判断语音通路是否和我们实现的一致
fn step3_watch_reports_with(
    rt: &Runtime,
    _rx: &std::sync::mpsc::Receiver<Event>,
) -> anyhow::Result<()> {
    println!("\n【第三步】看它发了什么报文");
    println!("我们实现的语音通路是 Fire TV 3rd Gen 那套：");
    println!("  HID vendor report 0xF2 开麦 → 0xF0 吐音频 → Opus 解码");
    println!("别的遥控器可能走 ATVV GATT + ADPCM（Android TV 那类），那条没实现。");
    println!("\n请在 15 秒内：随便按几个键，然后**按住麦克风键说两句话**。\n");

    rt.seen_rids.lock().clear();
    // 把麦克风打开，不然遥控器不会吐音频流
    rt.status.mic_on.store(true, Ordering::Relaxed);
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_secs(15) {
        std::thread::sleep(Duration::from_millis(500));
        let n = rt.status.audio_frames.load(Ordering::Relaxed);
        print!("\r  已收到 {} 种报文，音频帧 {n}      ", rt.seen_rids.lock().len());
        let _ = std::io::stdout().flush();
    }
    rt.status.mic_on.store(false, Ordering::Relaxed);
    println!("\n");

    let seen = rt.seen_rids.lock().clone();
    if seen.is_empty() {
        println!("  一条报文都没收到 —— 第一步的设备可能选错了。");
        return Ok(());
    }
    for (rid, cnt) in &seen {
        let what = match *rid {
            0x01 => "键盘键（方向、OK 这些）",
            0x02 => "多媒体键（音量、播放）",
            0x03 => "电量",
            0xF0 => "音频流 ← 关键",
            0xEF | 0xF1 | 0xF2 => "厂商私有报文",
            _ => "没见过的类型",
        };
        println!("  0x{rid:02X}  {what:<26} {cnt} 条");
    }
    let audio = seen.get(&0xF0).copied().unwrap_or(0);
    println!();
    if audio > 0 {
        println!("  ✓ 语音通路一致：收到 {audio} 帧 0xF0 音频，和 Fire TV 3rd Gen 是同一条。");
        println!("    麦克风相关功能应该都能用。");
    } else {
        println!("  ✗ 没收到 0xF0 音频帧。");
        println!("    可能是没按住麦克风键，也可能它的语音走另一条通路。");
        println!("    把上面这份报文清单发回来，就能判断是哪种。");
    }
    Ok(())
}
