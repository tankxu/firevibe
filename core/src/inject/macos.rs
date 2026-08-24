//! macOS: CGEvent 合成按键。需要「辅助功能」(Accessibility) 权限。
//!
//! 音量 / 静音 / 播放 / 上下曲这些**不是普通按键**，是 NSEvent 的
//! `systemDefined` 事件（subtype 8），CGEvent 造不出来 —— 只能借 AppKit 拼一个
//! 再取它的 CGEvent 发出去。好处是能拿到系统原生的音量 HUD。

use super::Injector;
use anyhow::{anyhow, Result};
use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType};
use objc2_core_graphics::{CGEvent as CGEv, CGEventTapLocation as TapLoc};
use objc2_foundation::NSPoint;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn CGEventSourceFlagsState(state: u32) -> u64;
}
/// 合成事件发不出去时先看这个：残留的修饰键位会让文本事件被当快捷键丢掉
/// （`FIREVIBE_TYPE_DEBUG=1` 打开）
fn hw_flags() -> u64 {
    unsafe { CGEventSourceFlagsState(0) } // 0 = HIDSystemState
}

/// NX_KEYTYPE_* —— IOKit 里的媒体键编号
const NX: &[(&str, i64)] = &[
    ("volume_up", 0),
    ("volume_down", 1),
    ("mute", 7),
    ("play", 16),
    // 快进/快退映到「下一曲/上一曲」而不是 NX_KEYTYPE_FAST/REWIND(19/20)：
    // 后者几乎没有 app 认，前者浏览器和播放器普遍支持
    ("next", 17),
    ("prev", 18),
];

fn nx_of(name: &str) -> Option<i64> {
    let l = name.to_ascii_lowercase();
    NX.iter().find(|(n, _)| *n == l).map(|(_, c)| *c)
}

/// 发一次媒体键（按下 + 松开）
fn post_media(nx: i64) -> Result<()> {
    for down in [true, false] {
        // 高位 0xA = 按下，0xB = 松开；data1 低 16 位重复放这个状态字
        let state: u64 = if down { 0xA00 } else { 0xB00 };
        let data1 = (nx << 16) | state as i64;
        let ev = NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(
            NSEventType::SystemDefined,
            NSPoint::ZERO,
            NSEventModifierFlags(state as usize),
            0.0,
            0,
            None,
            8,
            data1 as isize,
            -1,
        )
        .ok_or_else(|| anyhow!("建 NSEvent 失败"))?;
        let cg = ev.CGEvent().ok_or_else(|| anyhow!("取 CGEvent 失败"))?;
        CGEv::post(TapLoc::HIDEventTap, Some(&cg));
    }
    Ok(())
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
}

/// macOS 虚拟键码（kVK_*），跟 HID usage 不是一回事
const VK: &[(&str, u16)] = &[
    ("return", 0x24),
    ("enter", 0x24),
    ("tab", 0x30),
    ("space", 0x31),
    ("delete", 0x33),
    ("backspace", 0x33),
    ("escape", 0x35),
    ("esc", 0x35),
    ("left", 0x7B),
    ("right", 0x7C),
    ("down", 0x7D),
    ("up", 0x7E),
    ("home", 0x73),
    ("pageup", 0x74),
    ("forwarddelete", 0x75),
    ("end", 0x77),
    ("pagedown", 0x79),
    ("a", 0x00),
    ("s", 0x01),
    ("d", 0x02),
    ("f", 0x03),
    ("h", 0x04),
    ("g", 0x05),
    ("z", 0x06),
    ("x", 0x07),
    ("c", 0x08),
    ("v", 0x09),
    ("b", 0x0B),
    ("q", 0x0C),
    ("w", 0x0D),
    ("e", 0x0E),
    ("r", 0x0F),
    ("y", 0x10),
    ("t", 0x11),
    ("o", 0x1F),
    ("u", 0x20),
    ("i", 0x22),
    ("p", 0x23),
    ("l", 0x25),
    ("j", 0x26),
    ("k", 0x28),
    ("n", 0x2D),
    ("m", 0x2E),
    ("1", 0x12),
    ("2", 0x13),
    ("3", 0x14),
    ("4", 0x15),
    ("5", 0x17),
    ("6", 0x16),
    ("7", 0x1A),
    ("8", 0x1C),
    ("9", 0x19),
    ("0", 0x1D),
    ("minus", 0x1B),
    ("equal", 0x18),
    ("leftbracket", 0x21),
    ("rightbracket", 0x1E),
    ("semicolon", 0x29),
    ("quote", 0x27),
    ("comma", 0x2B),
    ("period", 0x2F),
    ("slash", 0x2C),
    ("backslash", 0x2A),
    ("grave", 0x32),
    // 标点的字面量别名。热键录制拿到的是键面上印的字符（"]" 而不是 "rightbracket"），
    // 两种写法都收，配置文件里存哪种都能用。
    ("-", 0x1B),
    ("=", 0x18),
    ("[", 0x21),
    ("]", 0x1E),
    (";", 0x29),
    ("'", 0x27),
    (",", 0x2B),
    (".", 0x2F),
    ("/", 0x2C),
    ("\\", 0x2A),
    ("`", 0x32),
    ("f1", 0x7A),
    ("f2", 0x78),
    ("f3", 0x63),
    ("f4", 0x76),
    ("f5", 0x60),
    ("f6", 0x61),
    ("f7", 0x62),
    ("f8", 0x64),
    ("f9", 0x65),
    ("f10", 0x6D),
    ("f11", 0x67),
    ("f12", 0x6F),
    // F13~F15：没有默认绑定，很适合当外部 app 的热键，按住也不会打出字符
    ("f13", 0x69),
    ("f14", 0x6B),
    ("f15", 0x71),
    // 修饰键本体。外部语音 app（闪电说这类）常把「按着说」绑在这上面，
    // 得能单独发。注意 fn 是硬件级的，合成基本无效 —— 这也是为什么
    // 界面上建议你去那个 app 里改成普通组合键。
    ("cmd", 0x37),
    ("leftcmd", 0x37),
    ("rightcmd", 0x36),
    ("shift", 0x38),
    ("leftshift", 0x38),
    ("rightshift", 0x3C),
    ("alt", 0x3A),
    ("leftoption", 0x3A),
    ("rightoption", 0x3D),
    ("ctrl", 0x3B),
    ("leftcontrol", 0x3B),
    ("rightcontrol", 0x3E),
    ("fn", 0x3F),
];

