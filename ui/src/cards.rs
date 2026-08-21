//! 自定义操作区 —— 一个按键一张卡，卡里分短按 / 长按两行。

use crate::theme::*;
use crate::widget::*;
use crate::{EditState, FireVibe};
use firevibe_core::{
    config::{Action, ActionType},
    layout::Slot,
    runtime::applescript_presets,
};
use gpui::{deferred, div, prelude::*, px, AnyElement, Context, ElementId, SharedString};
use gpui_component::input::InputState;

impl FireVibe {
    pub fn action_section(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let l = self.l();
        let slots: Vec<Slot> =
            self.rt.cfg.read().profile().actions.iter().map(|a| a.slot).collect();

        let mut grid = div().flex().flex_col().gap(px(14.));
        // 设计稿是 2 列等宽等高栅格 —— gpui 没有 grid，按两两一行铺，
        // 行内 flex_1 + items_stretch 就能等宽等高。
        for row in slots.chunks(2) {
            let mut r = div().flex().gap(px(14.));
            for &s in row {
                r = r.child(div().flex_1().min_w(px(0.)).child(self.action_card(s, cx)));
            }
            if row.len() == 1 {
                r = r.child(div().flex_1().min_w(px(0.)));
            }
            grid = grid.child(r);
        }

        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .mb(px(12.))
                    .child(section_lab(l.actions()))
                    .child(spacer())
                    .child(add_btn("add-key", l.add_key()).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.adding = true;
                            cx.notify();
                        },
                    ))),
            )
            .child(if slots.is_empty() {
                // 空状态。留一片空白比什么都不说更糟 —— 用户不知道是没配还是坏了。
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(10.))
                    .py(px(56.))
                    .rounded(px(14.))
                    .border_1()
                    .border_color(c(LINE))
                    .bg(c(SURFACE))
                    .child(
                        div()
                            .w(px(46.))
                            .h(px(46.))
                            .rounded(px(23.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(c(HOVER))
                            .text_color(c(INK3))
                            .child(icon("plus", 20.)),
                    )
                    .child(
                        div()
                            .text_size(px(13.5))
                            .font_weight(w(590.))
                            .text_color(c(INK))
                            .child("这套方案还没有自定义按键"),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(c(INK3))
                            .child("遥控器上的键保持系统原本的行为。点右上角「添加按键」挑一颗来配。"),
                    )
                    .into_any_element()
            } else {
                grid.into_any_element()
            })
    }

    fn action_card(&self, slot: Slot, cx: &mut Context<Self>) -> AnyElement {
        let cfg = self.rt.cfg.read();
        let disabled = cfg.profile().is_disabled(slot);
        let sa = cfg.profile().get(slot).cloned();
        drop(cfg);
        let (short, long) = match &sa {
            Some(x) => (x.short.clone(), x.long.clone()),
            None => (Action::none(), Action::none()),
        };
        // 用过渡进度算样式 —— gpui 没有 CSS transition，边框色和阴影都自己插值
        let t = self.card_t(slot);
        let l = self.l();

        let (mut btop, mut bbot, bink) = badge_tint(slot);
        if disabled {
            btop = desaturate(btop);
            bbot = desaturate(bbot);
        }

        let head = div()
            .relative()
            .flex()
            .items_center()
            .gap(px(11.))
            .px(px(14.))
            .pt(px(13.))
            .pb(px(11.))
            .child(
                div()
                    .size(px(36.))
                    .flex_none()
                    // 圆形 —— 卡片里所有带背景的东西都是圆的
                    .rounded(px(18.))
                    .bg(grad(160., btop, bbot))
                    .text_color(c(bink))
                    .shadow(sh1())
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(disabled, |d| d.opacity(0.5))
                    .child(badge_glyph(slot)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(7.))
                            .child(
                                div()
                                    .text_size(px(13.5))
                                    .font_weight(w(590.))
                                    .text_color(c(INK))
                                    .child(SharedString::from(card_title(slot))),
                            )
                            .when(disabled, |d| d.child(tag_off(l.disabled_tag()))),
                    )
            )
            .child(self.more_button(slot, cx));

        let body = div()
            .flex_1()
            .flex()
            .flex_col()
            .justify_center()
            .border_t_1()
            .border_color(c(LINE))
            .px(px(14.))
            .pt(px(4.))
            .pb(px(8.))
            .child(self.trig_row(slot, false, &short, t, cx))
            .child(div().h(px(1.)).bg(c(LINE_SOFT)))
            .child(self.trig_row(slot, true, &long, t, cx));

        card()
            .id(("card", slot as usize))
            .when(disabled, |d| d.bg(c(OFF_CARD)))
            .border_color(c(mix(LINE, LINE_STRONG, t)))
            .shadow(sh_lerp(t))
            .on_hover(cx.listener(move |this, over: &bool, _, cx| {
                if *over {
                    this.set_hover(Some(slot));
                } else if this.hover_card == Some(slot) {
                    this.set_hover(None);
                    this.menu_open = None;
                }
                cx.notify();
            }))
            .child(head)
            .child(body)
            .into_any_element()
    }

    /// 卡片右上角的「…」及其菜单
    fn more_button(&self, slot: Slot, cx: &mut Context<Self>) -> AnyElement {
        let l = self.l();
        let open = self.menu_open == Some(slot);
        let disabled = self.rt.cfg.read().profile().is_disabled(slot);

        let btn = div()
            .id(("more", slot as usize))
            .size(px(26.))
            .rounded(px(13.))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .text_color(c(mix(MORE_IDLE, INK2, self.card_t(slot))))
            .hover(|s| s.bg(c(HOVER)).text_color(c(INK)))
            .child(icon("ellipsis", 15.))
            .on_click(cx.listener(move |this, _, _, cx| {
                if !this.just_dismissed_pub() {
                    this.menu_open = if this.menu_open == Some(slot) { None } else { Some(slot) };
                    cx.notify();
                }
            }));

        let mut wrap = div().relative().flex_none().child(btn);

        if open {
            // deferred：gpui 按子节点顺序绘制，菜单在 card-head 里，
            // 不延后就会被 card-body 里后画的「测试 / 编辑」盖住
            wrap = wrap.child(deferred(
                div()
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                        this.dismiss_menus_pub();
                        cx.notify();
                    }))
                    .absolute()
                    .right(px(0.))
                    .top(px(30.))
                    .min_w(px(152.))
                    .bg(c(SURFACE))
                    .border_1()
                    .border_color(c(LINE_STRONG))
                    .rounded(px(10.))
                    .shadow(sh3())
                    .p(px(5.))
                    .flex()
                    .flex_col()
                    .child(
                        menu_item(("mi-dis", slot as usize), "eye-off", if disabled { l.enable_key() } else { l.disable_key() }, false)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let cur = this.rt.cfg.read().profile().is_disabled(slot);
                                this.rt.cfg.write().profile_mut().set_disabled(slot, !cur);
                                this.save();
                                this.menu_open = None;
                                cx.notify();
                            })),
                    )
                    .child(hline().my(px(4.)).mx(px(2.)))
                    .child(
                        menu_item(("mi-rm", slot as usize), "delete", l.remove(), true).on_click(
                            cx.listener(move |this, _, _, cx| {
                                this.rt.cfg.write().profile_mut().remove(slot);
                                this.save();
                                this.menu_open = None;
                                cx.notify();
                            }),
                        ),
                    ),
            ));
        }
        wrap.into_any_element()
    }

    /// 短按 / 长按一行
    fn trig_row(
        &self,
        slot: Slot,
        long: bool,
        a: &Action,
        t: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let l = self.l();
        let empty = a.kind == ActionType::None;
        let (val, note) = describe(a);

        let mut body = div()
            .flex_1()
            .min_w(px(0.))
            .min_h(px(36.))
            .flex()
            .flex_col()
            .justify_center();

        let mut valrow = div().flex().items_center().gap(px(6.)).text_size(px(12.5));
        if empty {
            valrow = valrow
                .text_color(c(INK3))
                .font_weight(w(450.))
                .child(SharedString::from(l.unset()));
        } else {
            if let Some(ic) = kind_icon(a.kind) {
                valrow = valrow.child(div().text_color(c(INK3)).flex_none().child(icon(ic, 13.)));
            }
            valrow = valrow
                .text_color(c(INK))
                .font_weight(w(520.))
                .child(SharedString::from(val));
        }
        body = body.child(valrow);
        if !note.is_empty() {
            body = body.child(
                div()
                    .text_size(px(11.))
                    .text_color(c(INK3))
                    .mt(px(2.))
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(SharedString::from(note)),
            );
        }

        // 淡入淡出：t 极小就干脆不画，避免看不见还能点
        let mut acts = div().flex().items_center().gap(px(4.)).flex_none().opacity(t);
        if t > 0.02 {
            if !empty {
                acts = acts.child(
                    round_btn(("test", slot as usize * 2 + long as usize), "play", false)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let r = this.rt.trigger_slot(slot, long);
                            this.toast(if r.is_empty() { "已执行".into() } else { r });
                            cx.notify();
                        })),
                );
            }
            acts = acts.child(
                round_btn(
                    ("edit", slot as usize * 2 + long as usize),
                    if empty { "plus" } else { "settings-2" },
                    true,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.open_editor(slot, long, window, cx);
                })),
            );
        }

        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .py(px(9.))
            .child(
                div()
                    .w(px(32.))
                    .flex_none()
                    .text_size(px(10.5))
                    .font_weight(w(680.))
                    .text_color(c(INK3))
                    .child(SharedString::from(if long { l.long_press() } else { l.short_press() })),
            )
            .child(body)
            .child(acts)
            .into_any_element()
    }

    /// 「添加按键」面板：列出还没配过的位置
    pub fn add_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let l = self.l();
        let used: Vec<Slot> =
            self.rt.cfg.read().profile().actions.iter().map(|a| a.slot).collect();
        let free: Vec<Slot> = Slot::ALL.into_iter().filter(|s| !used.contains(s)).collect();

        let mut chips = div().flex().flex_wrap().gap(px(6.));
        for s in free {
            chips = chips.child(
                slot_chip(s).on_click(cx.listener(
                    move |this, _, window, cx| {
                        this.rt.cfg.write().profile_mut().set_short(s, Action::none());
                        this.save();
                        this.adding = false;
                        this.open_editor(s, false, window, cx);
                    },
                )),
            );
        }

        overlay()
            .child(
                div()
                    .w(px(520.))
                    .bg(c(SURFACE))
                    .border_1()
                    .border_color(c(LINE))
                    .rounded(px(14.))
                    .shadow(sh3())
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .gap(px(12.))
                            .px(px(18.))
                            .pt(px(16.))
                            .pb(px(14.))
                            .child(
                                div()
                                    .flex_1()
                                    .child(
                                        div()
                                            .text_size(px(15.))
                                            .font_weight(w(620.))
                                            .child(SharedString::from(l.add_key())),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(11.5))
                                            .text_color(c(INK3))
                                            .mt(px(3.))
                                            .child(SharedString::from(l.add_key_hint())),
                                    ),
                            )
                            .child(icon_btn_sm("add-x", "close").on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.adding = false;
                                    cx.notify();
                                },
                            ))),
                    )
                    .child(div().px(px(18.)).pb(px(18.)).child(chips)),
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.adding = false;
                cx.notify();
            }))
    }
}

