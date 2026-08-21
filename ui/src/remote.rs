//! 左侧的软遥控器 —— 与 `design/mockup.html` 同一套几何。
//!
//! 机身那条上下微弧的轮廓 CSS 的 border-radius 画不出来（会把顶边削平），
//! 所以走 `PathBuilder` 把设计稿那条 SVG path 原样搬过来。外壳是四段渐变，
//! 而 gpui 的 linear_gradient 只吃两个色标，于是按 CSS 的停靠点
//! (0% / 18% / 62% / 100%) 切成三条各自带渐变的子路径，交界处重叠 1px 免出缝。

use crate::theme::*;
use crate::widget::*;
use crate::FireVibe;
use firevibe_core::layout::{
    scaled, Slot, BODY_VIEW_H, BODY_VIEW_W, DESIGN_H, DESIGN_W, DPAD_WELL, MIC_SLIT,
    VOL_CAPSULE, VOL_CAPSULE_R,
};
use gpui::{
    canvas, div, linear_color_stop, linear_gradient, point, prelude::*, px, AnyElement, Background,
    Context, PathBuilder, Pixels, Point,
};

pub const REMOTE_W: f32 = DESIGN_W;
pub const REMOTE_H: f32 = DESIGN_H;
/// 遥控器所在那一栏的宽度。遥控器本身 146.2 宽，在这栏里居中。
pub const COL_LEFT_W: f32 = 300.;

/// 外壳渐变的三条带子：(起点 y%, 终点 y%, 起色, 终色)
const BANDS: [(f32, f32, u32, u32); 3] = [
    (0.00, 0.18, 0x3a3d47, 0x22242b),
    (0.18, 0.62, 0x22242b, 0x101216),
    (0.62, 1.00, 0x101216, 0x1c1e25),
];

impl FireVibe {
    pub fn remote(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let w = REMOTE_W;
        let s = w / DESIGN_W;

        let (sl, st, sw, sh) = scaled(MIC_SLIT, w);
        let (wl, wt, ww, _) = scaled(DPAD_WELL, w);
        let (vl, vt, vw, vh) = scaled(VOL_CAPSULE, w);
        let vr = VOL_CAPSULE_R * s;

        let mut root = div()
            .relative()
            .w(px(REMOTE_W))
            .h(px(REMOTE_H))
            .flex_none()
            // 机身
            .child(
                div().absolute().inset_0().size_full().child(canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| paint_body(bounds.origin, w, window),
                )),
            )
            // 顶部麦克风缝
            .child(
                div()
                    .absolute()
                    .left(px(sl))
                    .top(px(st))
                    .w(px(sw))
                    .h(px(sh))
                    .rounded(px(2. * s))
                    .bg(c(MIC_SLIT_C)),
            )
            // D-pad 凹环
            .child(
                div()
                    .absolute()
                    .left(px(wl))
                    .top(px(wt))
                    .size(px(ww))
                    .rounded(px(ww / 2.))
                    .bg(c(WELL)),
            )
            // 音量摇杆的凹槽
            .child(
                div()
                    .absolute()
                    .left(px(vl - 1.))
                    .top(px(vt - 1.))
                    .w(px(vw + 2.))
                    .h(px(vh + 2.))
                    .rounded(px(vr + 1.))
                    .bg(c(WELL)),
            );

