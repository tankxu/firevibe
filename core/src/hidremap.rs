//! HID 设备层按键重映射（走 `hidutil`）。
//!
//! 为什么需要它：合成的修饰键事件在**事件层和状态层都和真键盘一模一样**了 ——
//! 键码、左右设备位、非合并位、全局修饰位、按住时长全部对齐，只差一个
//! `0x20000000`「进程合成标记」和 `pid != 0`。实测有的第三方语音工具**只认硬件来源**，
//! 那两位去不掉，所以合成这条路对它是死的。
//!
//! 设备层映射绕过了这个问题：遥控器的键在 HID 层就被改成目标键，
//! 系统收到的是**真硬件事件**（pid=0、没有合成标记），和你按键盘完全无法区分。
//!
//! ⚠️ 这是**进程外的系统状态**，我们退出了它还在。留着的话遥控器那颗键
//! 会一直是修饰键，所以：
//!   - 启动时按配置重新下发（幂等，顺带覆盖上次异常退出的残留）
//!   - 退出时必须清掉
//!   - 关掉开关时立刻清掉
//! 重启 macOS 也会清掉（映射不持久）。

use anyhow::{anyhow, Context, Result};
use std::process::Command;

/// 遥控器的 USB 标识 —— 映射只对这台设备生效，不碰用户自己的键盘
const VID: u16 = 0x0171;
const PID: u16 = 0x0421;

/// 麦克风键在 HID 里的来源 usage：Consumer page 0x0C，usage 0x0221 (AC Search)
const SRC_MIC: u64 = 0x0C00_00_0221;

fn matching() -> String {
    format!("{{\"ProductID\":0x{PID:04x},\"VendorID\":0x{VID:04x}}}")
}

/// 键名 → HID Keyboard page (0x07) 的 usage。只放修饰键 ——
/// 映射成普通键没意义（我们自己读 HID 就能干），修饰键才是合成搞不定的那类。
pub fn usage_of(key: &str) -> Option<u64> {
    let u: u64 = match key.to_ascii_lowercase().as_str() {
        "leftcontrol" => 0xE0,
        "leftshift" => 0xE1,
        "leftoption" | "leftalt" => 0xE2,
        "leftcmd" | "leftcommand" => 0xE3,
        "rightcontrol" => 0xE4,
        "rightshift" => 0xE5,
        "rightoption" | "rightalt" => 0xE6,
        "rightcmd" | "rightcommand" => 0xE7,
        _ => return None,
    };
    Some(0x0700_0000_00 | u)
}

/// 能映射成哪些键（给界面用）
pub const TARGETS: &[&str] = &[
    "rightoption",
    "rightcmd",
    "rightcontrol",
    "rightshift",
    "leftoption",
    "leftcmd",
    "leftcontrol",
    "leftshift",
];

fn run(set: &str) -> Result<String> {
    let out = Command::new("/usr/bin/hidutil")
        .args(["property", "--matching", &matching(), "--set", set])
        .output()
        .context("跑 hidutil 失败")?;
    if !out.status.success() {
        return Err(anyhow!(
            "hidutil 返回 {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into())
}

/// 把麦克风键映射成 `key`
pub fn apply(key: &str) -> Result<()> {
    let dst = usage_of(key).ok_or_else(|| anyhow!("{key:?} 不能作为映射目标（只支持修饰键）"))?;
    let set = format!(
        "{{\"UserKeyMapping\":[{{\"HIDKeyboardModifierMappingSrc\":0x{SRC_MIC:X},\
         \"HIDKeyboardModifierMappingDst\":0x{dst:X}}}]}}"
    );
    let out = run(&set)?;
    // hidutil 对没连上的设备是静默成功的，所以回读确认
    if !out.contains("HIDKeyboardModifierMappingDst") {
        return Err(anyhow!("hidutil 没有回报映射，遥控器可能没连上"));
    }
    Ok(())
}

/// 清掉映射。退出路径上会调好几次，做成幂等且不报错。
pub fn clear() {
    let _ = run("{\"UserKeyMapping\":[]}");
}

/// 现在设着映射吗
pub fn is_set() -> bool {
    Command::new("/usr/bin/hidutil")
        .args(["property", "--matching", &matching(), "--get", "UserKeyMapping"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("HIDKeyboardModifierMappingDst"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usages_are_right() {
        // HID Keyboard page 0x07 + 右 Option 的 usage 0xE6
        assert_eq!(usage_of("rightoption"), Some(0x7000000E6));
        assert_eq!(usage_of("leftcmd"), Some(0x7000000E3));
        assert_eq!(usage_of("a"), None, "普通键不该被接受");
        for k in TARGETS {
            assert!(usage_of(k).is_some(), "{k} 应该有 usage");
        }
    }

    #[test]
    fn matching_dict_targets_the_remote() {
        let m = matching();
        assert!(m.contains("0x0421") && m.contains("0x0171"), "{m}");
    }
}
