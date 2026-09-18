//! 菜单栏状态项（右上角常驻小图标）+ 窗口拖拽辅助。

#[cfg(target_os = "macos")]
use objc2::runtime::AnyObject;
#[cfg(target_os = "macos")]
use objc2::{define_class, MainThreadOnly};
#[cfg(target_os = "macos")]
use objc2_foundation::{NSObject, NSObjectProtocol};

// 菜单项的 target 类。用**自定义 selector**（fvShow: / fvQuit:）而不是系统标准的
// unhide: / terminate: —— 因为 macOS 26 会给标准动作在菜单里自动加个图标（那个 ⊠），
// 还占着左侧图标列的缩进。自定义 selector AppKit 不认得，就是纯文本、无图标、无缩进。
#[cfg(target_os = "macos")]
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FVTrayTarget"]
    struct TrayTarget;

    impl TrayTarget {
        #[unsafe(method(fvShow:))]
        fn fv_show(&self, _sender: Option<&AnyObject>) {
            use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
            let mtm = self.mtm();
            let app = NSApplication::sharedApplication(mtm);
            // 从托盘恢复窗口时重新成为普通应用，Dock 图标也跟着回来。
            app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
            app.unhide(None);
            app.activate();
        }

        #[unsafe(method(fvQuit:))]
        fn fv_quit(&self, _sender: Option<&AnyObject>) {
            use objc2_app_kit::NSApplication;
            let mtm = self.mtm();
            let app = NSApplication::sharedApplication(mtm);
            // terminate: 触发 applicationWillTerminate → gpui 跑 on_app_quit（清 hidremap）
            unsafe { app.terminate(None) };
        }

        // 菜单里点某个方案 → 切过去。菜单项的 tag 存的是方案下标。
        // 只改 active + bump 代数；刷新 tray/界面/硬件层映射交给 UI 的 pump（它比对代数）。
        #[unsafe(method(fvSwitchProfile:))]
        fn fv_switch_profile(&self, sender: Option<&AnyObject>) {
            use objc2::msg_send;
            let Some(sender) = sender else { return };
            let tag: isize = unsafe { msg_send![sender, tag] };
            if tag < 0 {
                return;
            }
            if let Some(cfg) = CFG.get() {
                let changed = {
                    let mut c = cfg.write();
                    let i = tag as usize;
                    if i < c.profiles.len() && i != c.active {
                        c.active = i;
                        let _ = c.save();
                        true
                    } else {
                        false
                    }
                };
                if changed {
                    crate::config::bump_profile_gen();
                }
            }
        }
    }

    unsafe impl NSObjectProtocol for TrayTarget {}
);

// 状态项、target 的裸指针（对象都 forget 泄漏、永久存活，主线程上重建引用）。
// 菜单要随方案变化重建，所以得留着句柄；Retained 不是 Sync，存指针最省事。
#[cfg(target_os = "macos")]
static ITEM_PTR: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg(target_os = "macos")]
static TARGET_PTR: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// 图标图片指针 —— 重建菜单时 setImage 用（每次 set_profiles 都要重设图标+标题）
#[cfg(target_os = "macos")]
static IMG_PTR: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// 共享配置：菜单读方案名/active、点击时改 active。由 UI 首帧 `attach_cfg` 注入。
#[cfg(target_os = "macos")]
static CFG: std::sync::OnceLock<std::sync::Arc<parking_lot::RwLock<crate::config::Config>>> =
    std::sync::OnceLock::new();
/// 「显示窗口 / 退出」两个固定菜单项的文案
#[cfg(target_os = "macos")]
static LABELS: std::sync::OnceLock<(String, String, String)> = std::sync::OnceLock::new();

