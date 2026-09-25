//! 隐私指示器归因用的对照探针 —— 只为回答「每 3 秒亮一下的那个点是谁点的」。
//!
//! app 里有两条 3 秒量级的路径，怀疑对象不同，必须分开跑：
//!   enum  —— 只做 cpal 设备枚举（= `loopback_status()` 干的事，理论上纯属性读）
//!   sink  —— 只建/拆 VoiceSink（= 语音链路重建那条，会开输出流 + 切默认输入）
//!   switch—— 只在真麦克风和虚拟声卡之间来回切默认输入设备
//!
//! 用法（每次只跑一个，盯着菜单栏那个点看 30 秒）：
//!   cargo run --release -p firevibe-core --example micdot -- enum
//!   cargo run --release -p firevibe-core --example micdot -- sink
//!   cargo run --release -p firevibe-core --example micdot -- switch
//!
//! ⚠️ 跑之前先把 FireVibe 退掉，否则分不清是谁在亮。

use std::time::{Duration, Instant};

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "enum".into());
    let dev = "FireVibe Mic";
    println!("模式：{mode}（每 3 秒一轮，Ctrl-C 退出）\n盯着菜单栏的隐私指示器，看它跟不跟着这个节拍闪。\n");

    loop {
        let t = Instant::now();
        match mode.as_str() {
            "enum" => {
                let n = firevibe_core::voice::list_output_devices().len();
                println!("[{:>5.0}ms] 枚举了 {n} 个输出设备", t.elapsed().as_secs_f32() * 1000.0);
            }
            "sink" => match firevibe_core::voice::VoiceSink::start(dev, 1.0) {
                Ok(s) => {
                    println!("[{:>5.0}ms] sink 建起来了，持有 1 秒再拆", t.elapsed().as_secs_f32() * 1000.0);
                    std::thread::sleep(Duration::from_secs(1));
                    drop(s);
                }
                Err(e) => println!("sink 建不起来：{e:#}"),
            },
            "switch" => {
                let devs = firevibe_core::audio::input_devices();
                let virt = devs.iter().find(|d| d.name.to_lowercase().contains("firevibe"));
                let real = devs.iter().find(|d| !d.name.to_lowercase().contains("firevibe"));
                if let (Some(v), Some(r)) = (virt, real) {
                    let _ = firevibe_core::audio::set_default_input(v.id);
                    std::thread::sleep(Duration::from_millis(500));
                    let _ = firevibe_core::audio::set_default_input(r.id);
                    println!("[{:>5.0}ms] {} → {}", t.elapsed().as_secs_f32() * 1000.0, v.name, r.name);
                } else {
                    println!("找不到虚拟声卡或真麦克风，跳过");
                }
            }
            other => {
                eprintln!("不认识的模式：{other}（只有 enum / sink / switch）");
                std::process::exit(2);
            }
        }
        std::thread::sleep(Duration::from_secs(3));
    }
}
