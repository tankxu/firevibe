//! 设置页 —— 通用 + 关于（版本显示 / 检查更新）。

use crate::theme::*;
use crate::widget::*;
use crate::{FireVibe, Screen};
use firevibe_core::{
    config::Lang,
    update::{UpdateStatus, VERSION},
};
use gpui::{div, prelude::*, px, AnyElement, Context, SharedString};

impl FireVibe {
    pub fn settings_page(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let l = self.l();
        let cfg = self.rt.cfg.read();
        let launch = cfg.settings.launch_at_login;
        let lang = cfg.settings.lang;
        let long_ms = cfg.settings.long_press_ms;
        let auto_in = cfg.settings.auto_switch_input;
        let hud = cfg.settings.show_level_hud;
        let stt_locale = cfg.settings.stt_locale.clone();
        let stt_enter = cfg.settings.stt_auto_enter;
        drop(cfg);
        let stt_ok = firevibe_core::stt::authorized();
        let stt_st = firevibe_core::stt::auth_status();

        div()
            .max_w(px(620.))
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(11.))
                    .mb(px(4.))
                    .child(icon_btn_sm("set-back", "chevron-left").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.screen = Screen::Main;
                            cx.notify();
                        },
                    )))
                    .child(
                        div()
                            .text_size(px(22.))
                            .font_weight(w(640.))
                            .child(SharedString::from(l.settings())),
                    ),
            )
            .child(section_lab(l.general()).mt(px(26.)).mb(px(8.)))
            .child(
                group()
                    // 开机启动
                    .child(
                        group_row()
                            .child(row_icon("rocket"))
                            .child(row_text(l.launch_at_login(), Some(l.launch_hint())))
                            .child(switch_ui("sw-launch", launch).on_click(cx.listener(
                                |this, _, _, cx| {
                                    let v = !this.rt.cfg.read().settings.launch_at_login;
                                    this.rt.cfg.write().settings.launch_at_login = v;
                                    this.save();
                                    match firevibe_core::autostart::set(v) {
                                        Ok(_) => this.toast(if v {
                                            "已加入登录启动项"
                                        } else {
                                            "已移出登录启动项"
                                        }),
                                        Err(e) => this.toast(format!("设置失败: {e}")),
                                    }
                                    cx.notify();
                                },
                            ))),
                    )
                    .child(hline())
                    // 语言
                    .child(
                        group_row()
                            .child(row_icon("globe"))
                            .child(row_text(l.language(), None))
                            .child(
                                seg_wrap()
                                    .child(seg_item("lg-zh", "中文", lang == Lang::Zh).on_click(
                                        cx.listener(|this, _, _, cx| {
                                            this.rt.cfg.write().settings.lang = Lang::Zh;
                                            this.save();
                                            cx.notify();
                                        }),
                                    ))
                                    .child(
                                        seg_item("lg-en", "English", lang == Lang::En).on_click(
                                            cx.listener(|this, _, _, cx| {
                                                this.rt.cfg.write().settings.lang = Lang::En;
                                                this.save();
                                                cx.notify();
                                            }),
                                        ),
                                    ),
                            ),
                    )
                    .child(hline())
                    // 说话时自动切系统输入设备
                    .child(
                        group_row()
                            .child(row_icon("mic"))
                            .child(row_text(
                                "说话时自动切输入设备",
                                Some("按下说话键把系统默认输入切到虚拟声卡，说完切回原来的（实测 3~13ms）"),
                            ))
                            .child(switch_ui("sw-autoin", auto_in).on_click(cx.listener(
                                |this, _, _, cx| {
                                    let v = !this.rt.cfg.read().settings.auto_switch_input;
                                    this.rt.cfg.write().settings.auto_switch_input = v;
                                    this.save();
                                    if !v {
                                        // 关掉时把可能已经切走的设备还原回来
                                        this.rt.restore_input();
                                    }
                                    cx.notify();
                                },
                            ))),
                    )
                    .child(hline())
                    // 说话时的悬浮电平条
                    .child(
                        group_row()
                            .child(row_icon("mic"))
                            .child(row_text(
                                "说话时显示电平条",
                                Some("屏幕底部浮出一条电平，让你知道确实在收音"),
                            ))
                            .child(switch_ui("sw-hud", hud).on_click(cx.listener(
                                |this, _, _, cx| {
                                    let v = !this.rt.cfg.read().settings.show_level_hud;
                                    this.rt.cfg.write().settings.show_level_hud = v;
                                    this.save();
                                    cx.notify();
                                },
                            ))),
                    )
                    .child(hline())
                    // 长按阈值
                    .child(
                        group_row()
                            .child(row_icon("timer"))
                            .child(row_text(l.long_ms(), Some(l.long_ms_hint())))
                            .child(
                                stepper_wrap()
                                    .child(stepper_btn("ms-dec", "−").on_click(cx.listener(
                                        |this, _, _, cx| {
                                            let mut g = this.rt.cfg.write();
                                            g.settings.long_press_ms =
                                                g.settings.long_press_ms.saturating_sub(50).max(150);
                                            drop(g);
                                            this.save();
                                            cx.notify();
                                        },
                                    )))
                                    .child(
                                        div()
                                            .min_w(px(34.))
                                            .text_center()
                                            .text_size(px(12.5))
                                            .child(SharedString::from(long_ms.to_string())),
                                    )
                                    .child(stepper_btn("ms-inc", "+").on_click(cx.listener(
                                        |this, _, _, cx| {
                                            let mut g = this.rt.cfg.write();
                                            g.settings.long_press_ms =
                                                (g.settings.long_press_ms + 50).min(1200);
                                            drop(g);
                                            this.save();
                                            cx.notify();
                                        },
                                    )))
                                    .child(
                                        div()
                                            .ml(px(2.))
                                            .text_size(px(11.))
                                            .text_color(c(INK3))
                                            .child("ms"),
                                    ),
                            ),
                    ),
            )
            .child(section_lab("语音转文字").mt(px(26.)).mb(px(8.)))
            .child(
                group()
                    // 权限
                    .child(
                        group_row()
                            .child(row_icon("mic"))
                            .child(row_text(
                                "语音识别权限",
                                Some(if stt_ok {
                                    "已授权，可以用「语音转文字」动作"
                                } else {
                                    "系统自带的离线识别需要一次授权"
                                }),
                            ))
                            .child(if stt_ok {
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(5.))
                                    .flex_none()
                                    .text_size(px(11.5))
                                    .text_color(c(OK))
                                    .child(icon("circle-check", 14.))
                                    .child(SharedString::from(stt_st))
                                    .into_any_element()
                            } else {
                                mini2("stt-req", "请求授权")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        std::thread::spawn(|| {
                                            let _ = firevibe_core::stt::request_auth();
                                        });
                                        this.toast("已弹出系统授权框");
                                        cx.notify();
                                    }))
                                    .into_any_element()
                            }),
                    )
                    .child(hline())
                    // 识别语言
                    .child(
                        group_row()
                            .child(row_icon("globe"))
                            .child(row_text("识别语言", Some("BCP-47，比如 zh-CN / en-US")))
                            .child(
                                seg_wrap()
                                    .child(
                                        seg_item("stt-zh", "中文", stt_locale == "zh-CN").on_click(
                                            cx.listener(|this, _, _, cx| {
                                                this.rt.cfg.write().settings.stt_locale =
                                                    "zh-CN".into();
                                                this.save();
                                                cx.notify();
                                            }),
                                        ),
                                    )
                                    .child(
                                        seg_item("stt-en", "English", stt_locale == "en-US")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.rt.cfg.write().settings.stt_locale =
                                                    "en-US".into();
                                                this.save();
                                                cx.notify();
                                            })),
                                    ),
                            ),
                    )
                    .child(hline())
                    // 自动回车
                    .child(
                        group_row()
                            .child(row_icon("undo-2"))
                            .child(row_text(
                                "识别后自动回车",
                                Some("在 agent 里就是说完直接发出去"),
                            ))
                            .child(switch_ui("sw-enter", stt_enter).on_click(cx.listener(
                                |this, _, _, cx| {
                                    let v = !this.rt.cfg.read().settings.stt_auto_enter;
                                    this.rt.cfg.write().settings.stt_auto_enter = v;
                                    this.save();
                                    cx.notify();
                                },
                            ))),
                    ),
            )
            .child(section_lab(l.about()).mt(px(26.)).mb(px(8.)))
            .child(group().child(self.about_row(cx)))
    }

    /// 关于行：版本 + 更新状态 + 按钮。三种状态只显示一个。
    fn about_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let l = self.l();
        let has = self.update.has_update();
        let (state_text, state_icon, state_col) = match &self.update {
            UpdateStatus::Available { .. } => (l.has_update(), "sparkles", ACCENT),
            UpdateStatus::UpToDate => (l.up_to_date(), "badge-check", INK3),
            UpdateStatus::Checking => (l.checking(), "loader-circle", INK3),
            UpdateStatus::NotConfigured => (l.no_endpoint(), "triangle-alert", WARN),
            UpdateStatus::Failed(_) => ("检查失败", "triangle-alert", WARN),
            UpdateStatus::Idle => (l.not_checked(), "refresh-cw", INK3),
        };
        let ver = match &self.update {
            UpdateStatus::Available { version, .. } => {
                format!("{VERSION} → {version} 可更新")
            }
            _ => format!("{VERSION} · Rust + GPUI"),
        };

        let mut row = group_row()
            .when(has, |d| d.bg(c(ALT_ROW)))
            .child(
                div()
                    .size(px(38.))
                    .flex_none()
                    .rounded(px(10.))
                    .bg(grad(160., BADGE_MIC.0, BADGE_MIC.1))
                    .text_color(c(SURFACE))
                    .shadow(sh1())
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon("mic", 18.)),
            )
            .child(
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
                            .child("FireVibe"),
                    )
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(c(INK3))
                            .child(SharedString::from(ver)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .flex_none()
                    .text_size(px(11.5))
                    .text_color(c(state_col))
                    .when(has, |d| d.font_weight(w(560.)))
                    .child(icon(state_icon, 14.))
                    .child(SharedString::from(state_text)),
            );

        row = if has {
            let url = match &self.update {
                UpdateStatus::Available { url, .. } => url.clone(),
                _ => String::new(),
            };
            row.child(
                primary_btn_sm_ico("do-upd", "download", l.do_update())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if url.is_empty() {
                            this.toast("清单里没给下载地址");
                        } else {
                            let _ = std::process::Command::new("open").arg(&url).spawn();
                        }
                        cx.notify();
                    })),
            )
        } else {
            row.child(
                mini2_ico("chk-upd", "refresh-cw", l.check_update())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.check_update();
                        cx.notify();
                    })),
            )
        };
        row.into_any_element()
    }
}