/// 关掉 macOS 的窗口恢复。**必须在 `main()` 最开头调**，见下面为什么。
///
/// ⚠️ 崩溃（或被强杀）过一次之后，macOS 下一次启动会弹「意外退出，要恢复窗口吗」
/// 的**模态框**（`NSPersistentUIRestorer promptToIgnorePersistentStateWithCrashHistory:`
/// → `NSAlert runModal`），主线程就停在 `runModal` 上 —— 进程活着、状态栏图标也在，
/// 但什么都不干：连不上遥控器、按键毫无反应。这和「HID 读线程失效」长得一模一样，
/// 2026-09-16 为此往 HID 层白查了很久。判据：`sample <pid>` 看主线程有没有 `runModal`。
///
/// ⚠️ **Info.plist 的 `NSQuitAlwaysKeepsWindows=false` 挡不住它**（实测）——
/// 那个键管的是「正常退出时保不保存窗口」，这个框走崩溃历史那条路。
/// 只有 `ApplePersistenceIgnoreState` 管用，而装机时没人会手动 `defaults write`，
/// 所以 app 自己写进自家 domain。
///
/// ⚠️ **调用时机是这条的关键**：那个框是 AppKit 处理 open event 时弹的，也就是
/// `NSApplication run` 之后。放在 `tray::install()`（run 闭包末尾）里等于**永远跑不到**
/// —— 弹框卡住主线程 → 代码没机会执行 → 下次启动照样弹，死循环。踩过。
/// 放在 `main()` 第一行就赶在读取它之前，**当次启动即生效**。
#[cfg(target_os = "macos")]
pub fn disable_window_restore() {
    unsafe {
        use objc2_foundation::{ns_string, NSUserDefaults};
        let d = NSUserDefaults::standardUserDefaults();
        if !d.boolForKey(ns_string!("ApplePersistenceIgnoreState")) {
            d.setBool_forKey(true, ns_string!("ApplePersistenceIgnoreState"));
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn disable_window_restore() {}

/// 装菜单栏状态项。⚠️ 必须在**主线程、NSApp 起来之后**调（gpui 的 run 闭包里）。
#[cfg(target_os = "macos")]
pub fn install(icon_png: &[u8], show_label: &str, quit_label: &str, profiles_label: &str) {
    use objc2::rc::Retained;
    use objc2::{msg_send, AnyThread, MainThreadMarker};
    use objc2_app_kit::{NSImage, NSStatusBar, NSVariableStatusItemLength};
    use objc2_foundation::{NSData, NSSize};

    let Some(mtm) = MainThreadMarker::new() else {
        eprintln!("[tray] 不在主线程，跳过");
        return;
    };
    let _ = LABELS.set((
        show_label.to_string(),
        quit_label.to_string(),
        profiles_label.to_string(),
    ));

    let bar = NSStatusBar::systemStatusBar();
    let item = unsafe { bar.statusItemWithLength(NSVariableStatusItemLength) };
    if let Some(button) = item.button(mtm) {
        // 自绘的遥控器剪影模板图（内嵌 PNG，见 ui/assets/tray）。setTemplate(true)
        // 让 macOS 按菜单栏深浅色自动着色。18pt 高、宽按原图比例（遥控器高瘦形）。
        let data = NSData::with_bytes(icon_png);
        if let Some(img) = NSImage::initWithData(NSImage::alloc(), &data) {
            img.setTemplate(true);
            let sz = img.size();
            let w = if sz.height > 0.0 { 18.0 * sz.width / sz.height } else { 18.0 };
            img.setSize(NSSize::new(w, 18.0));
            button.setImage(Some(&img));
            IMG_PTR.store(Retained::as_ptr(&img) as usize, std::sync::atomic::Ordering::Relaxed);
            std::mem::forget(img);
        }
    }

    // 自定义 target（forget 保活 + 存指针）
    let target: Retained<TrayTarget> = {
        let this = mtm.alloc::<TrayTarget>();
        unsafe { msg_send![this, init] }
    };
    TARGET_PTR.store(Retained::as_ptr(&target) as usize, std::sync::atomic::Ordering::Relaxed);
    ITEM_PTR.store(Retained::as_ptr(&item) as usize, std::sync::atomic::Ordering::Relaxed);
    std::mem::forget(item);
    std::mem::forget(target);

    // 先建一版菜单（此时还没 cfg，就只有「显示/退出」）；attach_cfg 后会重建带方案列表
    rebuild_menu();
    eprintln!("[tray] 状态栏图标已装");
}

/// UI 首帧把共享配置交给 tray（菜单要读方案名、点击要改 active），随后刷新一次。
#[cfg(target_os = "macos")]
pub fn attach_cfg(cfg: std::sync::Arc<parking_lot::RwLock<crate::config::Config>>) {
    let _ = CFG.set(cfg);
    set_profiles();
}

/// 刷新 tray：图标右侧的方案名 + 菜单里的方案列表/勾选。方案变了就调（主线程）。
#[cfg(target_os = "macos")]
pub fn set_profiles() {
    rebuild_menu();
}

#[cfg(target_os = "macos")]
fn rebuild_menu() {
    use objc2::{sel, MainThreadMarker};
    use objc2_app_kit::{NSImage, NSControlStateValueOn, NSMenu, NSMenuItem, NSStatusItem};
    use objc2_foundation::NSString;

    let Some(mtm) = MainThreadMarker::new() else { return };
    let ip = ITEM_PTR.load(std::sync::atomic::Ordering::Relaxed);
    let tp = TARGET_PTR.load(std::sync::atomic::Ordering::Relaxed);
    if ip == 0 || tp == 0 {
        return;
    }
    // 这些对象已 forget、永久存活，主线程上重建引用是安全的
    let item: &NSStatusItem = unsafe { &*(ip as *const NSStatusItem) };
    let target: &TrayTarget = unsafe { &*(tp as *const TrayTarget) };

    // 读方案信息（没 cfg 时给空）
    let (names, active, show_name) = match CFG.get() {
        Some(cfg) => {
            let c = cfg.read();
            (c.profile_names(), c.active, c.has_profile_switch())
        }
        None => (Vec::new(), 0, false),
    };
    let (show_label, quit_label, profiles_label) = LABELS
        .get()
        .cloned()
        .unwrap_or_else(|| ("显示窗口".into(), "退出".into(), "方案".into()));

    // 图标右侧的方案名：只有配了「切换方案」动作才显示（需求如此）
    if let Some(button) = item.button(mtm) {
        // 重设图标（setTitle 有时会顶掉图标，稳妥起见一起重设）
        let imgp = IMG_PTR.load(std::sync::atomic::Ordering::Relaxed);
        if imgp != 0 {
            let img: &NSImage = unsafe { &*(imgp as *const NSImage) };
            button.setImage(Some(img));
        }
        let title = if show_name {
            names.get(active).cloned().unwrap_or_default()
        } else {
            String::new()
        };
        // 前面留个空格，和图标之间有点间距
        let t = if title.is_empty() { String::new() } else { format!(" {title}") };
        button.setTitle(&NSString::from_str(&t));
    }

    // 重建菜单。顺序：显示窗口 → (分隔 + 「方案」标题 + 方案列表) → 分隔 + 退出。
    let menu = NSMenu::new(mtm);
    let empty = NSString::from_str("");

    // 一个可点的动作项（显示/退出用）
    let mk = |title: &str, action| {
        let it = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(title),
                Some(action),
                &empty,
            )
        };
        unsafe { it.setTarget(Some(target)) };
        menu.addItem(&it);
    };

    // 1) 第一组：打开窗口
    mk(&show_label, sel!(fvShow:));

    // 2) 第二组：方案（>1 才有意义）。分隔线 + 一行禁用的「方案」标题（灰字、点不动，
    //    纯粹告诉用户下面这几行是干嘛的）+ 各方案（勾选当前、点击切换）。
    if names.len() > 1 {
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        let hdr = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(&profiles_label),
                None,
                &empty,
            )
        };
        hdr.setEnabled(false); // 禁用 = 灰字标题，不可选
        menu.addItem(&hdr);
        for (i, n) in names.iter().enumerate() {
            let it = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &NSString::from_str(n),
                    Some(sel!(fvSwitchProfile:)),
                    &empty,
                )
            };
            unsafe {
                it.setTarget(Some(target));
                it.setTag(i as isize);
            }
            if i == active {
                it.setState(NSControlStateValueOn); // 勾
            }
            menu.addItem(&it);
        }
    }

    // 3) 末组：退出
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    mk(&quit_label, sel!(fvQuit:));

    // setMenu 会 retain 新菜单、release 旧的；本地 Retained 随即 drop 不泄漏
    item.setMenu(Some(&menu));
}

