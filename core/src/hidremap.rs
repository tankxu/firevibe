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

/// 遥控器的 USB 标识 —— 映射只对这台设备生效，不碰用户自己的键盘。
/// 可被配置覆盖（见 Config::device_ids），所以这里是运行时传进来的。
pub const VID_DEFAULT: u16 = crate::device::VID;
pub const PID_DEFAULT: u16 = crate::device::PID;
static IDS: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(((VID_DEFAULT as u32) << 16) | PID_DEFAULT as u32);

/// 设置映射要匹配的设备。启动时按配置调一次。
pub fn set_ids(vid: u16, pid: u16) {
    IDS.store(
        ((vid as u32) << 16) | pid as u32,
        std::sync::atomic::Ordering::Relaxed,
    );
}

fn ids() -> (u16, u16) {
    let v = IDS.load(std::sync::atomic::Ordering::Relaxed);
    ((v >> 16) as u16, (v & 0xffff) as u16)
}

/// 麦克风键在 HID 里的来源 usage：Consumer page 0x0C，usage 0x0221 (AC Search)
const SRC_MIC: u64 = 0x0C00_00_0221;

/// 映射改成全局的之后不再用它下发（见 run 的注释），留着给以后
/// 可能的按设备诊断用
#[allow(dead_code)]
fn matching() -> String {
    let (vid, pid) = ids();
    format!("{{\"ProductID\":0x{pid:04x},\"VendorID\":0x{vid:04x}}}")
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
    // ⚠️ **全局映射，不带 --matching**。按设备匹配的映射只对「下发那一刻在线」
    // 的设备生效，断开重连就没了 —— 而这台遥控器闲置 8 秒必睡，每次说话的
    // 第一下都是「唤醒键」：它到达系统时我们还没来得及重新下发映射，于是
    // 以原始键码（AC Search）漏出去，Spotlight 弹出、第三方语音工具拿不到
    // 硬件修饰键，用户只能按两下。全局映射常驻在 HID 事件系统里、对**之后
    // 接入**的设备同样生效，唤醒那一下按键在到达的瞬间就被转成硬件来源的
    // 目标键。误伤面：源是 Consumer AC Search（0x0221），普通键盘不发这个
    // usage，实际只有遥控器命中。
    let out = Command::new("/usr/bin/hidutil")
        .args(["property", "--set", set])
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
    // 全局 set 的回显就是新映射本身；确认一下没有静默失败
    if !out.contains("HIDKeyboardModifierMappingDst") {
        return Err(anyhow!("hidutil 没有回报映射"));
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
        .args(["property", "--get", "UserKeyMapping"])
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
    /// 默认值和覆盖写在同一个测试里 —— IDS 是全局的，
    /// 拆成两个测试并行跑会互相踩。
    fn matching_dict_targets_the_remote() {
        let m = matching();
        assert!(
            m.contains("0x0421") && m.contains("0x0171"),
            "默认值不对: {m}"
        );

        set_ids(0x1234, 0x5678);
        let m = matching();
        assert!(
            m.contains("0x1234") && m.contains("0x5678"),
            "覆盖没生效: {m}"
        );

        set_ids(super::VID_DEFAULT, super::PID_DEFAULT);
        let m = matching();
        assert!(
            m.contains("0x0421") && m.contains("0x0171"),
            "还原失败: {m}"
        );
    }
}
