//! 遥控器电量。
//!
//! **三条本进程内的路都不通**（都实测过）：
//!   1. HID INPUT 报文 `0x03` 里有电量，但**设备想发才发**，界面能空很久。
//!   2. 主动 `GetReport(Input, 0x03)` → `0xE00002F0 data was not found`，
//!      这台设备的 BLE HOGP 通路不给读。
//!   3. IORegistry 的 `BatteryPercent` 只有 Apple 自家设备才发布；
//!      `system_profiler` 的 `device_batteryLevelMain` 对遥控器也是空的。
//!
//! 能用的是 **GATT 标准电池服务**（`0x180F` / 特征 `0x2A19`）——
//! 笔记里说 macOS 隐藏的是 HID 服务 `0x1812`，电池服务不在此列，实测能读到。
//!
//! ⚠️ **蓝牙授权没点之前，症状是「什么都不发生」**：`CBCentralManager` 的状态
//! 永远停在 `Unknown(0)`，`centralManagerDidUpdateState` 一次都不来，
//! 既不报错也不超时，`tccd` / `bluetoothd` 的日志里也查不到任何一行。
//! 那个「"FireVibe" would like to use Bluetooth」的框只要还挂在屏幕上没答复，
//! **本机上别的进程申请蓝牙也一样卡住**（拿一个全新 bundle id 的 app 实测过）。
//! 我为此绕了很久：先怀疑 objc2、再怀疑进程内起不来、又怀疑签名和嵌套 bundle，
//! 全是错的。以后遇到「CoreBluetooth 毫无反应」，第一件事是截屏看有没有挂着的授权框。
//!
//! 具体实现走 `helper/battprobe.swift` 编出来的小程序：跑一次、往 stdout 打一个整数。
//! 之所以拆出去，是因为 Swift 写这段比 objc2 短得多，不是因为进程内不行。
//! ⚠️ 它必须是**裸二进制、由 FireVibe fork/exec 拉起**：这样 TCC 归责到父进程，
//! 用 FireVibe 的用途说明和那一份授权。要是打成 .app 走 LaunchServices，
//! 它就成了独立身份，得让用户再点一次名字莫名其妙的第二个授权框。

use std::path::PathBuf;
use std::sync::atomic::{AtomicI32, Ordering};

static LEVEL: AtomicI32 = AtomicI32::new(-1);
/// 上一次是「蓝牙没授权」——界面可以据此给提示
static NO_AUTH: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 蓝牙权限看起来没给
pub fn needs_bluetooth_permission() -> bool {
    NO_AUTH.load(Ordering::Relaxed)
}

/// 忘掉已读到的电量（换遥控器时调）—— 否则界面会显示上一个设备的陈旧值
pub fn forget() {
    LEVEL.store(-1, Ordering::Relaxed);
}

/// 最近一次读到的电量（1~100）
pub fn last() -> Option<i32> {
    let v = LEVEL.load(Ordering::Relaxed);
    (1..=100).contains(&v).then_some(v)
}

/// 辅助程序的位置：bundle 里的 Contents/MacOS/battprobe，
/// 开发时退回 /tmp（package.sh 会编到 bundle 里）
fn probe_path() -> Option<PathBuf> {
    // 诊断用：指向另一份新编的 battprobe，省得为改 helper 整包重签。蓝牙授权归责到
    // 父进程（FireVibe.app），helper 在不在 bundle 里都行。
    if let Ok(p) = std::env::var("FIREVIBE_PROBE") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    let exe = std::env::current_exe().ok()?;
    let p = exe.with_file_name("battprobe");
    p.is_file().then_some(p)
}

/// 读一次电量。**会阻塞几百毫秒到 8 秒**（辅助程序自己有超时），
/// 所以只在后台线程调。
pub fn read_blocking(name_contains: &str) -> Option<i32> {
    let p = probe_path()?;
    // 直接 fork/exec：TCC 会归责到 FireVibe，用它的
    // NSBluetoothAlwaysUsageDescription 和那一份蓝牙授权。
    let out = std::process::Command::new(&p)
        .arg(name_contains)
        .output()
        .ok()?;
    // 退出码：0 读到 / 2 蓝牙状态回调没来（多半没授权）/ 3 设备没连 /
    //         4 读到空值 / 5 连上了但没读完 / 134 被 TCC 杀（没有用途说明）
    let code = out.status.code().unwrap_or(-1);
    let raw = String::from_utf8_lossy(&out.stdout);
    if std::env::var_os("FIREVIBE_BATT_DEBUG").is_some() {
        for line in String::from_utf8_lossy(&out.stderr).lines() {
            eprintln!("[batt] probe: {line}");
        }
    }
    match code {
        0 => {}
        2 => {
            // 每 5 分钟重刷一遍这三行没意义，只在状态从「有授权」翻过来时说一次
            if !NO_AUTH.swap(true, Ordering::Relaxed) {
                eprintln!(
                    "[batt] 蓝牙没授权 —— 屏幕上那个「FireVibe 想使用蓝牙」的框点一下「允许」；"
                );
                eprintln!("[batt] 已经点过就去 系统设置 › 隐私与安全性 › 蓝牙 里勾上 FireVibe。");
                eprintln!("[batt] （框挂着没答复的时候，本机别的 app 申请蓝牙也会一起卡住）");
            }
        }
        3 => eprintln!("[batt] 遥控器没连着蓝牙"),
        134 => eprintln!("[batt] battprobe 被 TCC 杀了（蓝牙用途说明没归责到 app）"),
        c => eprintln!("[batt] battprobe 退出码 {c}"),
    }
    if code != 2 {
        NO_AUTH.store(false, Ordering::Relaxed);
    }
    let v: i32 = raw.trim().parse().ok()?;
    if (1..=100).contains(&v) {
        LEVEL.store(v, Ordering::Relaxed);
        return Some(v);
    }
    None
}

