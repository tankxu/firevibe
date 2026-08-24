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
            use objc2_app_kit::NSApplication;
            let mtm = self.mtm();
            let app = NSApplication::sharedApplication(mtm);
            unsafe { app.unhide(None) };
            app.activateIgnoringOtherApps(true);
        }

        #[unsafe(method(fvQuit:))]
        fn fv_quit(&self, _sender: Option<&AnyObject>) {
            use objc2_app_kit::NSApplication;
            let mtm = self.mtm();
            let app = NSApplication::sharedApplication(mtm);
            // terminate: 触发 applicationWillTerminate → gpui 跑 on_app_quit（清 hidremap）
            unsafe { app.terminate(None) };
        }
    }

    unsafe impl NSObjectProtocol for TrayTarget {}
);

/// 装菜单栏状态项。⚠️ 必须在**主线程、NSApp 起来之后**调（gpui 的 run 闭包里）。
#[cfg(target_os = "macos")]
pub fn install(icon_png: &[u8], show_label: &str, quit_label: &str) {
    use objc2::rc::Retained;
    use objc2::{msg_send, sel, AnyThread, MainThreadMarker};
    use objc2_app_kit::{NSImage, NSMenu, NSMenuItem, NSStatusBar, NSVariableStatusItemLength};
    use objc2_foundation::{NSData, NSSize, NSString};

    let Some(mtm) = MainThreadMarker::new() else {
        eprintln!("[tray] 不在主线程，跳过");
        return;
    };

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
        } else {
            button.setTitle(&NSString::from_str("FireVibe"));
        }
    }

    // 自定义 target（末尾 forget 保活）
    let target: Retained<TrayTarget> = {
        let this = mtm.alloc::<TrayTarget>();
        unsafe { msg_send![this, init] }
    };

    let menu = NSMenu::new(mtm);
    let empty = NSString::from_str("");
    let mut mk = |title: &str, action| {
        let it = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(title),
                Some(action),
                &empty,
            )
        };
        unsafe { it.setTarget(Some(&target)) };
        menu.addItem(&it);
    };
    mk(show_label, sel!(fvShow:));
    mk(quit_label, sel!(fvQuit:));
    drop(mk);
    item.setMenu(Some(&menu));

    // 永久保活：进程活多久它们活多久
    std::mem::forget(item);
    std::mem::forget(menu);
    std::mem::forget(target);
    eprintln!("[tray] 状态栏图标已装");
}

#[cfg(not(target_os = "macos"))]
pub fn install(_icon_png: &[u8], _show_label: &str, _quit_label: &str) {}

/// （已废弃）曾用 movableByWindowBackground 让整窗可拖，但 gpui 的输入框不是独立
/// NSView，整个内容被当背景 —— 在输入框里拖拽会变成拖窗、没法选文本。改用 header
/// 上的 `start_window_drag()`（performWindowDragWithEvent）精准拖。留空壳兼容旧调用。
pub fn make_windows_draggable() {}

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