pub fn key_names() -> Vec<&'static str> {
    VK.iter()
        .map(|(n, _)| *n)
        .chain(NX.iter().map(|(n, _)| *n))
        .collect()
}

fn code_of(name: &str) -> Option<u16> {
    let l = name.to_ascii_lowercase();
    VK.iter().find(|(n, _)| *n == l).map(|(_, c)| *c)
}

/// keycode → 键名。录快捷键时用（tap 给的是 keycode）。
/// 表里同一个 code 有多个别名（return/enter），取第一个 —— 那是首选写法。
pub fn name_of_code(code: u16) -> Option<&'static str> {
    VK.iter().find(|(_, c)| *c == code).map(|(n, _)| *n)
}

fn flags_of(mods: &[String]) -> CGEventFlags {
    let mut f = CGEventFlags::CGEventFlagNull;
    for m in mods {
        match m.to_ascii_lowercase().as_str() {
            "shift" => f |= CGEventFlags::CGEventFlagShift,
            "ctrl" | "control" => f |= CGEventFlags::CGEventFlagControl,
            "alt" | "option" | "opt" => f |= CGEventFlags::CGEventFlagAlternate,
            "cmd" | "command" | "meta" | "super" => f |= CGEventFlags::CGEventFlagCommand,
            _ => {}
        }
    }
    f
}

/// 发一个按键事件。
///
/// **修饰键（右 Command / 右 Option 这类）必须发 `flagsChanged` 类型**，
/// 普通 keyDown/keyUp 系统不当它是修饰键 —— 实测 `CGEventSourceFlagsState`
/// 完全不变，接收方也看不到。发 flagsChanged 才认。
/// 真实 HID 事件都带的 NonCoalesced 位。实测真键盘按右 Option 是 0x80140，
/// 我们原来只发 0x80000 —— 少了它和下面的设备位。
const FLAG_NON_COALESCED: u64 = 0x0000_0100;

/// IOKit 的左右设备位。真键盘的 flagsChanged 会带上「是左边还是右边那颗」，
/// 只发通用的 Alternate/Command 位，靠这个区分左右的监听方（豆包这类语音工具）
/// 就认不出来。实测右 Option 是 0x40。
fn device_bit_of(key: &str) -> Option<u64> {
    Some(match key.to_ascii_lowercase().as_str() {
        "leftcontrol" | "ctrl" | "control" => 0x0001,
        "leftshift" | "shift" => 0x0002,
        "rightshift" => 0x0004,
        "leftcmd" | "cmd" | "command" => 0x0008,
        "rightcmd" => 0x0010,
        "leftoption" | "alt" | "option" => 0x0020,
        "rightoption" => 0x0040,
        "rightcontrol" => 0x2000,
        _ => return None,
    })
}

