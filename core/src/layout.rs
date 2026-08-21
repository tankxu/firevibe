//! 遥控器物理按键布局 —— Amazon Alexa Voice Remote 3rd Gen（Fire TV）
//!
//! 布局依据实物照片，自上而下：
//!   电源(左上，单独一行) / 麦克风(蓝色 Alexa 环，居中)
//!   D-pad 环 + 中心 OK
//!   返回 · 主页 · 菜单
//!   快退 · 播放暂停 · 快进
//!   静音 · 音量摇杆(+/−，一个竖长胶囊) · TV
//!   Prime Video · Netflix
//!   Disney+ · Hulu
//! 共 21 个按键。
//!
//! 交叉验证：从 GATT 的 KeyMetric 特征 5DE24A19 读出的按键计数表是
//! key id 0x00–0x14 共 21 个（其中 0x02 从未按过）—— 21 个 ID 对 21 个实体键，吻合。
//!
//! 图上「位置」(Slot) 与实际 HID usage 的对应是**可学习**的：只有方向键/OK/麦克风
//! 是实机确认过的，其余按 HID 规范推测，点图上按钮再按实体键即可校正。

use crate::keys::{Key, PAGE_CONSUMER, PAGE_KEYBOARD, PAGE_VENDOR};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Slot {
    #[default]
    Power,
    Mic,
    Up,
    Left,
    Ok,
    Right,
    Down,
    Back,
    Home,
    Menu,
    Rewind,
    Play,
    Forward,
    Mute,
    VolUp,
    VolDown,
    Tv,
    App1,
    App2,
    App3,
    App4,
}

/// 按钮形状
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// 正圆
    Circle,
    /// 圆角矩形（D-pad 方向瓣、音量摇杆两半、App 键）
    Rounded,
}