/// 半透明遮罩 + 居中内容。
/// `occlude()` 是必须的 —— 否则遮罩只是变暗，背后的卡片照样能点。
pub fn overlay() -> gpui::Stateful<gpui::Div> {
    div()
        .id("overlay")
        .absolute()
        .inset_0()
        .size_full()
        .occlude()
        .bg(black(0.28))
        .flex()
        .items_center()
        .justify_center()
}

fn menu_item(
    id: impl Into<ElementId>,
    ic: &str,
    label: impl Into<SharedString>,
    danger: bool,
) -> gpui::Stateful<gpui::Div> {
    let d = div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(9.))
        .py(px(7.))
        .rounded(px(7.))
        .text_size(px(12.5))
        .cursor_pointer()
        .child(icon(ic, 13.))
        .child(label.into());
    if danger {
        d.text_color(c(ERR)).hover(|s| s.bg(c(ERR_SOFT)))
    } else {
        d.text_color(c(INK)).hover(|s| s.bg(c(MENU_HOVER)))
    }
}

/// 卡片里的圆形图标按钮。`accent` 决定是主题色还是中性色。
fn round_btn(id: impl Into<ElementId>, ico: &str, accent: bool) -> gpui::Stateful<gpui::Div> {
    let d = div()
        .id(id)
        .size(px(26.))
        .rounded(px(13.))
        .flex()
        .items_center()
        .justify_center()
        .flex_none()
        .cursor_pointer()
        .child(icon(ico, 13.));
    if accent {
        d.text_color(c(ACCENT)).bg(c(ACCENT_SOFT)).hover(|s| s.bg(c(0xd2edfa)))
    } else {
        d.text_color(c(INK2)).bg(c(HOVER)).hover(|s| s.bg(c(LINE_STRONG)).text_color(c(INK)))
    }
}

