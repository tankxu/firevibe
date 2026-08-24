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

/// 最近一次读到的电量（1~100）
pub fn last() -> Option<i32> {
    let v = LEVEL.load(Ordering::Relaxed);
    (1..=100).contains(&v).then_some(v)
}

/// 辅助程序的位置：bundle 里的 Contents/MacOS/battprobe，
/// 开发时退回 /tmp（package.sh 会编到 bundle 里）
fn probe_path() -> Option<PathBuf> {
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

/// 起一个后台线程定时读。`every_secs` 建议 300（电量变化慢，别频繁连蓝牙）。
pub fn spawn_tracker(name_contains: &str, every_secs: u64) {
    if probe_path().is_none() {
        eprintln!("[batt] 没找到 battprobe，跳过电量读取");
        return;
    }
    let want = name_contains.to_string();
    std::thread::Builder::new()
        .name("firevibe-batt".into())
        .spawn(move || loop {
            if let Some(v) = read_blocking(&want) {
                eprintln!("[batt] 蓝牙读到 {v}%");
            }
            std::thread::sleep(std::time::Duration::from_secs(every_secs.max(60)));
        })
        .ok();
}
