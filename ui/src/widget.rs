//! 界面基元 —— 一一对应设计稿里的 CSS 类。
//! 全部返回 `Stateful<Div>`，调用方自己挂 `.on_click(...)`。

use crate::theme::*;
use gpui::{
    AnyElement,
    div, prelude::*, px, svg, App, Div, FontWeight, Hsla, IntoElement, RenderOnce, SharedString,
    Stateful, Window,
};

/// 图标。名字是 `ui/assets/icons/<name>.svg` 去掉后缀。
///
/// 必须是个组件而不是直接返回 `svg()`：gpui 的 `Svg` 只认**自己**样式里的
/// `text.color`，祖先的 `text_color` 不会级联下来（`Style::default()` 起手，
/// 不继承）。所以渲染时现场从 `window.text_style()` 取色 —— 这样父级的
/// `text_color` 和 `hover` 变色都还能生效。
#[derive(IntoElement)]
pub struct Ico {
    name: SharedString,
    size: f32,
    color: Option<Hsla>,
}

impl RenderOnce for Ico {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let col = self.color.unwrap_or_else(|| window.text_style().color);
        svg()
            .path(format!("icons/{}.svg", self.name))
            .size(px(self.size))
            .flex_none()
            .text_color(col)
    }
}

pub fn icon(name: &str, size: f32) -> Ico {
    Ico { name: name.to_string().into(), size, color: None }
}

pub fn w(v: f32) -> FontWeight {
    FontWeight(v)
}

// ── 按钮 ──

/// 34×34 描边图标按钮（右上角设置那种）
pub fn icon_btn(id: &'static str, name: &str) -> Stateful<Div> {
    icon_btn_sized(id, name, 34., 17., R_SM)
}

/// 28×28 小号
pub fn icon_btn_sm(id: &'static str, name: &str) -> Stateful<Div> {
    icon_btn_sized(id, name, 28., 15., 7.)
}

/// 自定尺寸的描边图标按钮，用来跟旁边的按钮对齐高度
pub fn icon_btn_px(
    id: &'static str,
    name: &str,
    box_: f32,
    ico: f32,
    r: f32,
) -> Stateful<Div> {
    icon_btn_sized(id, name, box_, ico, r)
}

fn icon_btn_sized(id: &'static str, name: &str, box_: f32, ico: f32, r: f32) -> Stateful<Div> {
    div()
        .id(id)
        .size(px(box_))
        .rounded(px(r))
        .border_1()
        .border_color(c(LINE))
        .bg(c(SURFACE))
        .shadow(sh1())
        .text_color(c(INK2))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|s| s.border_color(c(LINE_STRONG)).text_color(c(INK)))
        .child(icon(name, ico))
}

/// 蓝色主按钮
pub fn primary_btn(id: &'static str, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(6.))
        .px(px(15.))
        .py(px(7.))
        .rounded(px(R_SM))
        .bg(c(ACCENT))
        .text_color(c(SURFACE))
        .text_size(px(12.5))
        .font_weight(w(560.))
        .cursor_pointer()
        .hover(|s| s.bg(c(ACCENT_HOVER)))
        .child(label.into())
}

/// 蓝色主按钮 · 小号 · 带前置图标
pub fn primary_btn_sm_ico(
    id: &'static str,
    ico: &str,
    label: impl Into<SharedString>,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(5.))
        .px(px(11.))
        .py(px(5.))
        .rounded(px(R_SM))
        .bg(c(ACCENT))
        .text_color(c(SURFACE))
        .text_size(px(11.5))
        .font_weight(w(560.))
        .cursor_pointer()
        .hover(|s| s.bg(c(ACCENT_HOVER)))
        .child(icon(ico, 13.))
        .child(label.into())
}

/// 「添加按键」那颗，圆角大一点
pub fn add_btn(id: &'static str, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(6.))
        .px(px(13.))
        .py(px(7.))
        .rounded(px(9.))
        .bg(c(ACCENT))
        .text_color(c(SURFACE))
        .text_size(px(12.5))
        .font_weight(w(560.))
        .cursor_pointer()
        .hover(|s| s.bg(c(ACCENT_HOVER)))
        .child(icon("plus", 13.))
        .child(label.into())
}

/// 描边次要按钮 · 带前置图标。设计稿里图标一律在文字左边，
/// 所以不能用 `mini2(..).child(icon(..))` —— 那样图标会跑到文字后面。
pub fn mini2_ico(
    id: &'static str,
    ico: &str,
    label: impl Into<SharedString>,
) -> Stateful<Div> {
    mini2_base(id).child(icon(ico, 13.)).child(label.into())
}

