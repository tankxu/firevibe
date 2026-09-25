//! CGEventTap：看系统把遥控器的键翻译成了什么事件，以及**把它吞掉**。
//!
//! 为什么需要：我们打不开独占 HID（独占要 root），所以系统和我们同时收到
//! 遥控器的按键。麦克风键的 usage 是 Consumer `0x0221`（AC Search），
//! macOS 自己就把它当「搜索键」→ 弹 Spotlight。要压住只能在事件层拦。
//!
//! 需要「辅助功能」权限。tap 跑在自己的线程 + 自己的 CFRunLoop 上，
//! 不碰 gpui 的主 run loop。
//!
//! ⚠️ 隐私：诊断模式**只打印非字符键**（功能/媒体键区、修饰键、systemDefined），
//! 普通打字一律不记录，也不打印 key_char。

#![cfg(target_os = "macos")]

use anyhow::{anyhow, Result};
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

type CFMachPortRef = *mut c_void;
type CFRunLoopSourceRef = *mut c_void;
type CFRunLoopRef = *mut c_void;
type CFStringRef = *const c_void;
type CGEventRef = *mut c_void;
type CGEventTapProxy = *mut c_void;

type TapCallback = extern "C" fn(CGEventTapProxy, u32, CGEventRef, *mut c_void) -> CGEventRef;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: u64,
        callback: TapCallback,
        user_info: *mut c_void,
    ) -> CFMachPortRef;
    fn CGEventTapEnable(port: CFMachPortRef, enable: bool);
    fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
    fn CGEventGetFlags(event: CGEventRef) -> u64;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFMachPortCreateRunLoopSource(
        alloc: *const c_void,
        port: CFMachPortRef,
        order: isize,
    ) -> CFRunLoopSourceRef;
    fn CFRunLoopGetCurrent() -> CFRunLoopRef;
    fn CFRunLoopAddSource(rl: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
    fn CFRunLoopRun();
    fn CFRunLoopStop(rl: CFRunLoopRef);
    fn CFRunLoopRemoveSource(rl: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
    fn CFMachPortInvalidate(port: CFMachPortRef);
    fn CFRelease(cf: *const c_void);
    static kCFRunLoopCommonModes: CFStringRef;
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
}

// kCGSessionEventTap = 1；插在队首才能吞掉
const TAP_SESSION: u32 = 1;
const PLACE_HEAD: u32 = 0;
const OPT_DEFAULT: u32 = 0;
const OPT_LISTEN_ONLY: u32 = 1;

pub const EV_KEY_DOWN: u32 = 10;
pub const EV_KEY_UP: u32 = 11;
pub const EV_FLAGS_CHANGED: u32 = 12;
pub const EV_SYSTEM_DEFINED: u32 = 14;

/// kCGKeyboardEventKeycode
const FIELD_KEYCODE: u32 = 9;
/// kCGEventSourceUnixProcessID —— 事件是哪个进程发的
const FIELD_SOURCE_PID: u32 = 41;
/// kCGKeyboardEventKeyboardType —— 不同物理键盘的类型 id 不一样，
/// 将来想把屏蔽精确到「只屏蔽遥控器、不影响自带键盘」就靠它
const FIELD_KEYBOARD_TYPE: u32 = 40;

/// macOS 已知会自己处理的遥控器按键，对应的事件键码。
///
/// 这些是**系统常量**，不是每个用户不一样的东西，所以内置 —— 不用谁都自己学一遍。
/// `0xb1`：麦克风键的 HID usage 是 Consumer `0x0221` (AC Search)，
/// macOS 把它翻成这个键码并弹 Spotlight。实测学到的就是它。
///
/// 注意：Mac 自带键盘上的搜索键也是这个码，所以屏蔽只在**遥控器连着时**生效，
/// 不然会顺手把你键盘的搜索键也吞了。
pub const BUILTIN_SUPPRESS: &[i64] = &[0xb1];

fn mask(types: &[u32]) -> u64 {
    types.iter().fold(0u64, |m, &t| m | (1u64 << t))
}

/// 一个事件的关键信息
#[derive(Clone, Copy, Debug)]
pub struct Ev {
    pub kind: u32,
    /// 物理键盘类型 id，用来区分遥控器和自带键盘
    pub kb_type: i64,
    /// 发出这个事件的进程 pid（0 = 真实硬件）。
    /// 用来把**我们自己注入的按键**排除掉 —— 否则会学进屏蔽表、开始吞自己发的键。
    pub pid: i64,
    /// 键盘事件的虚拟键码；systemDefined 时是 NX 键码（从 data1 高 16 位取）
    pub code: i64,
    pub flags: u64,
    /// systemDefined 时：true = 按下
    pub nx_down: bool,
}

/// 判定要不要吞掉这个事件
type Decide = Box<dyn Fn(Ev) -> bool + Send + Sync>;

struct Ctx {
    decide: Decide,
    log: Option<Box<dyn Fn(Ev) + Send + Sync>>,
    /// 自己那个 mach port，回调里被系统禁用后要拿它重新 enable
    port: std::sync::atomic::AtomicUsize,
}

extern "C" fn on_event(
    _proxy: CGEventTapProxy,
    kind: u32,
    event: CGEventRef,
    user: *mut c_void,
) -> CGEventRef {
    // SAFETY: user 是 spawn 时泄漏的 Ctx，生命周期与进程同长
    let ctx = unsafe { &*(user as *const Ctx) };
    // 系统把 tap 禁用了（0xFFFF_FFFE 超时 / 0xFFFF_FFFF 被用户输入打断）。
    // ⚠️ **必须自己重新 enable** —— 以前这里只写「调用方应重新 enable」，
    // 而根本没有调用方会做，于是一旦超时，按键屏蔽就**静默永久失效**
    //（麦克风键重新开始弹 Spotlight，还查不出为什么）。
    // ⚠️ 但**没授权时绝不重新 enable**：一个没人服务得了的 tap 挂在链头，
    // 会把整条会话事件队列卡住（见 stop() 上面那段）。
    if kind == 0xFFFF_FFFE || kind == 0xFFFF_FFFF {
        let port = ctx.port.load(Ordering::Relaxed);
        let trusted = unsafe { AXIsProcessTrusted() };
        eprintln!("[tap] 被系统禁用（kind={kind:#x}，授权={trusted}）—— {}",
                  if trusted { "重新启用" } else { "没授权，保持关闭" });
        if trusted && port != 0 {
            unsafe { CGEventTapEnable(port as CFMachPortRef, true) };
        }
        return event;
    }
    let flags = unsafe { CGEventGetFlags(event) };
    let raw = unsafe { CGEventGetIntegerValueField(event, FIELD_KEYCODE) };
    let (code, nx_down) = if kind == EV_SYSTEM_DEFINED {
        // systemDefined 的 NX 键码在 data1 高 16 位；CGEvent 没有 data1 字段，
        // 但 keycode 字段对这类事件恰好放的是同一份整数
        ((raw >> 16) & 0xffff, ((raw >> 8) & 0xff) == 0x0a)
    } else {
        (raw, kind == EV_KEY_DOWN)
    };
    let pid = unsafe { CGEventGetIntegerValueField(event, FIELD_SOURCE_PID) };
    let kb_type = unsafe { CGEventGetIntegerValueField(event, FIELD_KEYBOARD_TYPE) };
    let ev = Ev {
        kind,
        code,
        flags,
        nx_down,
        pid,
        kb_type,
    };
    if let Some(l) = &ctx.log {
        l(ev);
    }
    if (ctx.decide)(ev) {
        std::ptr::null_mut() // 吞掉
    } else {
        event
    }
}

pub struct Tap {
    stop: Arc<AtomicBool>,
    rl: Arc<parking_lot::Mutex<Option<usize>>>,
    /// mach port。停的时候要**先**在这儿把 tap 关掉，见 stop()
    port: Arc<std::sync::atomic::AtomicUsize>,
}

impl Tap {
    /// ⚠️⚠️ **停 tap 不能只 `CFRunLoopStop`。**
    ///
    /// CGEventTap 是**进程外系统状态**：mach port 注册在 WindowServer 的会话 tap 链
    /// **队首**（我们要 PLACE_HEAD 才能吞事件）。run loop 一停就没人再服务这个 port，
    /// 而它还在链上 —— WindowServer 等不到回复，**整条会话事件队列卡住**：
    /// 点击和键盘全被吞，只有光标还能动（光标是 WindowServer 自己画的）。
    /// 表现得像触控板/键盘坏了。用户真撞上过（撤销授权那次）。
    ///
    /// 所以顺序是：**先 `CGEventTapEnable(false)` 让它立刻退出事件链**（任何线程都能调，
    /// 不用等 run loop 醒），再停 run loop，剩下的 remove source / invalidate / release
    /// 在 tap 线程上 `CFRunLoopRun()` 返回之后做。
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        let p = self.port.swap(0, Ordering::Relaxed);
        if p != 0 {
            unsafe { CGEventTapEnable(p as CFMachPortRef, false) };
        }
        // ⚠️ `take()` 而不是读一眼：tap 线程退出后这个 CFRunLoopRef 就失效了，
        // 而 `Drop` 还会再调一次 stop() —— 留着就是对已释放的 run loop 再来一发。
        if let Some(p) = self.rl.lock().take() {
            unsafe { CFRunLoopStop(p as CFRunLoopRef) };
        }
    }
}

