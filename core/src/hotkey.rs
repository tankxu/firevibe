//! 录快捷键。**走 CGEventTap，不走窗口的键盘事件。**
//!
//! 为什么：如果要录的组合键已经被别的软件注册成全局热键，那个软件会在事件
//! 到达我们窗口之前把它吃掉 —— 录制框永远等不到。而会话层的 tap
//! （`kCGSessionEventTap` + HeadInsert）比应用级热键更早拿到事件。
//!
//! 录制期间还会**把这次按键吞掉**，否则录「已被占用的组合」时会顺带把那个
//! 软件也唤起来。只吞硬件事件（pid==0），并且 10 秒自动结束 ——
//! 万一界面卡住，键盘不会一直失灵。

use crate::tap::{self, Ev, Tap, EV_FLAGS_CHANGED, EV_KEY_DOWN};
use anyhow::Result;
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// 录到的一组按键
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grabbed {
    /// 主键名（injector 那套名字，比如 "space" / "rightoption"）
    pub key: String,
    /// 修饰键名
    pub mods: Vec<String>,
}

/// 录制会话。drop 即停止（tap 随之关掉，按键恢复正常）。
pub struct Capture {
    _tap: Tap,
    slot: Arc<Mutex<Option<Grabbed>>>,
    started: Instant,
}

impl Capture {
    /// 取走录到的结果（只会成功一次）
    pub fn take(&self) -> Option<Grabbed> {
        self.slot.lock().take()
    }
    /// 超时了吗。界面据此自动结束录制，避免键盘一直被吞
    pub fn timed_out(&self) -> bool {
        self.started.elapsed() > Duration::from_secs(10)
    }
}

/// keycode → 修饰键名。用来支持「只按一个修饰键」当热键。
fn modifier_of(code: i64) -> Option<&'static str> {
    Some(match code {
        0x37 => "leftcmd",
        0x36 => "rightcmd",
        0x38 => "leftshift",
        0x3c => "rightshift",
        0x3a => "leftoption",
        0x3d => "rightoption",
        0x3b => "leftcontrol",
        0x3e => "rightcontrol",
        0x3f => "fn",
        _ => return None,
    })
}

const FLAG_SHIFT: u64 = 0x0002_0000;
const FLAG_CTRL: u64 = 0x0004_0000;
const FLAG_ALT: u64 = 0x0008_0000;
const FLAG_CMD: u64 = 0x0010_0000;
const FLAG_FN: u64 = 0x0080_0000;
/// 只看这几位，别把 NonCoalesced(0x100) 之类当成修饰键
const FLAG_ALL: u64 = FLAG_SHIFT | FLAG_CTRL | FLAG_ALT | FLAG_CMD | FLAG_FN;

fn mods_of(flags: u64) -> Vec<String> {
    let mut v = Vec::new();
    if flags & FLAG_CMD != 0 {
        v.push("cmd".into());
    }
    if flags & FLAG_SHIFT != 0 {
        v.push("shift".into());
    }
    if flags & FLAG_ALT != 0 {
        v.push("alt".into());
    }
    if flags & FLAG_CTRL != 0 {
        v.push("ctrl".into());
    }
    v
}

/// 录制状态机。**抽出来是为了能单测** —— 真实录制要靠硬件事件，
/// 合成事件会被 `is_hardware` 挡掉，没法自动化验证。
///
/// 判定规则（组合键之所以要按 key-up 判定）：
/// - 普通键**按下**时落定：主键 = 该键，修饰键 = 当时按着的那些。
///   这一步必须在按下时做，因为松开时 flags 已经清了。
/// - 只按修饰键时**不能一按下就落定** —— 按 ⌘⇧A 的过程里 ⌘ 会先按下，
///   那样会被误判成「单修饰键热键」。所以要等**修饰键全部松开**、
///   且期间没按过普通键，才把「按住过的最后一个修饰键」当作热键。
#[derive(Default, Debug)]
pub struct Machine {
    /// 期间按下过的最后一个修饰键（keycode）
    last_mod: Option<i64>,
    /// 期间有没有按过普通键 —— 有就说明是组合键，别再走单修饰键那条
    saw_key: bool,
}

/// 一个事件喂进状态机后的结果
#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    /// 还没录完，继续等。bool = 要不要吞掉这个事件
    Wait(bool),
    /// 录完了
    Done(Grabbed),
    /// 用户按了 Esc
    Cancel,
}

impl Machine {
    /// `kind` 用 tap 的事件类型常量
    pub fn feed(&mut self, kind: u32, code: i64, flags: u64) -> Step {
        if kind == EV_FLAGS_CHANGED {
            let held = flags & FLAG_ALL;
            if let Some(_name) = modifier_of(code) {
                if held != 0 {
                    // 还有修饰键按着 → 记下它，继续等
                    self.last_mod = Some(code);
                    return Step::Wait(true);
                }
                // 全松开了
                let last = self.last_mod.take();
                if self.saw_key {
                    // 是组合键的收尾，普通键那步已经落定过了
                    self.saw_key = false;
                    return Step::Wait(false);
                }
                if let Some(name) = last.and_then(modifier_of) {
                    // 只按了修饰键 → 它本身就是热键
                    return Step::Done(Grabbed {
                        key: name.into(),
                        mods: Vec::new(),
                    });
                }
            }
            return Step::Wait(false);
        }
        // 普通键按下
        let Some(name) = crate::inject::name_of_code(code as u16) else {
            return Step::Wait(false); // 不认识的键不吞，免得白吃用户的按键
        };
        if name == "escape" {
            return Step::Cancel;
        }
        self.saw_key = true;
        Step::Done(Grabbed {
            key: name.into(),
            mods: mods_of(flags),
        })
    }
}

