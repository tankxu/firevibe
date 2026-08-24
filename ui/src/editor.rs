//! 编辑操作弹窗。改的是临时状态，点保存才写回配置。

use crate::cards::{card_title, new_input, to_action};
use crate::theme::*;
use crate::widget::*;
use crate::{EditState, FireVibe};
use firevibe_core::{
    config::{Action, ActionType},
    inject::key_names,
    layout::Slot,
    runtime::{app_presets, applescript_presets},
};
use gpui::{div, prelude::*, px, AnyElement, Context, SharedString, Window};
use gpui_component::input::Input;

impl FireVibe {
    pub fn open_editor(
        &mut self,
        slot: Slot,
        long: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let cfg = self.rt.cfg.read();

        let a = cfg
            .profile()
            .get(slot)
            .map(|sa| {
                if long {
                    sa.long.clone()
                } else {
                    sa.short.clone()
                }
            })
            .unwrap_or_else(Action::none);
        drop(cfg);

        let input = new_input(&a.arg, window, cx);
        let body_in = new_input(&a.body, window, cx);
        let retries_in = new_input(&a.retries.to_string(), window, cx);
        let timeout_in = new_input(
            &(if a.timeout_ms > 0 { a.timeout_ms } else { 2000 }).to_string(),
            window,
            cx,
        );
        self.dialog = Some(EditState {
            slot,
            long,
            kind: a.kind,
            key: a.key.clone(),
            mods: a.mods.clone(),
            hold: a.arg == "hold",
            // 默认单击。参考项目的注释说豆包默认是双击，但实际用这些 app 的人
            // 说没见过双击触发，所以只作为可选项留着，不当默认。
            dbl: a.arg == "double",
            input,
            post: a.method.eq_ignore_ascii_case("POST"),
            body_in,
            retries_in,
            timeout_in,
            focus: cx.focus_handle(),
            recording: false,
            grab: None,
        });
        self.menu_open = None;
        // 打开就是热键类型且还没设过键 → 直接等你按
        if matches!(a.kind, ActionType::Key | ActionType::VoiceHotkey) && a.key.is_empty() {
            if let Some(dd) = &mut self.dialog {
                dd.recording = true;
                let h = dd.focus.clone();
                window.focus(&h);
            }
        }
        cx.notify();
    }
    pub fn edit_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
        let l = self.l();
        let Some(d) = &self.dialog else {
            return div().into_any_element();
        };
        let key_id = self
            .rt
            .cfg
            .read()
            .slot_key(d.slot)
            .map(|k| k.id())
            .unwrap_or_default();
        let sub = format!(
            "{} · {} · {}",
            card_title(d.slot),
            if d.long {
                l.long_press()
            } else {
                l.short_press()
            },
            key_id
        );
        // 动作类型
        let mut types = div().flex().flex_wrap().gap(px(6.));
        for k in ActionType::ALL {
            // 「按住说话」要靠松手收尾，短按（松手才触发一次）做不到；
            // 「开始/停止说话」是点一下翻转，挂在长按上没意义。各自只出现在该出现的地方。
            if !d.long && k == ActionType::VoicePtt {
                continue;
            }
            if d.long && k == ActionType::VoiceToggle {
                continue;
            }
            types = types.child(chip(("kind", k as usize), k.label(), k == d.kind).on_click(
                cx.listener(move |this, _, window, cx| {
                    if let Some(dd) = &mut this.dialog {
                        dd.kind = k;
                        // 选了要热键的类型、又还没设过键 —— 直接开始录，
                        // 省得再点一下输入框
                        let needs_hotkey = matches!(k, ActionType::Key | ActionType::VoiceHotkey);
                        if needs_hotkey && dd.key.is_empty() {
                            dd.recording = true;
                            let h = dd.focus.clone();
                            window.focus(&h);
                        } else {
                            dd.recording = false;
                        }
                    }
                    cx.notify();
                }),
            ));
        }
        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .px(px(18.))
            .pt(px(2.))
            .pb(px(18.))
            .child(div().child(field_lab(l.action_type())).child(types));
        // 参数区，按类型不同
        match d.kind {
            ActionType::Key => {
                body = body.child(
                    div()
                        .child(field_lab(l.hotkey()))
                        .child(self.hotkey_field(d, cx)),
                );
            }
            ActionType::VoiceHotkey => {
                // 纯修饰键会走 HID 设备层映射（见 core/src/hidremap.rs），
                // 不是合成事件 —— 这里说清楚，因为它的行为确实不一样。
                let hw = d.mods.is_empty()
                    && firevibe_core::hidremap::usage_of(&d.key).is_some();
                if hw {
                    body = body.child(
                        div()
                            .px(px(12.))
                            .py(px(9.))
                            .rounded(px(9.))
                            .bg(c(CODE_BG))
                            .border_1()
                            .border_color(c(LINE))
                            .text_size(px(11.5))
                            .text_color(c(INK2))
                            .line_height(gpui::relative(1.5))
                            .child(SharedString::from(
                                "这颗键会在硬件层直接变成这个修饰键 —— 系统收到的和你按真键盘\
                                 完全一样，所以那些只认硬件按键的语音工具也能收到。\
                                 应用退出时会自动还原。",
                            )),
                    );
                }
                let dbl = d.dbl;
                body = body.child(
div().child(field_lab(l.hotkey())).child(self.hotkey_field(d, cx))).when(!d.long, |b| b.child(
    div().flex().flex_col().gap(px(6.))
        .child(field_lab("触发方式"))
        .child(div().flex().gap(px(6.))
            .child(chip("hk-tap", "单击一下", !dbl).on_click(cx.listener(|this, _, _, cx| {
                if let Some(d) = &mut this.dialog { d.dbl = false; }
                cx.notify();
            })))
            .child(chip("hk-dbl", "双击", dbl).on_click(cx.listener(|this, _, _, cx| {
                if let Some(d) = &mut this.dialog { d.dbl = true; }
                cx.notify();
            }))))
)).child(
div().px(px(12.)).py(px(10.)).rounded(px(9.)).bg(c(CODE_BG)).border_1().border_color(c(LINE)).text_size(px(11.5)).text_color(c(INK2)).line_height(gpui::relative(1.5)).child(SharedString::from(
if d.long {
"长按遥控器时按住这个快捷键不放，松手才松开 —— 对应第三方语音输入工具的「按住说话」。注意 Fn 是硬件级的，合成不出去；单独的修饰键能发出事件，但改不了系统全局修饰位，那个工具认不认只能实测，想稳就用普通组合键。"
} else {
"短按遥控器时敲一下这个快捷键 —— 对应第三方语音输入工具的「按一下开始、再按一下结束」。注意 Fn 是硬件级的，合成不出去；单独的修饰键能发出事件，但改不了系统全局修饰位，那个工具认不认只能实测，想稳就用普通组合键。"
})),);
            }
            ActionType::VoiceDictate => {
                let st = firevibe_core::stt::auth_status();
                let ok = firevibe_core::stt::authorized();
                body = body.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .child(
                            div()
                                .px(px(12.))
                                .py(px(10.))
                                .rounded(px(9.))
                                .bg(c(CODE_BG))
                                .border_1()
                                .border_color(c(LINE))
                                .text_size(px(11.5))
                                .text_color(c(INK2))
                                .line_height(gpui::relative(1.5))
                                .child(SharedString::from(if d.long {
                                    "按住遥控器说话，松手后用系统自带的离线识别转成文字，打进当前焦点。不依赖任何第三方工具，也不会动你的系统输入设备。语言和「识别后自动回车」在设置里调。"
                                } else {
                                    "点一下开始录，再点一下结束并识别，文字打进当前焦点。不依赖任何第三方工具，也不会动你的系统输入设备。语言和「识别后自动回车」在设置里调。"
                                })),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(7.))
                                .text_size(px(11.5))
                                .text_color(if ok { c(OK) } else { c(WARN) })
                                .child(icon(if ok { "circle-check" } else { "triangle-alert" }, 14.))
                                .child(SharedString::from(format!("语音识别权限：{st}"))),
                        )
                        .when(!ok, |d| {
                            d.child(mini2("stt-auth", "请求授权").on_click(cx.listener(
                                |this, _, _, cx| {
                                    std::thread::spawn(|| {
                                        let _ = firevibe_core::stt::request_auth();
                                    });
                                    this.toast("已弹出系统授权框，允许之后回来再看");
                                    cx.notify();
                                },
                            )))
                        }),
                );
            }
            ActionType::AppleScript => {
                body = body
                    .child(
                        div()
                            .child(field_lab(l.applescript_code()))
                            .child(code_field(d)),
                    )
                    .child(
                        div()
                            .child(field_lab(l.presets()))
                            .child(preset_chips(applescript_presets(), cx)),
                    );
            }
            ActionType::OpenApp => {
                body = body
                    .child(div().child(field_lab(l.app_target())).child(code_field(d)))
                    .child(
                        div()
                            .child(field_lab(l.presets()))
                            .child(preset_chips(app_presets(), cx)),
                    );
            }
            ActionType::Shell => {
                body = body.child(div().child(field_lab(l.shell_cmd())).child(code_field(d)));
            }
            ActionType::Http => {
                let post = d.post;
                body = body
                    .child(div().child(field_lab("URL")).child(code_field(d)))
                    .child(
                        div().flex().flex_col().gap(px(6.)).child(field_lab("方法")).child(
                            div()
                                .flex()
                                .gap(px(6.))
                                .child(chip("m-get", "GET", !post).on_click(cx.listener(
                                    |t, _, _, cx| {
                                        if let Some(d) = &mut t.dialog {
                                            d.post = false;
                                        }
                                        cx.notify();
                                    },
                                )))
                                .child(chip("m-post", "POST", post).on_click(cx.listener(
                                    |t, _, _, cx| {
                                        if let Some(d) = &mut t.dialog {
                                            d.post = true;
                                        }
                                        cx.notify();
                                    },
                                ))),
                        ),
                    )
                    .when(post, |b| {
                        b.child(
                            div()
                                .child(field_lab("请求体"))
                                .child(input_box(&d.body_in)),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .gap(px(12.))
                            .child(
                                div()
                                    .flex_1()
                                    .child(field_lab("重试次数"))
                                    .child(input_box(&d.retries_in)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .child(field_lab("超时 (毫秒)"))
                                    .child(input_box(&d.timeout_in)),
                            ),
                    );
            }
            ActionType::Text => {
                body = body.child(div().child(field_lab(l.text_arg())).child(code_field(d)));
            }
            _ => {
                // 无 / 语音两种没有参数，用一行说明代替，弹窗高度不至于塌掉
                body = body.child(
                    div()
                        .px(px(12.))
                        .py(px(10.))
                        .rounded(px(9.))
                        .bg(c(CODE_BG))
                        .border_1()
                        .border_color(c(LINE))
                        .text_size(px(12.))
                        .text_color(c(INK2))
                        .child(SharedString::from(d.kind.hint())),
                );
            }
        }
        let foot = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(18.))
            .py(px(13.))
            .border_t_1()
            .border_color(c(LINE))
            .bg(c(FOOT_BG))
            .child(
                mini2_ico("dlg-test", "zap", l.test_once()).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.run_dialog_action(cx);
                    },
                )),
            )
            .child(spacer())
            .child(
                mini2("dlg-cancel", l.cancel()).on_click(cx.listener(|this, _, _, cx| {
                    this.dialog = None;
                    cx.notify();
                })),
            )
            .child(
                primary_btn("dlg-save", l.save()).on_click(cx.listener(|this, _, _, cx| {
                    this.save_dialog(cx);
                })),
            );
        crate::cards::overlay()
            .child(
                div()
                    .id("dlg")
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
                                            .child(SharedString::from(l.edit_action())),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(11.5))
                                            .text_color(c(INK3))
                                            .mt(px(3.))
                                            .child(SharedString::from(sub)),
                                    ),
                            )
                            .child(icon_btn_sm("dlg-x", "close").on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.dialog = None;
                                    cx.notify();
                                },
                            ))),
                    )
                    .child(body)
                    .child(foot),
            )
            .into_any_element()
    }
    /// 「外部语音 app」的 arg 存的是触发模式，其余类型才是输入框里的文本
    fn dialog_arg(&self, cx: &Context<Self>) -> String {
        let Some(d) = &self.dialog else {
            return String::new();
        };
        if d.kind == ActionType::VoiceHotkey {
            // 长按 = 按住；短按 = 单击或双击（看目标工具的约定）
            if d.long {
                "hold".into()
            } else if d.dbl {
                "double".into()
            } else {
                "tap".into()
            }
        } else {
            d.input.read(cx).value().to_string()
        }
    }
    /// 从弹窗当前状态构造完整 Action（含 HTTP 的方法/请求体/重试/超时）
    fn build_action(&self, cx: &Context<Self>) -> Option<Action> {
        let arg = self.dialog_arg(cx);
        let d = self.dialog.as_ref()?;
        let mut a = to_action(d, &arg);
        if d.kind == ActionType::Http {
            a.method = if d.post { "POST".into() } else { "GET".into() };
            a.body = d.body_in.read(cx).value().to_string();
            a.retries = d.retries_in.read(cx).value().trim().parse().unwrap_or(0);
            a.timeout_ms = d.timeout_in.read(cx).value().trim().parse().unwrap_or(0);
        }
        Some(a)
    }
    /// 保存弹窗
    fn save_dialog(&mut self, cx: &mut Context<Self>) {
        let Some(a) = self.build_action(cx) else { return };
        let Some(d) = &self.dialog else { return };
        let (slot, long) = (d.slot, d.long);
        {
            let mut g = self.rt.cfg.write();
            if long {
                g.profile_mut().set_long(slot, a);
            } else {
                g.profile_mut().set_short(slot, a);
            }
        }
        self.save();
        self.dialog = None;
        cx.notify();
    }
    /// 弹窗里「测试一次」：用当前编辑值直接跑，不落盘
    fn run_dialog_action(&mut self, cx: &mut Context<Self>) {
        let Some(a) = self.build_action(cx) else { return };
        let r = self.rt.run_action(&a, true);
        self.toast(if r.is_empty() { "已执行".into() } else { r });
        cx.notify();
    }
}
impl FireVibe {
    /// 热键录制框：点一下进入录制，然后按你想要的组合键。
    ///
    /// 为什么不是一排预设芯片 —— 键盘有一百多个键，列不完，字母和 `]` 这种
    /// 标点全都得能选。直接监听真实按键最省事也最准。
    fn hotkey_field(&self, d: &EditState, cx: &mut Context<Self>) -> AnyElement {
        let rec = d.recording;
        let label = if rec {
            "按下组合键…（Esc 取消）".to_string()
        } else if d.key.is_empty() {
            "点这里录制快捷键".to_string()
        } else {
            combo_text(&d.mods, &d.key)
        };
        let mut field = div()
            .id("hotkey-field")
            .track_focus(&d.focus)
            .flex()
            .items_center()
            .justify_center()
            .gap(px(8.))
            .h(px(46.))
            .rounded(px(9.))
            .border_1()
            .cursor_pointer()
            .on_click(cx.listener(|this, _, window, cx| {
                let mut err = None;
                if let Some(dd) = &mut this.dialog {
                    dd.recording = true;
                    // 起 tap 录制。失败（缺辅助功能权限）也不拦着 ——
                    // 退回窗口的 on_key_down，只是录不到被别人占用的组合。
                    match firevibe_core::hotkey::start() {
                        Ok(g) => dd.grab = Some(g),
                        Err(e) => err = Some(format!("{e:#}")),
                    }
                    let h = dd.focus.clone();
                    window.focus(&h);
                }
                if let Some(e) = err {
                    this.toast(format!("录制退回窗口模式（{e}）"));
                }
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, ev: &gpui::KeyDownEvent, _, cx| {
                let Some(dd) = &mut this.dialog else { return };
                if !dd.recording {
                    return;
                }
                let k = ev.keystroke.key.as_str();
                if k == "escape" {
                    dd.recording = false;
                    cx.notify();
                    return;
                }
                // 纯修饰键按下不算录完，继续等真正的主键
                if is_modifier_name(k) {
                    return;
                }
                let Some(name) = map_key(k) else { return };
                let m = &ev.keystroke.modifiers;
                let mut mods = Vec::new();
                if m.platform {
                    mods.push("cmd".to_string());
                }
                if m.shift {
                    mods.push("shift".to_string());
                }
                if m.alt {
                    mods.push("alt".to_string());
                }
                if m.control {
                    mods.push("ctrl".to_string());
                }
                dd.key = name;
                dd.mods = mods;
                dd.recording = false;
                cx.notify();
            }));
        field = if rec {
            field
                .bg(c(ACCENT_SOFT))
                .border_color(c(ACCENT))
                .text_color(c(ACCENT_INK))
        } else {
            field
                .bg(c(CODE_BG))
                .border_color(c(LINE_STRONG))
                .text_color(if d.key.is_empty() { c(INK3) } else { c(INK) })
                .hover(|st| st.border_color(c(INK3)))
        };
        let mut row = div().flex().items_center().gap(px(8.)).child(
            field
                .flex_1()
                .text_size(if d.key.is_empty() || rec {
                    px(12.5)
                } else {
                    px(15.)
                })
                .font_weight(w(if d.key.is_empty() || rec { 450. } else { 600. }))
                .child(SharedString::from(label)),
        );
        if !d.key.is_empty() && !rec {
            row = row.child(mini2("hk-clear", "清除").h(px(46.)).on_click(cx.listener(
                |this, _, _, cx| {
                    if let Some(dd) = &mut this.dialog {
                        dd.key.clear();
                        dd.mods.clear();
                    }
                    cx.notify();
                },
            )));
        }
        // 只按一个修饰键当热键（闪电说「自由说」默认就是右 Command）。
        // 这些没法录 —— gpui 的按键事件区分不出左右修饰键，只能列出来点。
        const MODONLY: [(&str, &str); 6] = [
            ("rightcmd", "右 ⌘"),
            ("rightoption", "右 ⌥"),
            ("rightshift", "右 ⇧"),
            ("rightcontrol", "右 ⌃"),
            ("cmd", "左 ⌘"),
            ("fn", "Fn"),
        ];
        let mut only = div().flex().flex_wrap().gap(px(6.));
        for (i, (name, label)) in MODONLY.into_iter().enumerate() {
            let on = d.key == name && d.mods.is_empty();
            only = only.child(chip_sm(("modonly", i), label, on).on_click(cx.listener(
                move |this, _, _, cx| {
                    if let Some(dd) = &mut this.dialog {
                        dd.key = name.to_string();
                        dd.mods.clear();
                        dd.recording = false;
                    }
                    cx.notify();
                },
            )));
        }
        div().flex().flex_col().gap(px(8.)).child(row).child(
div().flex().flex_col().gap(px(5.)).child(
div().text_size(px(11.)).text_color(c(INK3)).child("或者只用一个修饰键 —— 这些改不了系统全局修饰位，能不能驱动那个工具只能实测"),).child(only),).into_any_element()
    }
}
/// 组合键的可读写法
fn combo_text(mods: &[String], key: &str) -> String {
    const SYM: [(&str, &str); 4] = [("cmd", "⌘"), ("shift", "⇧"), ("alt", "⌥"), ("ctrl", "⌃")];
    let mut out = String::new();
    for (n, sym) in SYM {
        if mods.iter().any(|m| m == n) {
            out.push_str(sym);
        }
    }
    out.push_str(&pretty_key(key));
    out
}
/// 主键的显示写法
fn pretty_key(k: &str) -> String {
    match k {
        "space" => "Space".into(),
        "return" => "↩".into(),
        "tab" => "⇥".into(),
        "backspace" => "⌫".into(),
        "forwarddelete" => "⌦".into(),
        "escape" => "esc".into(),
        "up" => "↑".into(),
        "down" => "↓".into(),
        "left" => "←".into(),
        "right" => "→".into(),
        other if other.len() == 1 => other.to_uppercase(),
        other => other.to_uppercase(),
    }
}
/// gpui 的键名不完全等于注入层的键名，翻一层。
/// 主要差异：gpui 用 `enter`（注入层是 `return`）、`delete` 指前向删除。
fn map_key(k: &str) -> Option<String> {
    let mapped = match k {
        "enter" => "return",
        "delete" => "forwarddelete",
        other => other,
    };
    let names = key_names();
    names.iter().find(|n| **n == mapped).map(|n| n.to_string())
}
fn is_modifier_name(k: &str) -> bool {
    matches!(
        k,
        "cmd"
            | "command"
            | "platform"
            | "ctrl"
            | "control"
            | "alt"
            | "option"
            | "shift"
            | "fn"
            | "function"
    )
}
/// 等宽代码框。`tall` 决定是不是 62px 起高。
fn code_field(d: &EditState) -> AnyElement {
    div()
        .font_family("Menlo")
        .text_size(px(12.))
        .text_color(c(INK))
        .bg(c(CODE_BG))
        .border_1()
        .border_color(c(LINE_STRONG))
        .rounded(px(9.))
        .px(px(12.))
        .py(px(10.))
        .child(Input::new(&d.input).appearance(false))
        .into_any_element()
}

/// 和 code_field 一样的外观，但吃任意 InputState（HTTP 那几个字段用）
fn input_box(input: &gpui::Entity<gpui_component::input::InputState>) -> AnyElement {
    div()
        .font_family("Menlo")
        .text_size(px(12.))
        .text_color(c(INK))
        .bg(c(CODE_BG))
        .border_1()
        .border_color(c(LINE_STRONG))
        .rounded(px(9.))
        .px(px(12.))
        .py(px(10.))
        .child(Input::new(input).appearance(false))
        .into_any_element()
}
/// 预设芯片：点一下把 code 灌进输入框
fn preset_chips(
    list: &'static [(&'static str, &'static str)],
    cx: &mut Context<FireVibe>,
) -> AnyElement {
    let mut row = div().flex().flex_wrap().gap(px(6.));
    for (i, (name, code)) in list.iter().enumerate() {
        row = row.child(chip_sm(("preset", i), *name, false).on_click(cx.listener(
            move |this, _, window, cx| {
                if let Some(d) = &this.dialog {
                    d.input.update(cx, |s, cx| s.set_value(*code, window, cx));
                }
                cx.notify();
            },
        )));
    }
    row.into_any_element()
}