fn emit(key: &str, mods: &[String], down: bool) -> Result<()> {
    let code = code_of(key).ok_or_else(|| anyhow!("不认识的按键名 {key:?}"))?;
    let self_flag = mod_flag_of(key);
    let mut bits = flags_of(mods).bits() | FLAG_NON_COALESCED;
    if down {
        if let Some(f) = self_flag {
            bits |= f.bits();
        }
        if let Some(b) = device_bit_of(key) {
            bits |= b;
        }
    }
    // ⚠️ 用 Private 源，别用 HIDSystemState —— 后者会往事件里塞一个真键盘
    // 根本没有的 0x20000000 位（实测），而且 set_flags 清不掉它。
    let src = CGEventSource::new(CGEventSourceStateID::Private)
        .map_err(|_| anyhow!("建 CGEventSource 失败"))?;
    let ev =
        CGEvent::new_keyboard_event(src, code, down).map_err(|_| anyhow!("建 CGEvent 失败"))?;
    if self_flag.is_some() {
        // 主键本身就是修饰键 → 改成 flagsChanged
        ev.set_type(core_graphics::event::CGEventType::FlagsChanged);
    }
    ev.set_flags(CGEventFlags::from_bits_retain(bits));
    ev.post(CGEventTapLocation::HID);
    Ok(())
}

/// app 最常用的那条路：`NSEvent.modifierFlags`（类属性 = 当前状态）。
/// 排障用 —— 判断合成的修饰键有没有真的让系统认为「按住了」。
pub fn ns_modifier_alt() -> bool {
    NSEvent::modifierFlags_class().contains(NSEventModifierFlags::Option)
}

/// 这个键名本身是不是修饰键；是的话返回它对应的 flag
fn mod_flag_of(key: &str) -> Option<CGEventFlags> {
    Some(match key.to_ascii_lowercase().as_str() {
        "cmd" | "command" | "leftcmd" | "rightcmd" => CGEventFlags::CGEventFlagCommand,
        "shift" | "leftshift" | "rightshift" => CGEventFlags::CGEventFlagShift,
        "alt" | "option" | "leftoption" | "rightoption" => CGEventFlags::CGEventFlagAlternate,
        "ctrl" | "control" | "leftcontrol" | "rightcontrol" => CGEventFlags::CGEventFlagControl,
        "fn" | "function" => CGEventFlags::CGEventFlagSecondaryFn,
        _ => return None,
    })
}

pub struct MacInjector;

pub fn new_injector() -> Box<dyn Injector> {
    Box::new(MacInjector)
}

impl Injector for MacInjector {
    fn available(&self) -> bool {
        unsafe { AXIsProcessTrusted() }
    }

    fn why(&self) -> String {
        if self.available() {
            "辅助功能权限已授予".into()
        } else {
            "缺少「辅助功能」(Accessibility) 权限。\n\
             系统设置 > 隐私与安全性 > 辅助功能 > 勾上本应用，然后完全退出再打开。"
                .into()
        }
    }

    fn key_stroke(&self, key: &str, mods: &[String]) -> Result<()> {
        // 媒体键走另一条路，且不吃修饰键
        if let Some(nx) = nx_of(key) {
            return post_media(nx);
        }
        emit(key, mods, true)?;
        emit(key, mods, false)
    }

    fn key_down(&self, key: &str, mods: &[String]) -> Result<()> {
        if let Some(nx) = nx_of(key) {
            return post_media(nx); // 媒体键没有「按住」语义
        }
        emit(key, mods, true)
    }

    fn key_up(&self, key: &str, mods: &[String]) -> Result<()> {
        if nx_of(key).is_some() {
            return Ok(());
        }
        emit(key, mods, false)
    }