impl Slot {
    pub const ALL: [Slot; 21] = [
        Slot::Power,
        Slot::Mic,
        Slot::Up,
        Slot::Left,
        Slot::Ok,
        Slot::Right,
        Slot::Down,
        Slot::Back,
        Slot::Home,
        Slot::Menu,
        Slot::Rewind,
        Slot::Play,
        Slot::Forward,
        Slot::Mute,
        Slot::VolUp,
        Slot::VolDown,
        Slot::Tv,
        Slot::App1,
        Slot::App2,
        Slot::App3,
        Slot::App4,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Slot::Power => "power",
            Slot::Mic => "mic",
            Slot::Up => "up",
            Slot::Left => "left",
            Slot::Ok => "ok",
            Slot::Right => "right",
            Slot::Down => "down",
            Slot::Back => "back",
            Slot::Home => "home",
            Slot::Menu => "menu",
            Slot::Rewind => "rewind",
            Slot::Play => "play",
            Slot::Forward => "forward",
            Slot::Mute => "mute",
            Slot::VolUp => "vol_up",
            Slot::VolDown => "vol_down",
            Slot::Tv => "tv",
            Slot::App1 => "app1",
            Slot::App2 => "app2",
            Slot::App3 => "app3",
            Slot::App4 => "app4",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Slot::Power => "电源",
            Slot::Mic => "麦克风",
            Slot::Up => "上",
            Slot::Left => "左",
            Slot::Ok => "OK",
            Slot::Right => "右",
            Slot::Down => "下",
            Slot::Back => "返回",
            Slot::Home => "主页",
            Slot::Menu => "菜单",
            Slot::Rewind => "快退",
            Slot::Play => "播放/暂停",
            Slot::Forward => "快进",
            Slot::Mute => "静音",
            Slot::VolUp => "音量+",
            Slot::VolDown => "音量−",
            Slot::Tv => "TV",
            Slot::App1 => "Prime Video",
            Slot::App2 => "Netflix",
            Slot::App3 => "Disney+",
            Slot::App4 => "Hulu",
        }
    }

    /// 图上显示的字形。避开 emoji —— 免得触发字体回退，字重会不一致。
    pub fn glyph(self) -> &'static str {
        match self {
            Slot::Power => "⏻",
            Slot::Mic => "◉",
            Slot::Up => "▲",
            Slot::Left => "◀",
            Slot::Ok => "OK",
            Slot::Right => "▶",
            Slot::Down => "▼",
            Slot::Back => "↩",
            Slot::Home => "⌂",
            Slot::Menu => "≡",
            Slot::Rewind => "◀◀",
            Slot::Play => "▶‖",
            Slot::Forward => "▶▶",
            Slot::Mute => "静音",
            Slot::VolUp => "＋",
            Slot::VolDown => "－",
            Slot::Tv => "TV",
            Slot::App1 => "prime",
            Slot::App2 => "NETFLIX",
            Slot::App3 => "Disney+",
            Slot::App4 => "hulu",
        }
    }

    /// 各 App 键的品牌色（画上去更像实物）
    pub fn tint(self) -> Option<u32> {
        Some(match self {
            Slot::Mic => 0x2b7fd4,     // Alexa 蓝
            Slot::App1 => 0x1a5fa8,    // prime video 蓝
            Slot::App2 => 0xd7202a,    // Netflix 红
            Slot::App3 => 0x1a2b6d,    // Disney+ 深蓝
            Slot::App4 => 0x1ce783,    // hulu 绿
            _ => return None,
        })
    }

    /// 默认猜测的 HID usage。方向键 / OK / 麦克风为实机确认，其余为推测。
    /// HID usage —— **全部实机测绘所得**（firevibe-cli --map，2026-08）。
    /// 注意几个反直觉的：
    ///   OK   = 0x58 Keypad Enter（不是 0x28 Return）
    ///   返回 = 0x00F1 键盘页非标准值（Amazon 私有，不是 Esc）
    ///   TV   = 0x008D Program Guide（不是 0x0089 Media Select TV）
    /// 四个 App 快捷键走 vendor report 0xEF（Usage Page 0xFF00），按下发 A1~A4。
    pub fn default_key(self) -> Option<Key> {
        Some(match self {
            Slot::Power => Key::new(PAGE_KEYBOARD, 0x0066),
            Slot::Mic => Key::new(PAGE_CONSUMER, 0x0221),
            Slot::Up => Key::new(PAGE_KEYBOARD, 0x0052),
            Slot::Left => Key::new(PAGE_KEYBOARD, 0x0050),
            Slot::Ok => Key::new(PAGE_KEYBOARD, 0x0058),
            Slot::Right => Key::new(PAGE_KEYBOARD, 0x004F),
            Slot::Down => Key::new(PAGE_KEYBOARD, 0x0051),
            Slot::Back => Key::new(PAGE_KEYBOARD, 0x00F1),
            Slot::Home => Key::new(PAGE_CONSUMER, 0x0223),
            Slot::Menu => Key::new(PAGE_CONSUMER, 0x0040),
            Slot::Rewind => Key::new(PAGE_CONSUMER, 0x00B4),
            Slot::Play => Key::new(PAGE_CONSUMER, 0x00CD),
            Slot::Forward => Key::new(PAGE_CONSUMER, 0x00B3),
            Slot::Mute => Key::new(PAGE_CONSUMER, 0x00E2),
            Slot::VolUp => Key::new(PAGE_CONSUMER, 0x00E9),
            Slot::VolDown => Key::new(PAGE_CONSUMER, 0x00EA),
            Slot::Tv => Key::new(PAGE_CONSUMER, 0x008D),
            // 四个 App 键走 vendor report 0xEF（页 0xFF00），实测 A1~A4
            Slot::App1 => Key::new(PAGE_VENDOR, 0xA1),
            Slot::App2 => Key::new(PAGE_VENDOR, 0xA2),
            Slot::App3 => Key::new(PAGE_VENDOR, 0xA3),
            Slot::App4 => Key::new(PAGE_VENDOR, 0xA4),
        })
    }

    /// usage 是否实机确认过
    /// 21 个键全部实机测绘确认，不再需要「学习」功能
    pub fn confirmed(self) -> bool {
        true
    }

    /// 设计稿坐标：(left, top, w, h)，单位是设计画布像素（146.2 × 484.5）。
    /// 取自定稿 `design/mockup.html`，一个数一个数量出来的，别手改 —— 改了就和设计稿脱钩。
    fn design_rect(self) -> (f32, f32, f32, f32) {
        match self {
            Slot::Power => (16.1, 20.4, 25.5, 25.5),
            Slot::Mic => (55.2, 39.9, 35.7, 35.7),
            // D-pad：方向瓣落在内外圈之间，OK 在正中
            Slot::Up => (58.6, 93.1, 28.9, 22.1),
            Slot::Left => (14.0, 135.2, 22.1, 34.0),
            Slot::Ok => (39.9, 119.0, 66.3, 66.3),
            Slot::Right => (110.1, 135.2, 22.1, 34.0),
            Slot::Down => (58.6, 189.1, 28.9, 22.1),
            // 返回 / 主页 / 菜单
            Slot::Back => (17.8, 226.9, 25.5, 25.5),
            Slot::Home => (60.4, 226.9, 25.5, 25.5),
            Slot::Menu => (102.8, 226.9, 25.5, 25.5),
            // 快退 / 播放暂停 / 快进
            Slot::Rewind => (17.8, 267.8, 25.5, 25.5),
            Slot::Play => (60.4, 267.8, 25.5, 25.5),
            Slot::Forward => (102.8, 267.8, 25.5, 25.5),
            // 静音 / 音量摇杆两半 / TV
            Slot::Mute => (17.8, 309.4, 25.5, 25.5),
            Slot::VolUp => (60.4, 307.7, 25.5, 28.4),
            Slot::VolDown => (60.4, 336.6, 25.5, 28.9),
            Slot::Tv => (102.8, 309.4, 25.5, 25.5),
            // App 快捷键
            Slot::App1 => (11.0, 377.4, 59.5, 26.3),
            Slot::App2 => (75.6, 377.4, 59.5, 26.3),
            Slot::App3 => (11.0, 411.4, 59.5, 26.3),
            Slot::App4 => (75.6, 411.4, 59.5, 26.3),
        }
    }

    /// 换算成实际画布像素：(left, top, w, h)。等比缩放，圆的永远是圆的。
    pub fn rect(self, cw: f32, _ch: f32) -> (f32, f32, f32, f32) {
        let s = cw / DESIGN_W;
        let (l, t, w, h) = self.design_rect();
        (l * s, t * s, w * s, h * s)
    }

    /// 圆角半径（设计稿像素）。圆形返回半径 = 边长一半。
    pub fn design_radius(self) -> f32 {
        match self.shape() {
            Shape::Circle => self.design_rect().2 / 2.,
            Shape::Rounded => 9.0,
        }
    }

    pub fn shape(self) -> Shape {
        match self {
            Slot::Up
            | Slot::Down
            | Slot::Left
            | Slot::Right
            | Slot::VolUp
            | Slot::VolDown
            | Slot::App1
            | Slot::App2
            | Slot::App3
            | Slot::App4 => Shape::Rounded,
            _ => Shape::Circle,
        }
    }
}