/// 要读哪台设备的电量（按蓝牙名字模糊匹配）。**换遥控器要跟着改** ——
/// 早先写死 "Amazon"，换成别的牌子（比如 BLE_TEST_412）就永远读不到、
/// 界面一直显示上一台的陈旧电量。
static TARGET: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
/// 目标一变就 +1，tracker 看到变化立刻重读（否则要等下一个 5 分钟周期）
static GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// 设定要读哪台设备（连上遥控器 / 配对新遥控器时调）。名字空就不读。
pub fn set_target(name_contains: &str) {
    if let Ok(mut g) = TARGET.lock() {
        if *g != name_contains {
            *g = name_contains.to_string();
            // 换目标了，旧值作废，并催 tracker 立刻重读
            LEVEL.store(-1, Ordering::Relaxed);
            GEN.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// 起一个后台线程定时读。`every_secs` 建议 300（电量变化慢，别频繁连蓝牙）。
/// 读哪台设备看 `set_target()`，每轮重新取 —— 中途换遥控器也能跟上。
pub fn spawn_tracker(every_secs: u64) {
    if probe_path().is_none() {
        eprintln!("[batt] 没找到 battprobe，跳过电量读取");
        return;
    }
    std::thread::Builder::new()
        .name("firevibe-batt".into())
        .spawn(move || {
            // 诊断钩子：FIREVIBE_GATT_DUMP=<设备名> 时先枚举一次 GATT 服务，
            // 用来判断某台遥控器的语音是不是走 GATT（HID 那条不出流时）。
            if let Ok(name) = std::env::var("FIREVIBE_GATT_DUMP") {
                if let Some(p) = probe_path() {
                    eprintln!("[gatt] 枚举 {name} 的 GATT 服务…");
                    match std::process::Command::new(&p).arg(&name).arg("--dump").output() {
                        Ok(o) => {
                            for line in String::from_utf8_lossy(&o.stderr).lines() {
                                eprintln!("[gatt] {line}");
                            }
                            eprintln!("[gatt] 退出码 {:?}", o.status.code());
                        }
                        Err(e) => eprintln!("[gatt] 起不来: {e}"),
                    }
                }
            }
            // 诊断钩子：FIREVIBE_GATT_LISTEN=<设备名> 时订阅该设备**所有** notify 特征，
            // 把推过来的字节按时间打出来（只订阅、不写任何特征）。用来验证国产遥控器的
            // 语音是不是走它自己的私有服务（FFF0 那一系按下麦克风键就自己推流）。
            // 时长用 FIREVIBE_GATT_LISTEN_SECS 调，默认 30 秒。
            if let Ok(name) = std::env::var("FIREVIBE_GATT_LISTEN") {
                if let Some(p) = probe_path() {
                    let secs = std::env::var("FIREVIBE_GATT_LISTEN_SECS")
                        .unwrap_or_else(|_| "30".into());
                    eprintln!("[listen] 监听 {name} 的 GATT notify，{secs} 秒 —— 现在按住麦克风键说话");
                    match std::process::Command::new(&p)
                        .arg(&name)
                        .arg("--listen")
                        .arg(format!("--secs={secs}"))
                        .output()
                    {
                        Ok(o) => {
                            for line in String::from_utf8_lossy(&o.stderr).lines() {
                                eprintln!("[listen] {line}");
                            }
                            eprintln!("[listen] 退出码 {:?}", o.status.code());
                        }
                        Err(e) => eprintln!("[listen] 起不来: {e}"),
                    }
                }
            }
            let mut seen_gen = u64::MAX; // 首轮必读
            loop {
                let g = GEN.load(Ordering::Relaxed);
                let want = TARGET.lock().ok().map(|t| t.clone()).unwrap_or_default();
                if !want.is_empty() {
                    if let Some(v) = read_blocking(&want) {
                        eprintln!("[batt] 蓝牙读到 {v}%（{want}）");
                    } else {
                        eprintln!("[batt] 没读到（找 {want}）");
                    }
                }
                seen_gen = g;
                // 分片睡：目标一变（GEN 变了）立刻醒来重读，不用等满一个周期
                let total = every_secs.max(60);
                for _ in 0..total {
                    std::thread::sleep(std::time::Duration::from_secs(1));
                    if GEN.load(Ordering::Relaxed) != seen_gen {
                        break;
                    }
                }
            }
        })
        .ok();
}
