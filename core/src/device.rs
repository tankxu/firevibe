//! Fire TV 遥控器的 HID 协议常量与报告解析。
//! 全部字节来自实机逆向，细节见 ~/LocalDev/firetv-remote-mac/NOTES.md

pub const VID: u16 = 0x0171;
pub const PID: u16 = 0x0421;

pub const RID_KEYBOARD: u8 = 0x01;
pub const RID_CONSUMER: u8 = 0x02;
pub const RID_BATTERY: u8 = 0x03;
pub const RID_AUDIO: u8 = 0xF0;
pub const RID_VENDOR_EF: u8 = 0xEF;
pub const RID_VENDOR_F1: u8 = 0xF1;
pub const RID_CMD: u8 = 0xF2;

/// 开麦 / 关麦。决定开关的是第二个字节；单字节 [0x01] 无效。
/// hidapi 把 buffer[0] 当 report ID，所以是 [0xF2, opcode, arg]。
pub const MIC_ON: [u8; 3] = [RID_CMD, 0x01, 0x01];
pub const MIC_OFF: [u8; 3] = [RID_CMD, 0x01, 0x00];

use crate::keys::{Key, PAGE_CONSUMER, PAGE_KEYBOARD, PAGE_VENDOR};

/// 从键盘报告里解出当前按下的键（3 字节 usage 数组）
pub fn parse_keyboard(payload: &[u8]) -> Vec<Key> {
    payload
        .iter()
        .filter(|&&u| u != 0)
        .map(|&u| Key::new(PAGE_KEYBOARD, u as u16))
        .collect()
}

/// 从 Consumer 报告里解出当前按下的键（2 个小端 u16）
pub fn parse_consumer(payload: &[u8]) -> Vec<Key> {
    payload
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .filter(|&u| u != 0)
        .map(|u| Key::new(PAGE_CONSUMER, u))
        .collect()
}

/// 从 vendor report 0xEF 解出按下的 App 快捷键。
/// 实测：按下 `A1 00 00`（A1..A4 对应 prime/NETFLIX/Disney+/hulu），松开 `00 00 00`。
pub fn parse_vendor(payload: &[u8]) -> Vec<Key> {
    payload
        .iter()
        .filter(|&&u| u != 0)
        .map(|&u| Key::new(PAGE_VENDOR, u as u16))
        .collect()
}
