//! 设置页 —— 通用 + 关于（版本显示 / 检查更新）。

use crate::theme::*;
use crate::widget::*;
use crate::{FireVibe, Screen};
use firevibe_core::{
    config::{config_path, Config, Lang},
    update::{UpdateStatus, VERSION},
};
use gpui::{deferred, div, prelude::*, px, AnyElement, Context, SharedString};
use gpui_component::scroll::ScrollableElement;

impl FireVibe {
    pub fn settings_page(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let l = self.l();
        let l2 = l;
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
        let cfg_path = config_path();
        let cfg_path_text = cfg_path.display().to_string();

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
                                move |this, _, _, cx| {
                                    let v = !this.rt.cfg.read().settings.launch_at_login;
                                    this.rt.cfg.write().settings.launch_at_login = v;
                                    this.save();
                                    match firevibe_core::autostart::set(v) {
                                        Ok(_) => this.toast(if v {
                                            l2.toast_login_added()
                                        } else {
                                            l2.toast_login_removed()
                                        }),
                                        Err(e) => this.toast(format!("{e}")),
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
                            .child(row_text(l.auto_switch_input(), Some(l.auto_switch_hint())))
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
                            .child(row_text(l.show_hud(), Some(l.show_hud_hint())))
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
            .child(section_lab(l.voice_to_text()).mt(px(26.)).mb(px(8.)))
            .child(
                group()
                    // 权限
                    .child(
                        group_row()
                            .child(row_icon("mic"))
                            .child(row_text(l.stt_perm(), Some(if stt_ok { l.stt_perm_ok() } else { l.stt_perm_no() })))
                            .child(if stt_ok {
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(5.))
                                    .flex_none()
                                    .text_size(px(11.5))
                                    .text_color(c(OK))
                                    .child(icon("circle-check", 14.))
                                    .child(SharedString::from(l.granted()))
                                    .into_any_element()
                            } else {
                                mini2("stt-req", l.request_perm())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        std::thread::spawn(|| {
                                            let _ = firevibe_core::stt::request_auth();
                                        });
                                        this.toast(l2.toast_stt_prompt());
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
                            .child(row_text(l.stt_lang(), Some(l.stt_lang_hint())))
                            .child(self.stt_locale_picker(&stt_locale, cx)),
                    )
                    .child(hline())
                    // 自动回车
                    .child(
                        group_row()
                            .child(row_icon("undo-2"))
                            .child(row_text(l.stt_enter(), Some(l.stt_enter_hint())))
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
            .child(section_lab(l.configuration()).mt(px(26.)).mb(px(8.)))
            .child(
                group()
                    .child(
                        group_row()
                            .child(row_icon("file"))
                            .child(row_text2(l.config_location(), cfg_path_text.clone()))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(7.))
                                    .flex_none()
                                    .child(
                                        mini2_ico(
                                            "cfg-reload",
                                            "refresh-cw",
                                            l.reload_config(),
                                        )
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.reload_config_from_disk();
                                            cx.notify();
                                        })),
                                    )
                                    .child(
                                        mini2_ico(
                                            "cfg-reveal",
                                            "folder-open",
                                            l.reveal_finder(),
                                        )
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            if let Err(e) = firevibe_core::file_dialog::reveal(
                                                &config_path(),
                                            ) {
                                                this.toast(
                                                    this.l()
                                                        .toast_config_reveal_failed(&e.to_string()),
                                                );
                                            }
                                            cx.notify();
                                        })),
                                    ),
                            ),
                    )
                    .child(hline())
                    .child(
                        group_row()
                            .child(row_icon("copy"))
                            .child(row_text(
                                l.config_manage(),
                                Some(l.config_manage_hint()),
                            ))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(7.))
                                    .flex_none()
                                    .child(
                                        mini2_ico(
                                            "cfg-import",
                                            "download",
                                            l.import_config(),
                                        )
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if this.config_import_rx.is_none() {
                                                this.config_import_rx = Some(
                                                    firevibe_core::file_dialog::pick_import(
                                                        l2.import_config_title(),
                                                        l2.import_config(),
                                                    ),
                                                );
                                            }
                                            cx.notify();
                                        })),
                                    )
                                    .child(
                                        mini2_ico(
                                            "cfg-export",
                                            "external-link",
                                            l.export_config(),
                                        )
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if this.config_export_rx.is_none() {
                                                this.config_export_rx = Some(
                                                    firevibe_core::file_dialog::pick_export(
                                                        l2.export_config_title(),
                                                        l2.export_config(),
                                                        "FireVibe-config.json",
                                                    ),
                                                );
                                            }
                                            cx.notify();
                                        })),
                                    ),
                            ),
                    ),
            )
            .child(section_lab(l.about()).mt(px(26.)).mb(px(8.)))
            .child(
                group()
                    .child(self.about_row(cx))
                    .child(self.cli_row(cx))
                    .child(self.repo_row(cx)),
            )
    }

    /// 消费原生文件面板的异步结果。放在主循环 pump 里，避免文件面板的嵌套
    /// run loop 重入 GPUI；取消选择时只清 pending，不显示错误。
    pub(crate) fn poll_config_file_io(&mut self) {
        use std::sync::mpsc::TryRecvError;

        let import_result = match self.config_import_rx.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(path)) => Some(path),
            Some(Err(TryRecvError::Disconnected)) => Some(None),
            _ => None,
        };
        if let Some(path) = import_result {
            self.config_import_rx = None;
            if let Some(path) = path {
                self.apply_imported_config(&path);
            }
        }

        let export_result = match self.config_export_rx.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(path)) => Some(path),
            Some(Err(TryRecvError::Disconnected)) => Some(None),
            _ => None,
        };
        if let Some(path) = export_result {
            self.config_export_rx = None;
            if let Some(path) = path {
                let cfg = self.rt.cfg.read().clone();
                match cfg.save_to(&path) {
                    Ok(()) => self.toast(self.l().toast_config_exported()),
                    Err(e) => {
                        self.toast(self.l().toast_config_export_failed(&format!("{e:#}")))
                    }
                }
            }
        }
    }

    fn apply_imported_config(&mut self, path: &std::path::Path) {
        let imported = match Config::load_from(path) {
            Ok(c) => c,
            Err(e) => {
                self.toast(self.l().toast_config_import_failed(&format!("{e:#}")));
                return;
            }
        };

        // 覆盖前保留一份固定位置的备份，方便手改错了回滚。
        let backup = config_path().with_file_name("config.backup.json");
        let current = self.rt.cfg.read().clone();
        if let Err(e) = current.save_to(&backup) {
            self.toast(
                self.l()
                    .toast_config_import_failed(&format!("备份当前配置失败：{e:#}")),
            );
            return;
        }
        if let Err(e) = imported.save() {
            self.toast(
                self.l()
                    .toast_config_import_failed(&format!("写入配置失败：{e:#}")),
            );
            return;
        }

        self.apply_live_config(imported);
        self.toast(self.l().toast_config_imported());
    }

    /// 重新读取应用当前使用的 config.json。严格解析，手改坏了就保留内存里的
    /// 旧配置；成功后动作、界面设置和语音链路都立即使用新值。
    fn reload_config_from_disk(&mut self) {
        let reloaded = match Config::load_from(&config_path()) {
            Ok(c) => c,
            Err(e) => {
                self.toast(self.l().toast_config_reload_failed(&format!("{e:#}")));
                return;
            }
        };
        self.apply_live_config(reloaded);
        self.toast(self.l().toast_config_reloaded());
    }

    fn apply_live_config(&mut self, config: Config) {
        let (old_device, old_gain, old_mode) = {
            let current = self.rt.cfg.read();
            (
                current.voice.device.clone(),
                current.voice.gain,
                current.voice.mode,
            )
        };
        let voice_changed = old_device != config.voice.device
            || old_gain != config.voice.gain
            || old_mode != config.voice.mode;
        let launch = config.settings.launch_at_login;

        *self.rt.cfg.write() = config;
        let _ = self.rt.sync_hid_remap();
        let _ = firevibe_core::autostart::set(launch);

        // VoiceSink 在创建时固定设备和增益；这些值变了必须丢掉旧实例，让 pump
        // 在后台按新配置重建，否则看似加载成功、实际仍沿用旧声卡参数。
        if voice_changed {
            self.rt.stop_voice();
            self.voice_ready = false;
            self.voice_rx = None;
            self.loopback_at =
                std::time::Instant::now() - std::time::Duration::from_secs(4);
        }
        self.dismiss_menus();
    }

    /// Speech.framework 在本机支持的语言下拉。它只控制内置语音识别，和上面的
    /// FireVibe 界面语言没有联动关系。
    fn stt_locale_picker(
        &self,
        current: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let lang = self.lang();
        let current_name = self
            .stt_locales
            .iter()
            .find(|locale| locale.identifier == current)
            .map(|locale| match lang {
                Lang::Zh => locale.zh_name.clone(),
                Lang::En => locale.en_name.clone(),
            })
            .unwrap_or_else(|| current.to_string());

        let head = div()
            .id("stt-locale-picker")
            .min_w(px(210.))
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(11.))
            .py(px(7.))
            .rounded(px(8.))
            .border_1()
            .border_color(c(LINE_STRONG))
            .bg(c(SURFACE))
            .cursor_pointer()
            .hover(|s| s.border_color(c(INK3)))
            .on_click(cx.listener(|this, _, _, cx| {
                if !this.just_dismissed_pub() {
                    this.stt_locale_open = !this.stt_locale_open;
                    cx.notify();
                }
            }))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .overflow_hidden()
                    .text_ellipsis()
                    .text_size(px(12.5))
                    .font_weight(w(540.))
                    .child(SharedString::from(current_name)),
            )
            .child(
                div()
                    .flex_none()
                    .text_color(c(INK3))
                    .child(icon("chevron-down", 14.)),
            );

        let mut wrap = div().relative().flex_none().child(head);
        if self.stt_locale_open {
            let mut options = div().p(px(5.)).flex().flex_col();

            for (i, locale) in self.stt_locales.iter().enumerate() {
                let identifier = locale.identifier.clone();
                let selected = identifier == current;
                let name = match lang {
                    Lang::Zh => locale.zh_name.clone(),
                    Lang::En => locale.en_name.clone(),
                };
                options = options.child(
                    div()
                        .id(("stt-locale-option", i))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .px(px(9.))
                        .py(px(7.))
                        .rounded(px(7.))
                        .cursor_pointer()
                        .hover(|s| s.bg(c(MENU_HOVER)))
                        .child(
                            div()
                                .w(px(13.))
                                .flex_none()
                                .text_color(c(ACCENT))
                                .when(selected, |d| d.child(icon("check", 13.))),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .text_size(px(12.5))
                                        .text_color(c(INK))
                                        .child(SharedString::from(name)),
                                )
                                .child(
                                    div()
                                        .text_size(px(10.5))
                                        .text_color(c(INK3))
                                        .child(SharedString::from(identifier.clone())),
                                ),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.rt.cfg.write().settings.stt_locale = identifier.clone();
                            this.save();
                            this.stt_locale_open = false;
                            cx.notify();
                        })),
                );
            }
            // 绝对定位必须放在 Scrollable 外层；反过来包时 Scrollable 自己会参与
            // flex 布局，把整行撑到 300px 高。此处靠近窗口底部，菜单向上展开。
            let menu = div()
                .absolute()
                .bottom(px(40.))
                .right(px(0.))
                .w(px(280.))
                .bg(c(SURFACE))
                .border_1()
                .border_color(c(LINE_STRONG))
                .rounded(px(10.))
                .shadow(sh3())
                .child(options.max_h(px(300.)).overflow_y_scrollbar())
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    this.dismiss_menus_pub();
                    cx.notify();
                }));
            wrap = wrap.child(deferred(menu));
        }
        wrap.into_any_element()
    }

    /// 仓库入口 —— 放「关于」最后一行。使用说明、红外码怎么抓这些都在 README 里，
    /// 界面上没必要重讲一遍，给个门就行。
    fn repo_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let l = self.l();
        group_row()
            .child(
                div()
                    .size(px(38.))
                    .flex_none()
                    .rounded(px(10.))
                    .bg(c(CODE_BG))
                    .border_1()
                    .border_color(c(LINE))
                    .text_color(c(INK2))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon("github", 18.)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(div().text_size(px(13.)).font_weight(w(560.)).child(SharedString::from(l.repo())))
                    .child(div().text_size(px(11.5)).text_color(c(INK3)).child(SharedString::from(l.repo_sub()))),
            )
            .child(
                mini2_ico("open-repo", "external-link", l.open_link()).on_click(cx.listener(
                    |_, _, _, _| {
                        let _ = std::process::Command::new("open")
                            .arg("https://github.com/tankxu/firevibe")
                            .spawn();
                    },
                )),
            )
            .into_any_element()
    }

    /// 命令行工具下载行 —— firectl（配新遥控器 / 排障用），跳 GitHub Releases。
    fn cli_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let l = self.l();
        group_row()
            .child(
                div()
                    .size(px(38.))
                    .flex_none()
                    .rounded(px(10.))
                    .bg(c(CODE_BG))
                    .border_1()
                    .border_color(c(LINE))
                    .text_color(c(INK2))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon("square-terminal", 18.)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(div().text_size(px(13.)).font_weight(w(560.)).child(SharedString::from(l.cli_tool())))
                    .child(div().text_size(px(11.5)).text_color(c(INK3)).child(SharedString::from(l.cli_tool_sub()))),
            )
            .child(
                mini2_ico("dl-cli", "download", l.download()).on_click(cx.listener(|_, _, _, _| {
                    let _ = std::process::Command::new("open")
                        .arg("https://github.com/tankxu/firevibe/releases")
                        .spawn();
                })),
            )
            .into_any_element()
    }

    /// 关于行：版本 + 更新状态 + 按钮。三种状态只显示一个。
    fn about_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let l = self.l();
        let l2 = l;
        let has = self.update.has_update();
        let (state_text, state_icon, state_col) = match &self.update {
            UpdateStatus::Available { .. } => (l.has_update(), "sparkles", ACCENT),
            UpdateStatus::UpToDate => (l.up_to_date(), "badge-check", INK3),
            UpdateStatus::Checking => (l.checking(), "loader-circle", INK3),
            UpdateStatus::NotConfigured => (l.no_endpoint(), "triangle-alert", WARN),
            UpdateStatus::Failed(_) => (l.check_failed(), "triangle-alert", WARN),
            UpdateStatus::Idle => (l.not_checked(), "refresh-cw", INK3),
        };
        let ver = match &self.update {
            UpdateStatus::Available { version, .. } => {
                l.update_available_ver(VERSION, version)
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
                            this.toast(l2.toast_no_download_url());
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