/// 「添加按键」面板里的一颗：图标 + 名字，比纯文字好认得多
fn slot_chip(s: Slot) -> gpui::Stateful<gpui::Div> {
    div()
        .id(("addslot", s as usize))
        .flex()
        .items_center()
        .gap(px(6.))
        .px(px(10.))
        .py(px(6.))
        .rounded(px(R_SM))
        .border_1()
        .border_color(c(LINE_STRONG))
        .bg(c(SURFACE))
        .text_color(c(INK2))
        .text_size(px(12.))
        .cursor_pointer()
        .hover(|st| st.border_color(c(INK3)).text_color(c(INK)))
        .child(div().text_color(c(INK3)).flex_none().child(slot_icon(s)))
        .child(SharedString::from(card_title(s)))
}

/// 位置对应的图标。App 键用首字母，其余用实体键的图标。
fn slot_icon(s: Slot) -> AnyElement {
    let letter = match s {
        Slot::App1 => Some("P"),
        Slot::App2 => Some("N"),
        Slot::App3 => Some("D"),
        Slot::App4 => Some("h"),
        _ => None,
    };
    if let Some(t) = letter {
        return div()
            .w(px(13.))
            .text_center()
            .text_size(px(11.))
            .font_weight(w(700.))
            .child(t)
            .into_any_element();
    }
    icon(
        match s {
            Slot::Mic => "mic",
            Slot::Home => "house",
            Slot::Menu => "menu",
            Slot::Tv => "tv",
            Slot::Power => "power",
            Slot::Back => "undo-2",
            Slot::Play => "playpause-solid",
            Slot::Rewind => "rew-solid",
            Slot::Forward => "ffwd-solid",
            Slot::Mute => "volume-x",
            Slot::VolUp => "plus",
            Slot::VolDown => "minus",
            Slot::Up => "chevron-up",
            Slot::Down => "chevron-down",
            Slot::Left => "chevron-left",
            Slot::Right => "chevron-right",
            Slot::Ok => "circle-check",
            _ => "circle",
        },
        13.,
    )
    .into_any_element()
}

