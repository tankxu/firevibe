//! 原始 report 嗅探：把设备发来的**每一条** HID report 原样打印，
//! 包括 vendor report（0xEF / 0xF1）—— 四个 App 快捷键在键盘页/Consumer 页
//! 上零事件，怀疑就走这两条 vendor 通道。

use firevibe_core::{config::Config, runtime::{Event, Runtime}};
use std::collections::HashMap;
use std::io::Write;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

pub fn run() -> anyhow::Result<()> {
    let mut cfg = Config::load();
    cfg.exclusive = false; // 独占要 root，这里只读不拦

    let (rt, rx) = Runtime::new(cfg);
    rt.start()?;
    rt.learn.store(true, Ordering::Relaxed); // 只上报，不执行动作

    println!();
    println!("原始 report 嗅探 —— 每条报文都会打出来。");
    println!("请依次按下 **四个 App 快捷键**（prime / NETFLIX / Disney+ / hulu），");
    println!("每个按 2~3 次，中间停一下。60 秒后自动结束。");
    println!("（也可以顺便按别的键做对照）");
    println!("{}", "─".repeat(70));

    let t0 = Instant::now();
    let mut seen: HashMap<String, u32> = HashMap::new();
    let mut n = 0u32;

    while t0.elapsed() < Duration::from_secs(60) {
        while let Ok(ev) = rx.try_recv() {
            let t = t0.elapsed().as_secs_f32();
            match ev {
                Event::Raw { report_id, data } => {
                    let hex: String =
                        data.iter().map(|b| format!("{b:02X} ")).collect::<String>().trim_end().to_string();
                    let key = format!("0x{report_id:02X} | {hex}");
                    *seen.entry(key.clone()).or_insert(0) += 1;
                    n += 1;
                    println!("[{t:6.2}] VENDOR  id=0x{report_id:02X}  {hex}");
                }
                Event::Learned(k) => {
                    let key = format!("{} ({})", k, k.id());
                    *seen.entry(key.clone()).or_insert(0) += 1;
                    n += 1;
                    println!("[{t:6.2}] 按键    {k}   {}", k.id());
                }
                Event::Disconnected(e) => {
                    println!("连接断开: {e}");
                    return Ok(());
                }
                Event::Log(s) => println!("[{t:6.2}] · {s}"),
                _ => {}
            }
            let _ = std::io::stdout().flush();
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    rt.stop();

    println!("{}", "─".repeat(70));
    println!("共 {n} 条事件，去重后 {} 种：\n", seen.len());
    let mut v: Vec<_> = seen.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    for (k, c) in v {
        println!("  {c:>4} 次   {k}");
    }
    println!("\n如果 App 键完全没有对应条目，说明它们在 HID 层不发任何东西");
    println!("（可能只在配对到 Fire TV 时才由固件启用）。");
    Ok(())
}