        // 21 个按键。音量摇杆是一整块胶囊分两半，单独画。
        for slot in Slot::ALL {
            if matches!(slot, Slot::VolUp | Slot::VolDown) {
                continue;
            }
            root = root.child(self.key_btn(slot, w, cx));
        }
        root = root.child(self.vol_rocker(w, cx));
        root
    }

    /// 图上按键的三种状态。**鼠标按住是「凹下去」，实体按下是「亮起来」** ——
    /// 前者要模拟真按键的手感，后者是给人看「刚才按的是哪个键」，反过来就别扭了。
    pub fn press_kind(&self, s: Slot) -> Press {
        if self.mouse_down == Some(s) {
            return Press::Push;
        }
        if self.soft.map(|(x, _)| x == s).unwrap_or(false) {
            return Press::Flash;
        }
        let hit = self
            .rt
            .cfg
            .read()
            .slot_key(s)
            .map(|k| self.pressed.contains(&k))
            .unwrap_or(false);
        if hit {
            Press::Flash
        } else {
            Press::None
        }
    }

    fn key_btn(&self, slot: Slot, w: f32, cx: &mut Context<Self>) -> AnyElement {
        let s = w / DESIGN_W;
        let (l, t, bw, bh) = slot.rect(w, 0.);
        let pk = self.press_kind(slot);
        let r = slot.design_radius() * s;
        // 按住时四周各收 1px —— 压暗对深色键几乎看不出来，缩一圈才像真按下去了
        let d = if pk == Press::Push { 1.0 } else { 0.0 };

        let mut b = div()
            .id(("key", slot as usize))
            .absolute()
            .left(px(l + d))
            .top(px(t + d))
            .w(px(bw - 2. * d))
            .h(px(bh - 2. * d))
            .rounded(px((r - d).max(2.)))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.mouse_down = Some(slot);
                    cx.notify();
                }),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.tap(slot, cx)));

        b = match slot {
            // 方向瓣：无底色，只在悬停/按下时透出一层白
            Slot::Up | Slot::Down | Slot::Left | Slot::Right => {
                let base = b.text_color(c(DPAD_INK)).hover(|st| st.bg(white(0.05)));
                match pk {
                    Press::Push => base.bg(black(0.28)),
                    Press::Flash => base.bg(white(0.16)),
                    Press::None => base,
                }
            }
            Slot::Ok => tint(b, pk, OK_TOP, OK_BOT, ok_shadow()),
            Slot::Mic => tint(b, pk, MIC_TOP, MIC_BOT, mic_shadow()).text_color(c(MIC_INK)),
            Slot::App1 | Slot::App2 | Slot::App3 | Slot::App4 => {
                let i = slot as usize - Slot::App1 as usize;
                let (top, bot, ink) = APP_TINT[i];
                tint(b, pk, top, bot, btn_shadow())
                    .text_color(c(ink))
                    .text_size(px(9.5))
                    .font_weight(w_(800.))
            }
            _ => tint(b, pk, BTN_TOP, BTN_BOT, btn_shadow())
                .when(pk == Press::None, |d| {
                    d.hover(|st| st.bg(grad_v(BTN_TOP_H, BTN_BOT_H)))
                })
                .text_color(c(BTN_INK)),
        };

        // 图上的字形：App 键是文字，其余是图标
        match slot {
            Slot::App1 | Slot::App2 | Slot::App3 | Slot::App4 => {
                b.child(slot.glyph()).into_any_element()
            }
            // OK 键是光板，没有字形
            Slot::Ok => b.into_any_element(),
            _ => {
                let (name, size) = glyph_icon(slot);
                b.child(icon(name, size * s)).into_any_element()
            }
        }
    }

    /// 音量摇杆：一整块全圆角胶囊，上下两半各自可点
    fn vol_rocker(&self, w: f32, cx: &mut Context<Self>) -> AnyElement {
        let s = w / DESIGN_W;
        let (l, t, cw, ch) = scaled(VOL_CAPSULE, w);
        let r = VOL_CAPSULE_R * s;
        let up_pk = self.press_kind(Slot::VolUp);
        let dn_pk = self.press_kind(Slot::VolDown);

        let half = |slot: Slot, ico: &str, pk: Press, cx: &mut Context<Self>| {
            let d = div()
                .id(("vol", slot as usize))
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(c(BTN_INK))
                .cursor_pointer()
                .hover(|st| st.bg(white(0.07)))
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.mouse_down = Some(slot);
                        cx.notify();
                    }),
                )
                .on_click(cx.listener(move |this, _, _, cx| this.tap(slot, cx)))
                .child(icon(ico, 15. * s));
            match pk {
                Press::Push => d.bg(black(0.3)),
                Press::Flash => d.bg(white(0.18)),
                Press::None => d,
            }
        };

        div()
            .absolute()
            .left(px(l))
            .top(px(t))
            .w(px(cw))
            .h(px(ch))
            .rounded(px(r))
            .overflow_hidden()
            .bg(grad_v(BTN_TOP, BTN_BOT))
            .shadow(btn_shadow())
            .flex()
            .flex_col()
            .child(half(Slot::VolUp, "plus", up_pk, cx))
            .child(div().h(px(1.)).bg(black(0.45)))
            .child(half(Slot::VolDown, "minus", dn_pk, cx))
            .into_any_element()
    }
}

/// 图上按键的按下状态
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Press {
    None,
    /// 鼠标按住 —— 凹下去：压暗、撤掉投影
    Push,
    /// 实体按下 / 点击后的一瞬 —— 亮起来，好认是哪个键
    Flash,
}

/// 按三态给底色和投影。Push 去掉投影是关键 —— 有投影就永远像凸起的。
fn tint(
    d: gpui::Stateful<gpui::Div>,
    pk: Press,
    top: u32,
    bot: u32,
    shadow: Vec<gpui::BoxShadow>,
) -> gpui::Stateful<gpui::Div> {
    match pk {
        Press::None => d.bg(grad_v(top, bot)).shadow(shadow),
        Press::Push => d
            .bg(grad_v(darken(top), darken(bot)))
            .shadow_none()
            .border_1()
            .border_color(black(0.45)),
        Press::Flash => d.bg(grad_v(lighten(top), lighten(bot))).shadow(shadow),
    }
}

/// 压暗一档，做「凹下去」的底色
fn darken(v: u32) -> u32 {
    let f = |sh: u32| {
        let ch = (v >> sh) & 0xff;
        (ch * 55 / 100) << sh
    };
    f(16) | f(8) | f(0)
}

fn w_(v: f32) -> gpui::FontWeight {
    gpui::FontWeight(v)
}

