//! 编辑操作弹窗。改的是临时状态，点保存才写回配置。

use crate::cards::{new_input, to_action, new_line_input};
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
        // 数字用真单行 —— new_input 是 auto_grow 的，装一个「0」也会占掉多行的高度
        let retries_in = new_line_input(&a.retries.to_string(), window, cx);
        let timeout_in = new_line_input(
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
            // 已有的码里带 label 就填进去，方便改
            ir_name: new_line_input(
                &firevibe_core::ir::IrCode::parse(&a.arg)
                    .map(|c| c.label)
                    .unwrap_or_default(),
                window,
                cx,
            ),
            ir_q: new_line_input("", window, cx),
            ir_pick: None,
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
            l.card_title(d.slot),
            if d.long {
                l.long_press()
            } else {
                l.short_press()
            },
            key_id
        );
        // PTT 遥控器 = 只在**物理麦克风键**按住期间出音频。于是有两处语音动作是摆设：
        //   · 麦克风键的**短按**槽 —— 点一下就松手，一帧都不出
        //   · **其它任何键** —— 音频跟那些键根本没关系，按了也不会出声
        // 两处都不给选，各给一行说明。
        let is_mic = d.slot == firevibe_core::layout::Slot::Mic;
        let is_ptt = self.rt.cfg.read().settings.mic_model.is_ptt();
        let ptt_short_mic = is_ptt && is_mic && !d.long;
        let ptt_other_key = is_ptt && !is_mic;
        // 其它按键：静默隐藏语音动作，不挂横幅（音频跟那些键本来就没关系，
        // 用户不会去那儿找语音，解释一遍反而是噪音）。
        // 麦克风键短按：四个语音动作**照常列出**，选中时用一行说明代替配置项 ——
        // 那才是用户真会去点、且需要被告知的地方。
        let hide_voice = ptt_other_key;
        // 仿品的红外是烧进它的键位表的：一个 scanId 一条码，只有四个键有 scanId，
        // 而且表里没有「长按」这回事。挂不上的地方干脆不给选 —— 让用户配一个
        // 永远不会响的动作，比不给配更糟。
        let ir_burnable = firevibe_core::irtable::supports_ir(d.slot) && !d.long;
        let hide_ir = is_ptt && !ir_burnable;

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
            if hide_ir && k == ActionType::IrBlast {
                continue;
            }
            if hide_voice
                && matches!(
                    k,
                    ActionType::VoicePtt
                        | ActionType::VoiceToggle
                        | ActionType::VoiceDictate
                        | ActionType::VoiceHotkey
                        | ActionType::Record
                )
            {
                continue;
            }
            // 用麦克风的那几个动作挂个琥珀色麦克风图标 —— 十个纯文字选项里
            // 挑不出来哪些跟语音有关，图标比高亮好认。
            let voiced = matches!(
                k,
                ActionType::VoicePtt
                    | ActionType::VoiceHotkey
                    | ActionType::VoiceDictate
                    | ActionType::VoiceToggle
                    | ActionType::Record
            );
            let el = if voiced {
                chip_icon(("kind", k as usize), k.label(), k == d.kind, "mic", MIC_MARK)
            } else {
                chip(("kind", k as usize), k.label(), k == d.kind)
            };
            types = types.child(el.on_click(
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
        // 内容区自己滚：动作类型不同高度差很多（红外那种带说明+码库+长码预览+校验，
        // 能顶到屏幕外）。头部和底部按钮固定，只有中间滚 —— 否则「保存」会被挤出可视区。
        // `min_h(0)` 不能省：flex 子项默认 min-height:auto，不给 0 它不肯被压缩，
        // max_h 就形同虚设。
        let mut body = div()
            .id("dlg-body")
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .gap(px(16.))
            .px(px(18.))
            .pt(px(2.))
            .pb(px(18.))
            .child(div().child(field_lab(l.action_type())).child(types))
;
        // 麦克风键的短按上，语音动作在 PTT 遥控器上是摆设（点一下就松手，一帧都不出）。
        // 保留选项让用户找得到，但把配置区换成一行说明，指向长按。
        let voice_kind = matches!(
            d.kind,
            ActionType::VoicePtt
                | ActionType::VoiceToggle
                | ActionType::VoiceDictate
                | ActionType::VoiceHotkey
                | ActionType::Record
        );
        // 命中就把参数区当成「无」来渲染，兜底分支里换成一行说明
        let ptt_note = ptt_short_mic && voice_kind;

        // 参数区，按类型不同
        match if ptt_note { ActionType::None } else { d.kind } {
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
                // （原来这儿有一段解释「这颗键会在硬件层变成修饰键」的说明 ——
                //   属于讲原理，不是当下要做的决定，挪去 README 了。）
                let dbl = d.dbl;
                body = body.child(
div().child(field_lab(l.hotkey())).child(self.hotkey_field(d, cx))).when(!d.long, |b| b.child(
    div().flex().flex_col().gap(px(6.))
        .child(field_lab(l.trigger_mode()))
        .child(div().flex().gap(px(6.))
            .child(chip("hk-tap", l.single_tap(), !dbl).on_click(cx.listener(|this, _, _, cx| {
                if let Some(d) = &mut this.dialog { d.dbl = false; }
                cx.notify();
            })))
            .child(chip("hk-dbl", l.double_tap(), dbl).on_click(cx.listener(|this, _, _, cx| {
                if let Some(d) = &mut this.dialog { d.dbl = true; }
                cx.notify();
            }))))
)).child(
div().px(px(12.)).py(px(10.)).rounded(px(9.)).bg(c(CODE_BG)).border_1().border_color(c(LINE)).text_size(px(11.5)).text_color(c(INK2)).line_height(gpui::relative(1.5)).child(SharedString::from(
if d.long {
l.hotkey_hold_hint()
} else {
l.hotkey_tap_hint()
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
                                    l.dictate_hold_hint()
                                } else {
                                    l.dictate_tap_hint()
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
                                .child(SharedString::from(l.stt_perm_label(st))),
                        )
                        .when(!ok, |d| {
                            d.child(mini2("stt-auth", l.request_perm()).on_click(cx.listener(
                                |this, _, _, cx| {
                                    std::thread::spawn(|| {
                                        let _ = firevibe_core::stt::request_auth();
                                    });
                                    this.toast(this.l().toast_stt_prompt2());
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
                    .child(div().child(field_lab(l.app_target())).child(text_field(d)))
                    .child(
                        div()
                            .child(field_lab(l.presets()))
                            .child(preset_chips(app_presets(), cx)),
                    );
            }
            ActionType::Shell => {
                body = body.child(div().child(field_lab(l.shell_cmd())).child(code_field(d)));
            }
            ActionType::IrBlast => {
                // 边填边校验：码对不对、多长、几段，立刻显示。不用点保存才知道。
                let txt = d.input.read(cx).value().to_string();
                // 空着不报错 —— 刚点进来还没开始填就先甩个警告，纯属添堵。
                // 「没填」这件事留到保存时再拦（见 save_dialog）。
                // 两种遥控器的红外语义是反的，说法也得两套：
                //   原厂 —— app 现场发射，「按下那个键就打出去」
                //   仿品 —— 码烧进遥控器，按实体键它自己发（电脑关着也发）
                let tail = if is_ptt { l.ir_clone_note() } else { l.ir_not_wired() };
                let verdict = if txt.trim().is_empty() {
                    None
                } else {
                    Some(match firevibe_core::ir::IrCode::parse(&txt) {
                        // 解析通过不等于能用：仿品对单条码有脉冲数上限，
                        // 超了必须**在这儿**就变成警告 —— 否则用户看到绿色的
                        // 「码没问题」，到保存才被拦，白填一趟。
                        Ok(c) => match is_ptt.then(|| firevibe_core::irtable::check_code(&c)) {
                            Some(Err(e)) => (false, format!("{}\n{e}", c.summary())),
                            _ => (true, format!("{}\n{}", c.summary(), tail)),
                        },
                        Err(e) => (false, e),
                    })
                };
                let limits = if is_ptt {
                    format!("{}\n{}", l.ir_clone_slots(), l.ir_clone_budget())
                } else {
                    l.ir_limits().to_string()
                };
                body = body
                    .child(hint_box(format!("{}\n{}", l.ir_help(), limits)))
                    .child(ir_library(d, l, is_ptt, cx))
                    .child(
                        div()
                            .child(field_lab(l.ir_name_label()))
                            .child(input_box(&d.ir_name)),
                    )
                    .child(div().child(field_lab(l.ir_code_label())).child(code_field(d)))
                    .when_some(verdict, |b, (ok, msg)| {
                        b.child(note_box(msg, if ok { Note::Ok } else { Note::Warn }))
                    })
                    ;
            }
            ActionType::Http => {
                let post = d.post;
                body = body
                    .child(div().child(field_lab("URL")).child(code_field(d)))
                    .child(
                        div().flex().flex_col().gap(px(6.)).child(field_lab(l.http_method())).child(
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
                                .child(field_lab(l.http_body()))
                                .child(input_box(&d.body_in)),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .gap(px(12.))
                            .child(
                                div()
                                    .w(px(110.))
                                    .flex_none()
                                    .child(field_lab(l.http_retries()))
                                    .child(num_box(&d.retries_in)),
                            )
                            .child(
                                div()
                                    .w(px(140.))
                                    .flex_none()
                                    .child(field_lab(l.http_timeout()))
                                    .child(num_box(&d.timeout_in)),
                            ),
                    );
            }
            ActionType::Text => {
                body = body.child(div().child(field_lab(l.text_arg())).child(text_field(d)));
            }
            _ if ptt_note => {
                body = body.child(note_box(l.ptt_short_note(), Note::Warn));
            }
            _ => {
                // 无 / 语音两种没有参数，用一行说明代替，弹窗高度不至于塌掉
                body = body.child(hint_box(d.kind.hint()));
                // 「按住说话」只灌音频、不发任何按键。想让第三方工具跟着一起开录，
                // 得用「第三方语音输入」—— 这两个的差别不写出来很容易选错。
                if d.kind == ActionType::VoicePtt {
                    body = body.child(
                        div()
                            .px(px(10.))
                            .py(px(8.))
                            .rounded(px(R))
                            .border_1()
                            .border_color(c(ACCENT))
                            .bg(c(ACCENT_SOFT))
                            .text_size(px(11.5))
                            .text_color(c(ACCENT_INK))
                            .line_height(gpui::relative(1.5))
                            .child(SharedString::from(l.ptt_vs_hotkey_note())),
                    );
                }
            }
        }
        let foot = div()
            .flex()
            .items_center()
            .flex_shrink_0()
            .gap(px(8.))
            .px(px(18.))
            .py(px(13.))
            .border_t_1()
            .border_color(c(LINE))
            .bg(c(FOOT_BG))
            // 自己收圆角：gpui 的 ContentMask 是矩形的，父容器的 overflow_hidden
            // 不按圆角裁，直角的按钮栏会从弹窗圆角里探出来。14 减去 1px 边框。
            .rounded_b(px(13.))
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
                    .flex()
                    .flex_col()
                    .w(px(520.))
                    // 别顶满：留一圈才看得出是浮层，也不会贴着窗口边
                    .max_h(gpui::relative(0.88))
                    .bg(c(SURFACE))
                    .border_1()
                    .border_color(c(LINE))
                    .rounded(px(14.))
                    // 底部那条按钮栏是直角的，不裁就会从圆角里探出来
                    .overflow_hidden()
                    .shadow(sh3())
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .flex_shrink_0()
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
        // 红外：把名称写进码里再存成 JSON。
        // 必须转成 JSON —— Pronto 和裸时长数组都表达不了 label，
        // 用户填了名字却存回原格式的话，下次打开就没了。
        if d.kind == ActionType::IrBlast {
            let name = d.ir_name.read(cx).value().trim().to_string();
            if let Ok(mut code) = firevibe_core::ir::IrCode::parse(&a.arg) {
                if code.label != name {
                    code.label = name;
                    a.arg = code.to_json();
                }
            }
        }
        Some(a)
    }
    /// 保存弹窗
    fn save_dialog(&mut self, cx: &mut Context<Self>) {
        let Some(a) = self.build_action(cx) else { return };
        // 红外码在这儿才拦：编辑时空着不报错，但空着/写错了不给存 —— 存进去
        // 也是个按下去只会报错的死动作。
        if a.kind == ActionType::IrBlast {
            match firevibe_core::ir::IrCode::parse(&a.arg) {
                Err(e) => {
                    self.toast(e);
                    return;
                }
                // 仿品放不下的码不给存 —— 存进去只会在同步时失败，
                // 而更早的版本会直接写进遥控器，那个键从此发一条乱码
                Ok(c) if self.rt.cfg.read().settings.mic_model.is_ptt() => {
                    if let Err(e) = firevibe_core::irtable::check_code(&c) {
                        self.toast(e);
                        return;
                    }
                }
                Ok(_) => {}
            }
        }
        let is_ir = a.kind == ActionType::IrBlast;
        let Some(d) = &self.dialog else { return };
        let (slot, long) = (d.slot, d.long);
        // 原来是红外、现在改成别的 —— 也得重写表，不然遥控器里还烧着旧码
        let was_ir = self
            .rt
            .cfg
            .read()
            .profile()
            .action(slot, long)
            .is_some_and(|old| old.kind == ActionType::IrBlast);
        {
            let mut g = self.rt.cfg.write();
            if long {
                g.profile_mut().set_long(slot, a);
            } else {
                g.profile_mut().set_short(slot, a);
            }
        }
        self.save();
        // 仿品遥控器：红外码要写进它的键位表才生效，但**不再自动写**
        //（写一次十几秒、GATT 会话容易和使用撞车）。这里只点亮顶栏的
        // 「写入红外」提示，写不写、什么时候写由用户点。删动作也一样 ——
        // 四行一起写，点一次写入才会把遥控器里的旧码清掉。
        if is_ir || was_ir {
            self.refresh_ir_pending();
            if self.ir_pending {
                let m = self.l().ir_pending_hint().to_string();
                self.toast(m);
            }
        }
        self.dialog = None;
        cx.notify();
    }
    /// 弹窗里「测试一次」：用当前编辑值直接跑，不落盘
    fn run_dialog_action(&mut self, cx: &mut Context<Self>) {
        let Some(a) = self.build_action(cx) else { return };
        // 仿品的红外电脑指挥不动（它自己按键才发），别让「测试一次」假装成功
        if a.kind == ActionType::IrBlast && self.rt.cfg.read().settings.mic_model.is_ptt() {
            self.toast(self.l().ir_clone_untestable().to_string());
            cx.notify();
            return;
        }
        let slot = self.dialog.as_ref().map(|d| d.slot);
        let r = self.rt.run_action_at(&a, true, slot);
        self.toast(if r.is_empty() { self.l().toast_executed().into() } else { r });
        cx.notify();
    }
}
impl FireVibe {
    /// 热键录制框：点一下进入录制，然后按你想要的组合键。
    ///
    /// 为什么不是一排预设芯片 —— 键盘有一百多个键，列不完，字母和 `]` 这种
    /// 标点全都得能选。直接监听真实按键最省事也最准。
    fn hotkey_field(&self, d: &EditState, cx: &mut Context<Self>) -> AnyElement {
        let l = self.l();
        let rec = d.recording;
        let label = if rec {
            l.hotkey_recording().to_string()
        } else if d.key.is_empty() {
            l.hotkey_click_record().to_string()
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
                    let m = this.l().toast_record_window_mode(&e);
                    this.toast(m);
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
            row = row.child(mini2("hk-clear", l.clear()).h(px(46.)).on_click(cx.listener(
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
        // side: 1=右, 0=左, 2=无（Fn）—— 左右前缀按语言翻译
        const MODONLY: [(&str, &str, u8); 6] = [
            ("rightcmd", "⌘", 1),
            ("rightoption", "⌥", 1),
            ("rightshift", "⇧", 1),
            ("rightcontrol", "⌃", 1),
            ("cmd", "⌘", 0),
            ("fn", "Fn", 2),
        ];
        let mut only = div().flex().flex_wrap().gap(px(6.));
        for (i, (name, sym, side)) in MODONLY.into_iter().enumerate() {
            let on = d.key == name && d.mods.is_empty();
            let label = match side {
                1 => format!("{} {sym}", l.mod_right()),
                0 => format!("{} {sym}", l.mod_left()),
                _ => sym.to_string(),
            };
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
div().text_size(px(11.)).text_color(c(INK3)).child(l.single_modifier_hint()),).child(only),).into_any_element()
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
/// 弹窗里那种成块的说明/提示。三种语气共用同一套几何，只换配色 ——
/// 以前各处手抄，padding 和圆角都对不齐。
#[derive(Clone, Copy, PartialEq)]
enum Note {
    /// 中性：解释这个动作是干嘛的
    Hint,
    /// 警告：这么配不会生效
    Warn,
    /// 成功：校验通过
    Ok,
}

fn note_box(text: impl Into<SharedString>, kind: Note) -> AnyElement {
    let (bg, line, ink) = match kind {
        Note::Hint => (CODE_BG, LINE, INK2),
        Note::Warn => (WARN_BG, WARN_LINE, INK2),
        Note::Ok => (OK_SOFT, OK, OK),
    };
    div()
        .px(px(12.))
        .py(px(10.))
        .rounded(px(9.))
        .bg(c(bg))
        .border_1()
        .border_color(c(line))
        .text_size(px(12.))
        .text_color(c(ink))
        .line_height(gpui::relative(1.5))
        .child(text.into())
        .into_any_element()
}

fn hint_box(text: impl Into<SharedString>) -> AnyElement {
    note_box(text, Note::Hint)
}

/// 内置红外码库的搜索区：搜品牌/型号 → 选设备 → 点按键，码直接灌进上面的输入框。
///
/// 为什么值得做：用户手上大概率只有「我家是大金空调」这点信息，让他去网上找
/// Pronto 码、或者搬个 ESP32 来抓，门槛太高。库里 1400 多个设备两万多条码，
/// 搜一下点一下就完事。
/// 码库：搜设备 → 挑按键 → 填进输入框。
///
/// `ptt` 为真时把放不进仿品遥控器的按键标出来 —— 让人挑完才知道选不了，
/// 比一开始就说清楚糟糕得多。标记而不是隐藏：那条码确实存在，
/// 只是得换原厂遥控器发。
fn ir_library(
    d: &EditState,
    l: crate::i18n::L,
    ptt: bool,
    cx: &mut Context<FireVibe>,
) -> AnyElement {
    let q = d.ir_q.read(cx).value().to_string();

    let mut col = div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(field_lab(l.ir_lib_label()))
        .child(
            div()
                .flex()
                .items_center()
                .h(px(32.))
                .px(px(6.))
                .rounded(px(R_SM))
                .bg(c(SURFACE))
                .border_1()
                .border_color(c(LINE_STRONG))
                .text_size(px(13.))
                .text_color(c(INK))
                .child(Input::new(&d.ir_q).appearance(false)),
        );

    // 选中某个设备后列它的按键；否则列搜索结果
    if let Some(idx) = d.ir_pick {
        let btns = firevibe_core::irdb::buttons_of(idx);
        let mut row = div().flex().flex_wrap().gap(px(5.));
        row = row.child(
            chip_sm("ir-back", l.ir_lib_back(), false).on_click(cx.listener(|this, _, _, cx| {
                if let Some(d) = &mut this.dialog {
                    d.ir_pick = None;
                }
                cx.notify();
            })),
        );
        let pulses = firevibe_core::irdb::pulses_of(idx);
        for (i, (name, src)) in btns.into_iter().enumerate() {
            // 合成出来的标一下来源，万一发不出去用户知道该去抓真码
            let mut label = if src == "raw" { name } else { format!("{name} ·{src}") };
            if ptt && pulses.get(i).is_some_and(|n| *n > firevibe_core::irtable::MAX_PULSES) {
                label = format!("{label} ·{}", l.ir_lib_toolong());
            }
            row = row.child(chip_sm(("ir-btn", i), label, false).on_click(cx.listener(
                move |this, _, window, cx| {
                    let Some(code) = firevibe_core::irdb::code_of(idx, i) else { return };
                    let text = code.to_json();
                    if let Some(d) = &this.dialog {
                        d.input.update(cx, |s, cx| s.set_value(&text, window, cx));
                        // 名称框跟着填上「品牌 型号 · 按键」，不然用户还得自己抄一遍
                        let name = code.label.clone();
                        d.ir_name.update(cx, |s, cx| s.set_value(&name, window, cx));
                    }
                    cx.notify();
                },
            )));
        }
        col = col.child(row);
    } else if !q.trim().is_empty() {
        let hits = firevibe_core::irdb::search(&q, 8);
        if hits.is_empty() {
            col = col.child(
                div()
                    .text_size(px(11.5))
                    .text_color(c(INK3))
                    .child(SharedString::from(l.ir_lib_none())),
            );
        }
        for h in hits {
            let idx = h.idx;
            col = col.child(
                div()
                    .id(("ir-hit", idx))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(8.))
                    .py(px(6.))
                    .rounded(px(R_SM))
                    .cursor_pointer()
                    .hover(|s| s.bg(c(HOVER)))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .text_size(px(12.5))
                            .text_color(c(INK))
                            .child(SharedString::from(format!("{} {}", h.brand, h.model))),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(11.))
                            .text_color(c(INK3))
                            .child(SharedString::from(format!("{} · {} 键", h.category, h.buttons))),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(d) = &mut this.dialog {
                            d.ir_pick = Some(idx);
                        }
                        cx.notify();
                    })),
            );
        }
    }
    col.into_any_element()
}

/// 代码类输入框：AppleScript / 命令 / URL / 红外码 —— 等宽字体 + 代码底色。
/// padding 收到 6：这些内容常常很长，多一分留白就少一分能看见的字符。
fn code_field(d: &EditState) -> AnyElement {
    div()
        .font_family("Menlo")
        .text_size(px(12.))
        .text_color(c(INK))
        .bg(c(CODE_BG))
        .border_1()
        .border_color(c(LINE_STRONG))
        .rounded(px(R_SM))
        .px(px(6.))
        .py(px(6.))
        .child(Input::new(&d.input).appearance(false))
        .into_any_element()
}

/// 普通文本输入框：要输入的文字 / 应用名 —— 用界面字体和常规底色，
/// 一眼能和上面那种「这里填代码」区分开。
fn text_field(d: &EditState) -> AnyElement {
    div()
        .text_size(px(13.))
        .text_color(c(INK))
        .bg(c(SURFACE))
        .border_1()
        .border_color(c(LINE_STRONG))
        .rounded(px(R_SM))
        .px(px(6.))
        .py(px(6.))
        .child(Input::new(&d.input).appearance(false))
        .into_any_element()
}

/// 和 code_field 一样的外观，但吃任意 InputState（HTTP 那几个字段用）
/// 长文本输入框（POST 请求体那种，内容通常是 JSON）。数字用 `num_box`，别混。
fn input_box(input: &gpui::Entity<gpui_component::input::InputState>) -> AnyElement {
    div()
        .font_family("Menlo")
        .text_size(px(12.))
        .text_color(c(INK))
        .bg(c(CODE_BG))
        .border_1()
        .border_color(c(LINE_STRONG))
        .rounded(px(R_SM))
        .px(px(6.))
        .py(px(6.))
        .child(Input::new(input).appearance(false))
        .into_any_element()
}

/// 数字输入框（重试次数、超时那种）。
///
/// ⚠️ 别拿 `input_box` 凑合 —— 那是给长文本的（`py(10)`），
/// 装一个「0」会撑得和 URL 框一样高。这里固定 32 高，和旁边按钮一条轴。
fn num_box(input: &gpui::Entity<gpui_component::input::InputState>) -> AnyElement {
    div()
        .h(px(32.))
        .flex()
        .items_center()
        .font_family("Menlo")
        .text_size(px(12.))
        .text_color(c(INK))
        .bg(c(CODE_BG))
        .border_1()
        .border_color(c(LINE_STRONG))
        .rounded(px(R_SM))
        .px(px(6.))
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