/// 卡片标题 —— 四个 App 键按机身印字来，跟设计稿一致
pub fn card_title(s: Slot) -> &'static str {
    match s {
        Slot::App1 => "Prime Video 键",
        Slot::App2 => "NETFLIX 键",
        Slot::App3 => "Disney+ 键",
        Slot::App4 => "hulu 键",
        other => other.label(),
    }
}

/// 键徽章配色
fn badge_tint(s: Slot) -> (u32, u32, u32) {
    match s {
        Slot::Mic => BADGE_MIC,
        Slot::App1 => BADGE_TINT[0],
        Slot::App2 => BADGE_TINT[1],
        Slot::App3 => BADGE_TINT[2],
        Slot::App4 => BADGE_TINT[3],
        _ => BADGE_DEFAULT,
    }
}

/// 徽章里画什么：App 键用首字母，其余用图标
fn badge_glyph(s: Slot) -> AnyElement {
    let letter = match s {
        Slot::App1 => Some("P"),
        Slot::App2 => Some("N"),
        Slot::App3 => Some("D"),
        Slot::App4 => Some("h"),
        _ => None,
    };
    if let Some(t) = letter {
        return div().text_size(px(14.)).font_weight(w(700.)).child(t).into_any_element();
    }
    let name = match s {
        Slot::Mic => "mic",
        Slot::Home => "house",
        Slot::Menu => "menu",
        Slot::Tv => "tv",
        Slot::Power => "power",
        Slot::Back => "undo-2",
        Slot::Play => "playpause-solid",
        Slot::Rewind => "rew-solid",
        Slot::Forward => "ffwd-solid",
        Slot::Mute => "volume-x",
        Slot::VolUp => "plus",
        Slot::VolDown => "minus",
        Slot::Up => "chevron-up",
        Slot::Down => "chevron-down",
        Slot::Left => "chevron-left",
        Slot::Right => "chevron-right",
        Slot::Ok => "circle-check",
        Slot::App1 | Slot::App2 | Slot::App3 | Slot::App4 => unreachable!(),
    };
    icon(name, 17.).into_any_element()
}

/// 动作类型的小图标
pub fn kind_icon(k: ActionType) -> Option<&'static str> {
    Some(match k {
        ActionType::None => return None,
        ActionType::Key => "keyboard",
        ActionType::Text => "a-large-small",
        ActionType::OpenApp => "external-link",
        ActionType::AppleScript => "zap",
        ActionType::Shell => "square-terminal",
        ActionType::VoicePtt | ActionType::VoiceToggle => "mic",
        ActionType::VoiceHotkey => "zap",
        ActionType::VoiceDictate => "mic",
    })
}

