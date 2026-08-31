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

/// 开麦 / 关麦。hidapi 把 buffer[0] 当 report id。
///
/// ★ 实测定死（`firectl --mic-off-test`，授权环境、控制变量、可复现）：
///   开麦 `[F2,01,01]`(3B) → 起流 ~50 帧/秒（第 3 字节 01=开）
///   关麦 **`[F2,00]`(2B)** → 停流。⚠️ **`[F2,01,00]`(3B) 停不了**（写成功但流照走 61/秒）——
///   费电真凶就是它：v0.1.0 一直用 [F2,01,00] 当关麦，从来没真关上过。
///   关麦不是「把开麦第 3 字节改 0」，而是另一条 2 字节命令。
///
/// ⚠️ 能不能发出去取决于**进程有没有「输入监控」授权**（跟长度无关）：
///   没授权 → 任何写都 `0xE00002E2 not permitted`。FireVibe.app 有自己的授权；
///   firectl 靠 disclaim 自持授权（见 main.rs）。别把权限错误当成字节错误
///   —— 我在这上面栽过一整轮。
pub const MIC_ON: [u8; 3] = [RID_CMD, 0x01, 0x01];
pub const MIC_OFF: [u8; 2] = [RID_CMD, 0x00];

use crate::keys::{Key, PAGE_CONSUMER, PAGE_KEYBOARD, PAGE_VENDOR};

/// 一台 HID 设备的身份信息，给「换一款遥控器」的选择列表用
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HidDev {
    pub vid: u16,
    pub pid: u16,
    pub product: String,
    pub vendor: String,
}

impl HidDev {
    /// 界面上显示的名字，产品名拿不到就退回标识
    pub fn label(&self) -> String {
        if self.product.trim().is_empty() {
            format!("0x{:04x}:0x{:04x}", self.vid, self.pid)
        } else {
            self.product.clone()
        }
    }
    pub fn ids(&self) -> String {
        format!("0x{:04x} / 0x{:04x}", self.vid, self.pid)
    }
}

/// 列出系统里所有 HID 设备（按 VID/PID 去重）。
///
/// ⚠️ **必须在后台线程调用** —— hidapi 枚举会跑 run loop，在 gpui 的
/// `cx.new` / `update` 里调会撞上 `RefCell already borrowed` 直接 abort。
pub fn list_hid() -> Vec<HidDev> {
    let Ok(api) = hidapi::HidApi::new() else {
        return Vec::new();
    };
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<HidDev> = api
        .device_list()
        .filter(|d| seen.insert((d.vendor_id(), d.product_id())))
        .map(|d| HidDev {
            vid: d.vendor_id(),
            pid: d.product_id(),
            product: d.product_string().unwrap_or("").trim().to_string(),
            vendor: d.manufacturer_string().unwrap_or("").trim().to_string(),
        })
        .collect();
    // 有名字的排前面，方便肉眼找遥控器
    out.sort_by(|a, b| {
        (a.product.is_empty(), a.label().to_lowercase())
            .cmp(&(b.product.is_empty(), b.label().to_lowercase()))
    });
    out
}

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

// ───────────────────────── 「输入监控」授权自检 ─────────────────────────

#[cfg(target_os = "macos")]
#[link(name = "IOKit", kind = "framework")]
extern "C" {
    /// `IOHIDCheckAccess(kIOHIDRequestTypeListenEvent)`
    /// 返回 0=已授权 1=被拒 2=还没问过
    fn IOHIDCheckAccess(request_type: u32) -> u32;
}

/// 「输入监控」到底给没给。
///
/// 加它是因为这个权限的失效方式特别隐蔽：**枚举设备照常、打开设备也成功，
/// 只是一条输入报告都收不到**。表现成「设备连上了但按键没反应」，
/// 极容易误判成设备坏了 —— 实际白查了大半天。
/// 而系统设置里的开关**看着是开的**也不代表真生效（见 CLAUDE.md 的签名坑）。
///
/// 判据要用系统自己的答案，别靠猜。
pub fn input_monitoring() -> &'static str {
    #[cfg(target_os = "macos")]
    unsafe {
        // kIOHIDRequestTypeListenEvent = 1
        match IOHIDCheckAccess(1) {
            0 => "已授权",
            1 => "被拒绝",
            _ => "还没问过",
        }
    }
    #[cfg(not(target_os = "macos"))]
    "不适用"
}