/// 开始录制。返回的 Capture 一 drop 就停。
pub fn start() -> Result<Capture> {
    let slot: Arc<Mutex<Option<Grabbed>>> = Arc::new(Mutex::new(None));
    let s2 = slot.clone();
    let mach = Arc::new(Mutex::new(Machine::default()));
    let t = tap::spawn(
        &[EV_KEY_DOWN, EV_FLAGS_CHANGED],
        false, // 要拦：别让被占用的组合顺带唤起那个软件
        Box::new(move |ev: Ev| {
            // 只认真实硬件。我们自己（和别的 app）注入的按键不录也不吞
            if !tap::is_hardware(ev) {
                return false;
            }
            // 已经录到了就别再吞，让键盘立刻恢复
            if s2.lock().is_some() {
                return false;
            }
            match mach.lock().feed(ev.kind, ev.code, ev.flags) {
                Step::Wait(swallow) => swallow,
                Step::Cancel => {
                    *s2.lock() = Some(Grabbed {
                        key: "escape".into(),
                        mods: Vec::new(),
                    });
                    true
                }
                Step::Done(g) => {
                    *s2.lock() = Some(g);
                    true
                }
            }
        }),
        None,
    )?;
    Ok(Capture {
        _tap: t,
        slot,
        started: Instant::now(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tap::EV_KEY_DOWN;

    const CMD: i64 = 0x37;
    const SHIFT: i64 = 0x38;
    const R_OPT: i64 = 0x3d;
    const A: i64 = 0x00;
    const ESC: i64 = 0x35;

    #[test]
    fn modifier_table_covers_both_sides() {
        for (code, want) in [
            (0x37, "leftcmd"),
            (0x36, "rightcmd"),
            (0x3a, "leftoption"),
            (0x3d, "rightoption"),
            (0x3b, "leftcontrol"),
            (0x3e, "rightcontrol"),
        ] {
            assert_eq!(modifier_of(code), Some(want), "keycode 0x{code:x}");
        }
        assert_eq!(modifier_of(0x00), None, "普通键不该被当成修饰键");
    }

    #[test]
    fn flags_decode_to_mod_names() {
        assert_eq!(mods_of(FLAG_CMD | FLAG_SHIFT), vec!["cmd", "shift"]);
        assert!(mods_of(0).is_empty());
        // 非修饰位不该被算进去
        assert!(mods_of(0x100).is_empty(), "NonCoalesced 不是修饰键");
    }

    /// ⌘⇧A：这是之前坏掉的场景 —— ⌘ 一按下就被当成单修饰键热键录完了
    #[test]
    fn combo_waits_for_the_real_key() {
        let mut m = Machine::default();
        assert_eq!(m.feed(EV_FLAGS_CHANGED, CMD, FLAG_CMD), Step::Wait(true));
        assert_eq!(
            m.feed(EV_FLAGS_CHANGED, SHIFT, FLAG_CMD | FLAG_SHIFT),
            Step::Wait(true)
        );
        assert_eq!(
            m.feed(EV_KEY_DOWN, A, FLAG_CMD | FLAG_SHIFT),
            Step::Done(Grabbed {
                key: "a".into(),
                mods: vec!["cmd".into(), "shift".into()],
            })
        );
    }

    /// 只按右 ⌥ 再松开 → 它自己当热键（要等松开才落定）
    #[test]
    fn modifier_only_settles_on_release() {
        let mut m = Machine::default();
        assert_eq!(m.feed(EV_FLAGS_CHANGED, R_OPT, FLAG_ALT), Step::Wait(true));
        assert_eq!(
            m.feed(EV_FLAGS_CHANGED, R_OPT, 0),
            Step::Done(Grabbed {
                key: "rightoption".into(),
                mods: Vec::new(),
            })
        );
    }

    /// 组合键松开修饰键时，不能再落定成单修饰键
    #[test]
    fn combo_release_does_not_settle_again() {
        let mut m = Machine::default();
        m.feed(EV_FLAGS_CHANGED, CMD, FLAG_CMD);
        m.feed(EV_KEY_DOWN, A, FLAG_CMD);
        assert_eq!(m.feed(EV_FLAGS_CHANGED, CMD, 0), Step::Wait(false));
    }

    #[test]
    fn escape_cancels() {
        let mut m = Machine::default();
        assert_eq!(m.feed(EV_KEY_DOWN, ESC, 0), Step::Cancel);
    }

    #[test]
    fn unknown_key_is_not_swallowed() {
        let mut m = Machine::default();
        // 0x0a 在 VK 表里没有 —— 不认识就别吞用户的按键
        assert_eq!(m.feed(EV_KEY_DOWN, 0x0a, 0), Step::Wait(false));
    }
}