    fn type_text(&self, s: &str) -> Result<()> {
        if s.is_empty() {
            return Ok(());
        }
        // ⚠️ 两处坑：
        // 1) 用 HIDSystemState 建源，事件会**继承当前硬件修饰键状态**。
        //    只要有修饰位在（哪怕是别人没松开的残留），带 flag 的文本事件
        //    会被目标 app 当成快捷键丢掉 —— 表现是 post 返回成功但一个字都不出。
        //    所以改用 Private 源，并且显式把 flags 清零。
        // 2) `CGEventKeyboardSetUnicodeString` 长串会被截断，按 ≤16 个
        //    UTF-16 单元切开发。
        if std::env::var_os("FIREVIBE_TYPE_DEBUG").is_some() {
            eprintln!(
                "[type] {} UTF-16 单元，发之前硬件修饰位 = 0x{:x}",
                s.encode_utf16().count(),
                hw_flags()
            );
        }
        let units: Vec<u16> = s.encode_utf16().collect();
        let mut i = 0usize;
        while i < units.len() {
            // 别把代理对切断
            let mut end = (i + 16).min(units.len());
            if end < units.len() && (0xD800..0xDC00).contains(&units[end - 1]) {
                end -= 1;
            }
            let chunk = String::from_utf16_lossy(&units[i..end]);
            for down in [true, false] {
                let src = CGEventSource::new(CGEventSourceStateID::Private)
                    .map_err(|_| anyhow!("建 CGEventSource 失败"))?;
                let ev = CGEvent::new_keyboard_event(src, 0, down)
                    .map_err(|_| anyhow!("建 CGEvent 失败"))?;
                ev.set_flags(CGEventFlags::CGEventFlagNull);
                ev.set_string(&chunk);
                ev.post(CGEventTapLocation::HID);
            }
            i = end;
            std::thread::sleep(std::time::Duration::from_millis(4));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn CGEventSourceKeyState(state: u32, keycode: u16) -> bool;
        fn CGEventSourceFlagsState(state: u32) -> u64;
    }
    const FLAG_ALTERNATE: u64 = 0x0008_0000;

    /// 修饰键单独当热键（闪电说的「自由说」= 右 Command 就是这种）。
    ///
    /// 记录一个查清楚的边界：**合成事件改不了系统的全局修饰位**。
    /// 无论发普通 keyDown 还是 flagsChanged，`CGEventSourceFlagsState`
    /// 都不变 —— 那反映的是硬件状态。但**事件本身是投递出去的**，
    /// 用 NSEvent 全局监听 / CGEventTap 的 app 能收到；靠轮询修饰位的收不到。
    /// 所以能不能驱动某个具体 app，只能实测，这里只保证调用链不报错。
    /// 手动跑：`cargo test -p firevibe-core -- --ignored --nocapture modifier_only`
    #[test]
    #[ignore = "会真的发右 option 事件，需要辅助功能权限"]
    fn modifier_only_hotkey_posts_without_error() {
        let inj = MacInjector;
        inj.key_down("rightoption", &[]).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(120));
        inj.key_up("rightoption", &[]).unwrap();
        let flags = unsafe { CGEventSourceFlagsState(COMBINED) } & FLAG_ALTERNATE;
        println!("发完之后全局 option 位 = 0x{flags:x}（预期 0：合成事件不改硬件状态）");
        assert_eq!(flags, 0, "不该残留按下状态");
    }
    /// kCGEventSourceStateCombinedSessionState
    const COMBINED: u32 = 0;

    /// 「按住」模式的核心语义：key_down 之后这个键要一直是按下的，
    /// key_up 之后才松。外部语音 app 的「按着说」全靠这个。
    ///
    /// 用 F13 测：普通键、没有默认绑定，按住也不会打出字符。
    /// **修饰键（Fn / 右 Command）测不通也改不了** —— macOS 里合成修饰键码
    /// 不会更新全局修饰位，这是系统行为。所以界面上明确建议去外部 app 里
    /// 把快捷键改成普通组合键。
    ///
    /// 手动跑：`cargo test -p firevibe-core -- --ignored --nocapture hold_keeps`
    #[test]
    #[ignore = "会真的按住一个键，且需要辅助功能权限"]
    fn hold_keeps_key_down() {
        const VK_F13: u16 = 0x69;
        let inj = MacInjector;
        let down_now = || unsafe { CGEventSourceKeyState(COMBINED, VK_F13) };
        assert!(!down_now(), "开跑前 F13 就是按下的？");

        inj.key_down("f13", &[]).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(250));
        let held = down_now();

        // 断言成败都要松开，别把键卡在按下状态
        inj.key_up("f13", &[]).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(250));
        let released = down_now();

        println!("按下后 F13 = {held}，松开后 = {released}");
        assert!(held, "key_down 之后键没按住 —— 合成没生效或缺辅助功能权限");
        assert!(!released, "key_up 之后键没松开，卡住了");
    }

    /// 手动跑：`cargo test -p firevibe-core -- --ignored --nocapture media_key`
    /// 需要跑测试的那个进程有「辅助功能」权限，否则事件会被系统静默丢掉。
    #[test]
    #[ignore = "会真的动系统音量，且需要辅助功能权限"]
    fn media_key_moves_system_volume() {
        fn vol() -> i32 {
            let out = std::process::Command::new("osascript")
                .args(["-e", "output volume of (get volume settings)"])
                .output()
                .expect("osascript");
            String::from_utf8_lossy(&out.stdout)
                .trim()
                .parse()
                .unwrap_or(-1)
        }
        let before = vol();
        post_media(nx_of("volume_down").unwrap()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(400));
        let after = vol();
        // 发完再调回去，别把用户音量留在别处
        post_media(nx_of("volume_up").unwrap()).unwrap();
        println!("音量 {before} -> {after}");
        assert!(
            after < before,
            "音量没动（{before} -> {after}）—— 大概是跑测试的进程没有辅助功能权限"
        );
    }
}