/// 描边次要按钮（弹窗页脚 / 关于行）
pub fn mini2(id: &'static str, label: impl Into<SharedString>) -> Stateful<Div> {
    mini2_base(id).child(label.into())
}

fn mini2_base(id: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(5.))
        .px(px(11.))
        .py(px(6.))
        .rounded(px(R_SM))
        .border_1()
        .border_color(c(LINE_STRONG))
        .bg(c(SURFACE))
        .text_color(c(INK2))
        .text_size(px(12.))
        .cursor_pointer()
        .hover(|s| s.text_color(c(INK)).border_color(c(INK3)))
}

/// 状态条里的小幽灵按钮（断开）
pub fn ghost_btn(id: &'static str, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .px(px(9.))
        .py(px(3.))
        .rounded(px(6.))
        .border_1()
        .border_color(c(LINE_STRONG))
        .bg(c(SURFACE))
        .text_color(c(INK2))
        .text_size(px(11.5))
        .cursor_pointer()
        .hover(|s| s.text_color(c(INK)).border_color(c(INK3)))
        .child(label.into())
}

/// 黄色警示卡里的「安装」
pub fn install_btn(id: &'static str, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .px(px(11.))
        .py(px(4.))
        .rounded(px(7.))
        .bg(c(ACCENT))
        .text_color(c(SURFACE))
        .text_size(px(11.5))
        .font_weight(w(560.))
        .cursor_pointer()
        .hover(|s| s.bg(c(ACCENT_HOVER)))
        .child(label.into())
}

// ── 芯片 ──

pub fn chip(id: impl Into<gpui::ElementId>, label: impl Into<SharedString>, on: bool) -> Stateful<Div> {
    let base = div()
        .id(id)
        .px(px(11.))
        .py(px(5.))
        .rounded(px(R_SM))
        .border_1()
        .text_size(px(12.))
        .cursor_pointer()
        .child(label.into());
    if on {
        base.bg(c(ACCENT)).border_color(c(ACCENT)).text_color(c(SURFACE)).font_weight(w(550.))
    } else {
        base.bg(c(SURFACE))
            .border_color(c(LINE_STRONG))
            .text_color(c(INK2))
            .hover(|s| s.border_color(c(INK3)).text_color(c(INK)))
    }
}

/// 和 `chip` 一样，但**未选中**时用强调色底 + 描边 —— 给「这一堆里最该先看的那个」用。
///
/// 麦克风键的动作类型有十种，第三方语音输入夹在中间，第一次用的人根本找不到。
/// 选中之后回到普通选中态，不再特殊。
pub fn chip_hi(id: impl Into<gpui::ElementId>, label: impl Into<SharedString>, on: bool) -> Stateful<Div> {
    if on {
        return chip(id, label, true);
    }
    div()
        .id(id)
        .px(px(11.))
        .py(px(5.))
        .rounded(px(R_SM))
        .border_1()
        .text_size(px(12.))
        .cursor_pointer()
        .bg(c(ACCENT_SOFT))
        .border_color(c(ACCENT))
        .text_color(c(ACCENT_INK))
        .font_weight(w(550.))
        .hover(|s| s.bg(c(SURFACE)))
        .child(label.into())
}

pub fn chip_sm(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    on: bool,
) -> Stateful<Div> {
    let base = div()
        .id(id)
        .px(px(9.))
        .py(px(4.))
        .rounded(px(R_SM))
        .border_1()
        .text_size(px(11.5))
        .cursor_pointer()
        .child(label.into());
    if on {
        base.bg(c(ACCENT)).border_color(c(ACCENT)).text_color(c(SURFACE)).font_weight(w(550.))
    } else {
        base.bg(c(SURFACE))
            .border_color(c(LINE_STRONG))
            .text_color(c(INK2))
            .hover(|s| s.border_color(c(INK3)).text_color(c(INK)))
    }
}

// ── 表单控件 ──

/// 38×22 开关
pub fn switch_ui(id: &'static str, on: bool) -> Stateful<Div> {
    let track = div()
        .id(id)
        .w(px(38.))
        .h(px(22.))
        .rounded(px(11.))
        .flex()
        .items_center()
        .p(px(2.))
        .flex_none()
        .cursor_pointer();
    let knob = div().size(px(18.)).rounded(px(9.)).bg(c(SURFACE)).shadow(sh1());
    if on {
        track.bg(c(ACCENT)).justify_end().child(knob)
    } else {
        track.bg(c(LINE_STRONG)).child(knob)
    }
}

