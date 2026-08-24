//! 遥控器按键的逻辑标识。
//! 不硬编码「这个遥控器有哪些键」—— 型号差异大，靠学习模式识别。

use serde::{Deserialize, Serialize};
use std::fmt;

pub const PAGE_KEYBOARD: u16 = 0x07;
pub const PAGE_CONSUMER: u16 = 0x0C;
/// Amazon 私有页 —— 报告描述符里声明为 `06 ff 00`（Usage Page Vendor 0xFF00），
/// 四个 App 快捷键走这条：report 0xEF，按下发 A1..A4，松开发 00。
pub const PAGE_VENDOR: u16 = 0xFF00;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Debug)]
pub struct Key {
    pub page: u16,
    pub usage: u16,
}

impl Key {
    pub const fn new(page: u16, usage: u16) -> Self {
        Self { page, usage }
    }
    /// 配置文件里用的稳定 id
    pub fn id(&self) -> String {
        format!("{:02x}:{:04x}", self.page, self.usage)
    }
    pub fn name(&self) -> &'static str {
        usage_name(*self).unwrap_or("")
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match usage_name(*self) {
            Some(n) => write!(f, "{n}"),
            None => write!(f, "0x{:02X}/0x{:04X}", self.page, self.usage),
        }
    }
}

fn usage_name(k: Key) -> Option<&'static str> {
    Some(match (k.page, k.usage) {
        // ── 以下全部实机测绘所得（firevibe-cli --map / --sniff）──
        (PAGE_KEYBOARD, 0x0066) => "电源",
        (PAGE_KEYBOARD, 0x0052) => "上",
        (PAGE_KEYBOARD, 0x0051) => "下",
        (PAGE_KEYBOARD, 0x0050) => "左",
        (PAGE_KEYBOARD, 0x004F) => "右",
        (PAGE_KEYBOARD, 0x0058) => "OK",   // Keypad Enter，不是 0x28
        (PAGE_KEYBOARD, 0x00F1) => "返回", // Amazon 私有值，不是 Esc

        (PAGE_CONSUMER, 0x0221) => "麦克风", // AC Search
        (PAGE_CONSUMER, 0x0223) => "主页",   // AC Home
        (PAGE_CONSUMER, 0x0040) => "菜单",
        (PAGE_CONSUMER, 0x00B4) => "快退",
        (PAGE_CONSUMER, 0x00CD) => "播放/暂停",
        (PAGE_CONSUMER, 0x00B3) => "快进",
        (PAGE_CONSUMER, 0x00E2) => "静音",
        (PAGE_CONSUMER, 0x00E9) => "音量+",
        (PAGE_CONSUMER, 0x00EA) => "音量-",
        (PAGE_CONSUMER, 0x008D) => "TV", // Program Guide，不是 0x0089

        (PAGE_VENDOR, 0xA1) => "Prime Video 键",
        (PAGE_VENDOR, 0xA2) => "NETFLIX 键",
        (PAGE_VENDOR, 0xA3) => "Disney+ 键",
        (PAGE_VENDOR, 0xA4) => "hulu 键",

        // ── 以下未在本机出现，留着以防别的型号用到 ──
        (PAGE_KEYBOARD, 0x0028) => "回车",
        (PAGE_KEYBOARD, 0x0029) => "Esc",
        (PAGE_KEYBOARD, 0x002A) => "退格",
        (PAGE_CONSUMER, 0x00B0) => "播放",
        (PAGE_CONSUMER, 0x00B1) => "暂停",
        (PAGE_CONSUMER, 0x00B5) => "下一个",
        (PAGE_CONSUMER, 0x00B6) => "上一个",
        (PAGE_CONSUMER, 0x0224) => "返回(AC Back)",
        _ => return None,
    })
}

/// 独占模式下系统收不到原始按键，没绑定的键要自动直通，否则方向键会变哑
pub fn passthrough(k: Key) -> Option<&'static str> {
    Some(match (k.page, k.usage) {
        (PAGE_KEYBOARD, 0x4F) => "right",
        (PAGE_KEYBOARD, 0x50) => "left",
        (PAGE_KEYBOARD, 0x51) => "down",
        (PAGE_KEYBOARD, 0x52) => "up",
        (PAGE_KEYBOARD, 0x58) => "return", // 实测：OK 是 Keypad Enter
        (PAGE_KEYBOARD, 0xF1) => "escape", // 实测：返回是 0x00F1（Amazon 私有）
        (PAGE_KEYBOARD, 0x2A) => "backspace",
        (PAGE_KEYBOARD, 0x2B) => "tab",
        (PAGE_KEYBOARD, 0x2C) => "space",
        (PAGE_KEYBOARD, 0x4A) => "home",
        (PAGE_KEYBOARD, 0x4B) => "pageup",
        (PAGE_KEYBOARD, 0x4E) => "pagedown",
        // Consumer 页：音量 / 静音 / 播放 / 快进快退 —— 不配也该直接有反应
        (PAGE_CONSUMER, 0x00E9) => "volume_up",
        (PAGE_CONSUMER, 0x00EA) => "volume_down",
        (PAGE_CONSUMER, 0x00E2) => "mute",
        (PAGE_CONSUMER, 0x00CD) => "play",
        (PAGE_CONSUMER, 0x00B3) => "next",
        (PAGE_CONSUMER, 0x00B4) => "prev",
        _ => return None,
    })
}