/// 设计画布尺寸，与 `design/mockup.html` 里渲染出来的遥控器一致。
pub const DESIGN_W: f32 = 146.2;
pub const DESIGN_H: f32 = 484.5;
/// 机身宽高比
pub const ASPECT: f32 = DESIGN_W / DESIGN_H;

/// 机身轮廓 SVG 的 viewBox 宽度。轮廓路径用 viewBox 坐标写，
/// 画的时候乘 `DESIGN_W / BODY_VIEW_W` 落到设计画布。
pub const BODY_VIEW_W: f32 = 316.0;
pub const BODY_VIEW_H: f32 = 1047.0;
/// 上下圆弧的高度（viewBox 单位），r=60、控制点系数 0.55/0.74
pub const BODY_R: f32 = 60.0;
pub const BODY_C1: f32 = 0.55;
pub const BODY_C2: f32 = 0.74;

/// 装饰件，设计画布像素 (left, top, w, h)
/// 顶部麦克风缝
pub const MIC_SLIT: (f32, f32, f32, f32) = (71.6, 9.3, 3.0, 7.6);
/// D-pad 凹环
pub const DPAD_WELL: (f32, f32, f32, f32) = (10.2, 89.2, 125.8, 125.8);
/// 音量摇杆胶囊（整体，含中缝）
pub const VOL_CAPSULE: (f32, f32, f32, f32) = (60.4, 307.7, 25.5, 57.8);
pub const VOL_CAPSULE_R: f32 = 12.8;

/// 把设计画布坐标换算到实际像素
pub fn scaled(r: (f32, f32, f32, f32), cw: f32) -> (f32, f32, f32, f32) {
    let s = cw / DESIGN_W;
    (r.0 * s, r.1 * s, r.2 * s, r.3 * s)
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct SlotBinding {
    pub slot: Slot,
    pub key: Key,
}

pub fn default_slots() -> Vec<SlotBinding> {
    Slot::ALL
        .iter()
        .filter_map(|&s| s.default_key().map(|k| SlotBinding { slot: s, key: k }))
        .collect()
}