#[cfg(not(target_os = "macos"))]
pub fn install(_icon_png: &[u8], _show_label: &str, _quit_label: &str, _profiles_label: &str) {}
#[cfg(not(target_os = "macos"))]
pub fn attach_cfg(_cfg: std::sync::Arc<parking_lot::RwLock<crate::config::Config>>) {}
#[cfg(not(target_os = "macos"))]
pub fn set_profiles() {}

/// （已废弃）曾用 movableByWindowBackground 让整窗可拖，但 gpui 的输入框不是独立
/// NSView，整个内容被当背景 —— 在输入框里拖拽会变成拖窗、没法选文本。改用 header
/// 上的 `start_window_drag()`（performWindowDragWithEvent）精准拖。留空壳兼容旧调用。
pub fn make_windows_draggable() {}

/// 窗口是不是缩到状态栏了。界面那边靠它决定「还要不要重绘」——
/// 这个 app 的常态就是缩在状态栏跑一整天，那期间画一帧都是浪费。
static HIDDEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn is_hidden() -> bool {
    HIDDEN.load(std::sync::atomic::Ordering::Relaxed)
}

fn set_hidden(v: bool) {
    HIDDEN.store(v, std::sync::atomic::Ordering::Relaxed);
}

/// 主窗口关闭后进入纯菜单栏模式：进程和状态栏图标继续存在，但不占 Dock。
/// 从托盘选择“显示窗口”时，`fv_show` 会把 activation policy 恢复为 Regular。
#[cfg(target_os = "macos")]
pub fn hide_to_tray() {
    set_hidden(true);
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

    let Some(mtm) = MainThreadMarker::new() else { return };
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    app.hide(None);
}

#[cfg(not(target_os = "macos"))]
pub fn hide_to_tray() {
    set_hidden(true);
}

/// 通过 Finder/Dock 的 reopen 事件恢复时，也确保重新显示 Dock 图标。
#[cfg(target_os = "macos")]
pub fn show_from_tray() {
    set_hidden(false);
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

    let Some(mtm) = MainThreadMarker::new() else { return };
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    app.unhide(None);
    app.activate();
}

#[cfg(not(target_os = "macos"))]
pub fn show_from_tray() {
    set_hidden(false);
}

/// 从当前鼠标事件开始拖窗 —— 挂在 header 的 on_mouse_down 上。用 AppKit 原生的
/// performWindowDragWithEvent，只有 header 触发，输入框等交互元素不受影响。
#[cfg(target_os = "macos")]
pub fn start_window_drag() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    let Some(mtm) = MainThreadMarker::new() else { return };
    let app = NSApplication::sharedApplication(mtm);
    if let (Some(ev), Some(win)) = (app.currentEvent(), app.keyWindow()) {
        win.performWindowDragWithEvent(&ev);
    }
}

#[cfg(not(target_os = "macos"))]
pub fn start_window_drag() {}