/// 分段选择器的单段。整体由调用方拼 `seg_wrap`。
pub fn seg_item(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    on: bool,
) -> Stateful<Div> {
    let base = div()
        .id(id)
        .px(px(11.))
        .py(px(4.))
        .rounded(px(6.))
        .text_size(px(11.5))
        .cursor_pointer()
        .child(label.into());
    if on {
        base.bg(c(SURFACE)).text_color(c(INK)).font_weight(w(560.)).shadow(sh1())
    } else {
        base.text_color(c(INK2))
    }
}

pub fn seg_wrap() -> Div {
    div().flex().bg(c(HOVER)).rounded(px(R_SM)).p(px(2.)).flex_none()
}

/// 数字步进器的外壳
pub fn stepper_wrap() -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(2.))
        .bg(c(HOVER))
        .rounded(px(R_SM))
        .px(px(4.))
        .py(px(2.))
        .flex_none()
}

pub fn stepper_btn(id: &'static str, glyph: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .w(px(20.))
        .flex()
        .justify_center()
        .text_size(px(13.))
        .text_color(c(INK2))
        .cursor_pointer()
        .hover(|s| s.text_color(c(INK)))
        .child(glyph)
}

// ── 文字 ──

/// 小号大写分组标题（方案 / 自定义操作 / 通用 / 关于）
pub fn section_lab(t: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(11.5))
        .font_weight(w(600.))
        .text_color(c(INK3))
        .child(t.into())
}

/// 弹窗里的字段标题
pub fn field_lab(t: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(11.5))
        .font_weight(w(600.))
        .text_color(c(INK3))
        .mb(px(7.))
        .child(t.into())
}

/// 「已禁用」徽标
pub fn tag_off(t: impl Into<SharedString>) -> Div {
    div()
        .px(px(6.))
        .rounded(px(99.))
        .border_1()
        .border_color(c(LINE))
        .bg(c(TAG_BG))
        .text_size(px(10.))
        .font_weight(w(650.))
        .text_color(c(INK3))
        .child(t.into())
}

// ── 容器 ──

/// 白卡（自定义操作那种）
pub fn card() -> Div {
    div()
        .flex()
        .flex_col()
        .min_w(px(0.))
        .bg(c(SURFACE))
        .border_1()
        .border_color(c(LINE))
        .rounded(px(R_LG))
        .shadow(sh1())
}

/// 设置页的分组容器
pub fn group() -> Div {
    div()
        .flex()
        .flex_col()
        .bg(c(SURFACE))
        .border_1()
        .border_color(c(LINE))
        .rounded(px(R_LG))
        .shadow(sh1())
        .overflow_hidden()
}

/// 分组里的一行
pub fn group_row() -> Div {
    div().flex().items_center().gap(px(12.)).px(px(15.)).py(px(13.))
}

pub fn spacer() -> Div {
    div().flex_1()
}

pub fn hline() -> Div {
    div().h(px(1.)).bg(c(LINE))
}


// ── 设置/适配页里的「一行」构件 ──

pub fn row_icon(name: &str) -> AnyElement {
    div().flex_none().flex().text_color(c(INK3)).child(icon(name, 16.)).into_any_element()
}

pub fn row_text(title: &str, hint: Option<&str>) -> AnyElement {
    let mut d = div()
        .flex_1()
        .min_w(px(0.))
        .flex()
        .flex_col()
        .gap(px(2.))
        .child(
            div()
                .text_size(px(13.))
                .font_weight(w(560.))
                .child(SharedString::from(title.to_string())),
        );
    if let Some(h) = hint {
        d = d.child(
            div()
                .text_size(px(11.5))
                .text_color(c(INK3))
                .child(SharedString::from(h.to_string())),
        );
    }
    d.into_any_element()
}

/// 标题和说明都是运行时拼出来的字符串时用这个
pub fn row_text2(title: impl Into<String>, hint: impl Into<String>) -> AnyElement {
    div()
        .flex_1()
        .min_w(px(0.))
        .flex()
        .flex_col()
        .gap(px(2.))
        .child(
            div()
                .text_size(px(13.))
                .font_weight(w(560.))
                .child(SharedString::from(title.into())),
        )
        .child(
            div()
                .text_size(px(11.5))
                .text_color(c(INK3))
                .child(SharedString::from(hint.into())),
        )
        .into_any_element()
}