/// 按下时把颜色提亮一档，让 App 键也有反馈
fn lighten(v: u32) -> u32 {
    let f = |sh: u32| {
        let ch = (v >> sh) & 0xff;
        (ch + (255 - ch) / 5).min(255) << sh
    };
    f(16) | f(8) | f(0)
}

/// 每个位置在图上用哪个图标、多大（设计稿实测）
fn glyph_icon(s: Slot) -> (&'static str, f32) {
    match s {
        Slot::Power => ("power", 14.),
        Slot::Mic => ("mic", 17.),
        Slot::Up => ("chevron-up", 12.),
        Slot::Down => ("chevron-down", 12.),
        Slot::Left => ("chevron-left", 12.),
        Slot::Right => ("chevron-right", 12.),
        Slot::Back => ("undo-2", 14.),
        Slot::Home => ("house", 14.),
        Slot::Menu => ("menu", 14.),
        Slot::Rewind => ("rew-solid", 14.5),
        Slot::Play => ("playpause-solid", 13.),
        Slot::Forward => ("ffwd-solid", 14.5),
        Slot::Mute => ("volume-x", 14.),
        Slot::Tv => ("tv", 14.),
        Slot::VolUp => ("plus", 15.),
        Slot::VolDown => ("minus", 15.),
        _ => ("circle", 12.),
    }
}

// ── 机身轮廓 ──

/// 画机身：三条带渐变的横带 + 边沿高光
fn paint_body(o: Point<Pixels>, w: f32, window: &mut gpui::Window) {
    let k = w / BODY_VIEW_W;
    let p = |x: f32, y: f32| point(o.x + px(x * k), o.y + px(y * k));

    for (i, (y0, y1, from, to)) in BANDS.iter().enumerate() {
        let mut b = PathBuilder::fill();
        // 交界处各自外扩 0.5px，避免抗锯齿露出底色
        let top = y0 * BODY_VIEW_H - if i == 0 { 0. } else { 0.5 / k };
        let bot = y1 * BODY_VIEW_H + if i == 2 { 0. } else { 0.5 / k };

        if i == 0 {
            // 上沿：两段三次贝塞尔组成的浅穹顶
            b.move_to(p(0., 60.));
            b.cubic_bezier_to(p(158., 0.), p(0., 27.), p(41.1, 0.));
            b.cubic_bezier_to(p(316., 60.), p(274.9, 0.), p(316., 27.));
            b.line_to(p(316., bot));
            b.line_to(p(0., bot));
        } else if i == 2 {
            b.move_to(p(0., top));
            b.line_to(p(316., top));
            b.line_to(p(316., 987.));
            b.cubic_bezier_to(p(158., 1047.), p(316., 1020.), p(274.9, 1047.));
            b.cubic_bezier_to(p(0., 987.), p(41.1, 1047.), p(0., 1020.));
        } else {
            b.move_to(p(0., top));
            b.line_to(p(316., top));
            b.line_to(p(316., bot));
            b.line_to(p(0., bot));
        }
        b.close();
        if let Ok(path) = b.build() {
            window.paint_path(path, band_grad(*from, *to));
        }
    }

    // 整圈极淡边沿
    let mut rim = PathBuilder::stroke(px(2.4 * k));
    rim.move_to(p(0., 60.));
    rim.cubic_bezier_to(p(158., 0.), p(0., 27.), p(41.1, 0.));
    rim.cubic_bezier_to(p(316., 60.), p(274.9, 0.), p(316., 27.));
    rim.line_to(p(316., 987.));
    rim.cubic_bezier_to(p(158., 1047.), p(316., 1020.), p(274.9, 1047.));
    rim.cubic_bezier_to(p(0., 987.), p(41.1, 1047.), p(0., 1020.));
    rim.close();
    if let Ok(path) = rim.build() {
        window.paint_path(path, white(0.05));
    }

    // 上沿高光（CSS rim 渐变的 0%→6% 那一段）
    let mut hi = PathBuilder::stroke(px(2.4 * k));
    hi.move_to(p(0., 60.));
    hi.cubic_bezier_to(p(158., 0.), p(0., 27.), p(41.1, 0.));
    hi.cubic_bezier_to(p(316., 60.), p(274.9, 0.), p(316., 27.));
    if let Ok(path) = hi.build() {
        window.paint_path(
            path,
            linear_gradient(
                180.,
                linear_color_stop(white(0.28), 0.),
                linear_color_stop(white(0.03), 1.),
            ),
        );
    }

    // 下沿反光
    let mut lo = PathBuilder::stroke(px(2.4 * k));
    lo.move_to(p(316., 987.));
    lo.cubic_bezier_to(p(158., 1047.), p(316., 1020.), p(274.9, 1047.));
    lo.cubic_bezier_to(p(0., 987.), p(41.1, 1047.), p(0., 1020.));
    if let Ok(path) = lo.build() {
        window.paint_path(path, white(0.10));
    }
}

fn band_grad(from: u32, to: u32) -> Background {
    linear_gradient(
        SHELL_ANGLE,
        linear_color_stop(c(from), 0.),
        linear_color_stop(c(to), 1.),
    )
}