/// 卡片上两行文案：(主行, 副行)。跟设计稿逐字对齐。
/// 按键名 → 好看的符号。卡片上写 `rightcontrol` 太丑，写「右⌃」一眼就懂。
pub fn key_label(key: &str) -> String {
    let sym = match key.to_ascii_lowercase().as_str() {
        "leftcmd" | "cmd" | "command" => "左⌘",
        "rightcmd" => "右⌘",
        "leftoption" | "option" | "alt" => "左⌥",
        "rightoption" => "右⌥",
        "leftshift" | "shift" => "左⇧",
        "rightshift" => "右⇧",
        "leftcontrol" | "ctrl" | "control" => "左⌃",
        "rightcontrol" => "右⌃",
        "fn" | "function" => "fn",
        "space" => "空格",
        "return" | "enter" => "⏎",
        "tab" => "⇥",
        "escape" | "esc" => "esc",
        "delete" | "backspace" => "⌫",
        "forwarddelete" => "⌦",
        "up" => "↑",
        "down" => "↓",
        "left" => "←",
        "right" => "→",
        _ => return key.to_uppercase(),
    };
    sym.to_string()
}

/// 组合键整体的显示名，比如 ⌘⇧A
fn combo_label(mods: &[String], key: &str) -> String {
    let m: String = mods.iter().map(|x| key_label(x)).collect();
    if key.is_empty() {
        return if m.is_empty() { "未选".into() } else { m };
    }
    format!("{m}{}", key_label(key))
}

pub fn describe(a: &Action) -> (String, String) {
    match a.kind {
        ActionType::None => (String::new(), String::new()),
        ActionType::Key => (
            format!("映射按键 · {}", combo_label(&a.mods, &a.key)),
            String::new(),
        ),
        ActionType::Text => ("输入文字".into(), a.arg.clone()),
        ActionType::OpenApp => (format!("打开 {}", app_label(&a.arg)), a.arg.clone()),
        ActionType::AppleScript => {
            let name = applescript_presets()
                .iter()
                .find(|(_, code)| *code == a.arg)
                .map(|(n, _)| *n)
                .unwrap_or("自定义");
            (format!("AppleScript · {name}"), a.arg.clone())
        }
        ActionType::Shell => ("执行命令".into(), a.arg.clone()),
        ActionType::VoiceToggle => {
            ("开始 / 停止说话".into(), "点一下开始，再点一下停止".into())
        }
        ActionType::VoicePtt => ("按住说话".into(), "按住送流，松手停止".into()),
        ActionType::VoiceDictate => (
            "语音转文字".into(),
            if a.arg == "hold" {
                "按住说话，松手识别并打字".into()
            } else {
                "点一下开始，再点一下结束并识别".into()
            },
        ),
        ActionType::VoiceHotkey => {
            let mode = match a.arg.as_str() {
                "hold" => "按住期间保持按下",
                "double" => "双击",
                _ => "敲一下",
            };
            (
                format!("第三方语音输入 · {}", combo_label(&a.mods, &a.key)),
                mode.into(),
            )
        }
    }
}

/// bundle id 显示成人看得懂的名字
pub fn app_label(t: &str) -> String {
    for (n, id) in firevibe_core::runtime::app_presets() {
        if *id == t {
            return n.to_string();
        }
    }
    t.rsplit('.').next().unwrap_or(t).to_string()
}

/// 给编辑弹窗用：初始化一个 InputState
pub fn new_input(
    text: &str,
    window: &mut gpui::Window,
    cx: &mut gpui::App,
) -> gpui::Entity<InputState> {
    let t = text.to_string();
    // auto_grow 而不是 multi_line：multi_line 模式的输入框是 h_full，
    // 外层没给固定高度时会塌成一行，长脚本被截断看不见。
    // auto_grow(2, 8) 正好等于设计稿 `.code-input{min-height:62px}` 的两行起高，
    // 内容多了自己长高，最多八行。
    cx.new(|cx| InputState::new(window, cx).auto_grow(2, 8).default_value(t))
}

/// 单行输入框（方案改名这类短文本用，不要 auto_grow 的多行）
pub fn new_line_input(
    text: &str,
    window: &mut gpui::Window,
    cx: &mut gpui::App,
) -> gpui::Entity<InputState> {
    let t = text.to_string();
    cx.new(|cx| InputState::new(window, cx).default_value(t))
}

/// 给编辑弹窗用：从 EditState 造回 Action
pub fn to_action(e: &EditState, arg: &str) -> Action {
    firevibe_core::runtime::make_action(e.kind, &e.key, e.mods.clone(), arg)
}