impl Drop for Tap {
    fn drop(&mut self) {
        self.stop();
    }
}

/// 起一个 tap。`listen_only=true` 时只看不拦（诊断用）。
pub fn spawn(
    types: &[u32],
    listen_only: bool,
    decide: Decide,
    log: Option<Box<dyn Fn(Ev) + Send + Sync>>,
) -> Result<Tap> {
    let m = mask(types);
    let stop = Arc::new(AtomicBool::new(false));
    let rl = Arc::new(parking_lot::Mutex::new(None));
    let port_out = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
    let rl2 = rl.clone();
    let port_out2 = port_out.clone();
    std::thread::spawn(move || {
        // Ctx 故意泄漏：回调的生命周期跟着 run loop，进程退出才结束
        let ctx = Box::leak(Box::new(Ctx {
            decide,
            log,
            port: std::sync::atomic::AtomicUsize::new(0),
        }));
        let port = unsafe {
            CGEventTapCreate(
                TAP_SESSION,
                PLACE_HEAD,
                if listen_only {
                    OPT_LISTEN_ONLY
                } else {
                    OPT_DEFAULT
                },
                m,
                on_event,
                ctx as *mut Ctx as *mut c_void,
            )
        };
        if port.is_null() {
            let _ = tx.send(Err("EVENT_TAP_FAILED".into()));
            return;
        }
        ctx.port.store(port as usize, Ordering::Relaxed);
        port_out2.store(port as usize, Ordering::Relaxed);
        let (src, cur) = unsafe {
            let src = CFMachPortCreateRunLoopSource(std::ptr::null(), port, 0);
            let cur = CFRunLoopGetCurrent();
            *rl2.lock() = Some(cur as usize);
            CFRunLoopAddSource(cur, src, kCFRunLoopCommonModes);
            CGEventTapEnable(port, true);
            (src, cur)
        };
        let _ = tx.send(Ok(()));
        unsafe { CFRunLoopRun() };
        // run loop 退出 = 有人调了 stop()。**必须在这儿把 tap 彻底拆出事件链** ——
        // 只停 run loop 的话 port 还挂在 WindowServer 的链头没人服务，整机输入会卡死。
        // （stop() 已经先 disable 过一次，这里是把 port 真正作废并释放。）
        unsafe {
            CGEventTapEnable(port, false);
            CFRunLoopRemoveSource(cur, src, kCFRunLoopCommonModes);
            CFMachPortInvalidate(port);
            CFRelease(src);
            CFRelease(port);
        }
        ctx.port.store(0, Ordering::Relaxed);
        eprintln!("[tap] 已拆除（run loop 退出、mach port 已作废）");
    });
    match rx.recv_timeout(std::time::Duration::from_secs(3)) {
        Ok(Ok(())) => Ok(Tap {
            stop,
            rl,
            port: port_out,
        }),
        Ok(Err(e)) => Err(anyhow!(e)),
        Err(_) => Err(anyhow!("event tap 启动超时")),
    }
}

/// 真实硬件发的事件吗（`pid == 0`）。
/// 学习和屏蔽都只认硬件事件 —— 否则会把我们自己注入的按键学进屏蔽表，
/// 然后开始吞自己发的键。
pub fn is_hardware(ev: Ev) -> bool {
    ev.pid == 0
}

/// 这个键码是「非字符键」吗 —— 诊断只打印这些，不碰你打的字
pub fn is_non_character(ev: Ev) -> bool {
    if ev.kind == EV_SYSTEM_DEFINED || ev.kind == EV_FLAGS_CHANGED {
        return true;
    }
    // 功能键 / 方向键 / 媒体键都在 0x60 以上；字母数字标点在 0x00~0x50
    ev.code >= 0x60
}
