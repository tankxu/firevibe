//! 按键测绘：逐个提示按下，记录每个物理键的真实 HID usage。
//! 跑完把结果贴回 layout.rs 就不再需要「学习」功能。

use firevibe_core::{
    config::Config,
    keys::Key,
    layout::Slot,
    runtime::{Event, Runtime},
};
use std::collections::HashMap;
use std::io::Write;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

pub fn run() -> anyhow::Result<()> {
    let want_exclusive = std::env::args().any(|a| a == "--exclusive");

    // 独占打开能避免按键同时被系统吃掉（方向键滚页面、麦克风键开 Spotlight），
    // 但 macOS 上独占打开键盘类 HID 需要 root —— 非 root 会报
    // (0xE00002C1) privilege violation。所以先试独占，失败就退回共享模式。
    let mut exclusive = want_exclusive;
    let (rt, rx) = loop {
        let mut cfg = Config::load();
        cfg.exclusive = exclusive;
        let (rt, rx) = Runtime::new(cfg);
        match rt.start() {
            Ok(()) => break (rt, rx),
            Err(e) => {
                let msg = e.to_string();
                if exclusive && (msg.contains("privilege") || msg.contains("0xE00002C1")) {
                    println!("独占打开被拒（需要 root），退回共享模式。");
                    println!("注意：共享模式下按键会同时被系统收到 —— 方向键会滚动、麦克风键会开 Spotlight。");
                    println!("想避免的话用: sudo {} --map --exclusive\n",
                             std::env::args().next().unwrap_or_default());
                    exclusive = false;
                    continue;
                }
                return Err(e);
            }
        }
    };
    // 学习模式：让所有按键事件原样上报，不执行任何动作
    rt.learn.store(true, Ordering::Relaxed);

    println!();
    println!("按键测绘 —— 会依次提示按哪个键，按下即自动记录并前进。");
    if exclusive {
        println!("独占模式：按键不会影响系统。");
    } else {
        println!("共享模式：按键会同时被系统收到（方向键滚页面、麦克风键开 Spotlight），忍一下。");
    }
    println!("中途 Ctrl-C 可退出。");
    println!("某个键按了没反应（比如遥控器上没有），等 8 秒会自动跳过。");
    println!("{}", "─".repeat(64));

    let mut map: HashMap<Slot, Key> = HashMap::new();
    let mut dup: Vec<(Slot, Slot, Key)> = Vec::new();

    for slot in Slot::ALL {
        print!("  [{:>2}/21] 请按  {:<12} ", map.len() + 1, slot.label());
        let _ = std::io::stdout().flush();

        let deadline = Instant::now() + Duration::from_secs(8);
        let mut got: Option<Key> = None;
        while Instant::now() < deadline {
            while let Ok(ev) = rx.try_recv() {
                match ev {
                    Event::Learned(k) => {
                        if got.is_none() {
                            got = Some(k);
                        }
                    }
                    Event::Disconnected(e) => {
                        println!("\n  连接断开: {e}");
                        return Ok(());
                    }
                    _ => {}
                }
            }
            if got.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(15));
        }

        match got {
            Some(k) => {
                if let Some((other, _)) = map.iter().find(|(_, v)| **v == k) {
                    println!("→ {k}   ⚠ 与「{}」重复", other.label());
                    dup.push((*other, slot, k));
                } else {
                    println!("→ {k}");
                }
                map.insert(slot, k);
            }
            None => println!("→ 跳过（超时）"),
        }

        // 等所有键松开 + 去抖，避免一次按压填两格
        let calm = Instant::now() + Duration::from_millis(350);
        while Instant::now() < calm {
            while rx.try_recv().is_ok() {}
            if !rt.pressed.lock().is_empty() {
                // 还按着就继续等
                std::thread::sleep(Duration::from_millis(30));
                continue;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        while rx.try_recv().is_ok() {}
    }

    rt.stop();

    println!("{}", "─".repeat(64));
    println!("测绘完成：{}/21 个键有 usage", map.len());
    if !dup.is_empty() {
        println!("\n⚠ 有重复（同一个 usage 被多个位置记到，说明其中一次记错了）：");
        for (a, b, k) in &dup {
            println!("   {} 和 {} 都是 {k}", a.label(), b.label());
        }
    }
    let missing: Vec<_> = Slot::ALL.iter().filter(|s| !map.contains_key(s)).collect();
    if !missing.is_empty() {
        println!("\n没测到的位置：");
        for s in &missing {
            println!("   {}", s.label());
        }
    }

    println!("\n{}", "═".repeat(64));
    println!("粘贴到 core/src/layout.rs 的 default_key() 里：\n");
    for slot in Slot::ALL {
        match map.get(&slot) {
            Some(k) => {
                let page = if k.page == firevibe_core::keys::PAGE_KEYBOARD {
                    "PAGE_KEYBOARD"
                } else {
                    "PAGE_CONSUMER"
                };
                println!(
                    "            Slot::{:?} => Key::new({}, 0x{:04X}),",
                    slot, page, k.usage
                );
            }
            None => println!("            // Slot::{:?} => 未测到", slot),
        }
    }
    println!("\n{}", "═".repeat(64));
    println!("原始 JSON（备份用）：");
    let json: Vec<_> = Slot::ALL
        .iter()
        .filter_map(|s| map.get(s).map(|k| format!("{{\"slot\":\"{}\",\"page\":{},\"usage\":{}}}", s.id(), k.page, k.usage)))
        .collect();
    println!("[{}]", json.join(","));
    Ok(())
}
