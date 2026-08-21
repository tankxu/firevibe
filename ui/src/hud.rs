//! 屏幕底部中间的悬浮电平条，跟语音输入法那种一样。
//!
//! 独立的置顶无边框窗口，**`focus: false` 是关键** —— 抢了焦点的话
//! 识别出的文字就打不进你原来的输入框了。

use crate::theme::*;
use crate::widget::*;
use firevibe_core::runtime::Runtime;
use gpui::{
    div, prelude::*, px, size, App, Bounds, Context, Entity, SharedString, Window, WindowBounds,
    WindowHandle, WindowKind, WindowOptions,
};
use std::sync::Arc;
use std::time::Duration;

// 窗口尺寸必须**正好等于药丸**。留了空白边的话，macOS 的窗口阴影按窗口矩形画，
// 透明边会把桌面透出来、再被阴影勾一圈边，看着就是药丸外面套了个白框。
const W: f32 = 300.;
const H: f32 = 44.;
/// 离屏幕底边多高。要够高 —— 输入法自己的候选/语音浮层就在屏幕底部，
/// 盖住了就没法看它的状态。
const BOTTOM_GAP: f32 = 240.;

pub struct Hud {
    rt: Arc<Runtime>,
}

impl Hud {
    fn new(rt: Arc<Runtime>, cx: &mut Context<Self>) -> Self {
        // 自己驱动重绘，电平才跟手。必须先 await 再 update。
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_millis(16)).await;
            if this.update(cx, |_, cx| cx.notify()).is_err() {
                break;
            }
        })
        .detach();
        Self { rt }
    }
}

impl Render for Hud {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let lvl = self.rt.level();
        let dictating = self.rt.dictating.lock().is_some();
        let bars = 22usize;
        let lit = (lvl * 60.0).min(bars as f32) as usize;
        let mut meter = div().flex().gap(px(3.)).items_center().h(px(20.));
        for i in 0..bars {
            // 中间高两头低，像个纺锤，静音时是一条细线
            let base = 3. + (1. - ((i as f32 / (bars - 1) as f32) - 0.5).abs() * 2.) * 5.;
            let h = if i < lit { base + 11. } else { base };
            meter = meter.child(
                div()
                    .w(px(3.5))
                    .h(px(h))
                    .rounded(px(2.))
                    .bg(if i < lit { white(0.95) } else { white(0.28) }),
            );
        }

        // 药丸铺满窗口：不留透明边，阴影就跟着圆角形状走。
        // 也不自己加 box-shadow —— macOS 已经给窗口画了一层，叠上去只会脏。
        div()
            .size_full()
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(16.))
            // 不自己画圆角矩形了 —— 透明窗口 + 圆角块的边上总有一条约 13% 白的
            // 1px 亮边（实测：红底版本顶边 rgb(231,87,58) vs 主体 rgb(227,57,23)，
            // 带我们的色 → 是叠在填充之上的，不是窗口外框），在浅色桌面上就是一圈白框。
            // 直接让窗口本身当容器：不透明窗口 + 铺满的深色底，干净且没有合成边缘。
            .bg(hsla_of(0x14161c, 1.0))
            .text_color(white(0.95))
            .child(div().flex_none().text_color(c(ACCENT)).child(icon("mic", 16.)))
            .child(div().flex_1().min_w(px(0.)).flex().justify_center().child(meter))
            .child(
                div()
                    .flex_none()
                    .text_size(px(11.5))
                    .font_weight(w(560.))
                    .text_color(white(0.7))
                    .child(SharedString::from(if dictating { "松手出字" } else { "麦克风已开" })),
            )
    }
}

/// 开一个悬浮窗，放在主屏底部中间
pub fn open(rt: Arc<Runtime>, cx: &mut App) -> Option<WindowHandle<Hud>> {
    let db = cx.primary_display()?.bounds();
    let x = db.origin.x + (db.size.width - px(W)) / 2.;
    let y = db.origin.y + db.size.height - px(H + BOTTOM_GAP);
    let opts = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: gpui::point(x, y),
            size: size(px(W), px(H)),
        })),
        titlebar: None,
        // 不抢焦点 —— 否则文字打不进你原来的输入框
        focus: false,
        show: true,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        // 不透明 —— 透明窗口在圆角边缘会留亮边，见 render 里的说明
        window_background: gpui::WindowBackgroundAppearance::Opaque,
        ..Default::default()
    };
    cx.open_window(opts, |_, cx| cx.new(|cx| Hud::new(rt, cx))).ok()
}

/// 关掉悬浮窗
pub fn close(h: &WindowHandle<Hud>, cx: &mut App) {
    let _ = h.update(cx, |_, window, _| window.remove_window());
}

/// 让编译器知道 Entity<Hud> 是被用到的
pub type HudEntity = Entity<Hud>;
