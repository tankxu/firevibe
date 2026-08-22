//! 「遥控器适配」面板 —— 给拿到另一款遥控器的人用，两步走完就能用上。
//!
//! 为什么需要它：我们按 VID/PID 打开设备，标识对不上就完全看不到；
//! 就算打开了，按键的 HID usage 也可能和实测那款不同。这两件事以前
//! 只能改代码或用命令行，现在做进界面。

use crate::theme::*;
use crate::widget::*;
use crate::{FireVibe, Screen};
use firevibe_core::layout::Slot;
use gpui::{div, prelude::*, px, relative, AnyElement, Context, SharedString};

impl FireVibe {
    pub fn adapt_page(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (cur_vid, cur_pid) = self.rt.cfg.read().device_ids();
        let connected = self.rt.status.connected.load(std::sync::atomic::Ordering::Relaxed);

        div()
            .max_w(px(680.))
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(11.))
                    .mb(px(4.))
                    .child(icon_btn_sm("adapt-back", "chevron-left").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.rt.set_learn(false);
                            this.mapping = None;
                            this.screen = Screen::Main;
                            cx.notify();
                        },
                    )))
                    .child(
                        div()
                            .text_size(px(22.))
                            .font_weight(w(640.))
                            .child("遥控器适配"),
                    ),
            )
            .child(
                div()
                    .text_size(px(12.5))
                    .text_color(c(INK3))
                    .line_height(relative(1.6))
                    .mt(px(6.))
                    .child(
                        "如果你的遥控器不是 Fire TV Alexa Voice Remote 3rd Gen，\
                         按下面两步适配。第一步必做，第二步只在按键不对时才需要。",
                    ),
            )
            .child(section_lab("第一步 · 选择你的遥控器").mt(px(24.)).mb(px(8.)))
            .child(self.device_group(cur_vid, cur_pid, cx))
            .child(section_lab("第二步 · 重新认键").mt(px(24.)).mb(px(8.)))
            .child(self.mapping_group(connected, cx))
    }

    /// 设备选择
    fn device_group(&self, cur_vid: u16, cur_pid: u16, cx: &mut Context<Self>) -> AnyElement {
        let mut g = group().child(
            group_row()
                .child(row_icon("keyboard"))
                .child(row_text(
                    "当前使用的设备标识",
                    Some("先在系统蓝牙里把遥控器连上，再回来这里选它"),
                ))
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(w(560.))
                        .text_color(c(INK2))
                        .child(SharedString::from(format!(
                            "0x{cur_vid:04x} / 0x{cur_pid:04x}"
                        ))),
                ),
        );

        match &self.hid_devs {
            None => {
                g = g.child(hline()).child(
                    group_row().child(row_icon("search")).child(row_text(
                        "扫描已连接的设备",
                        Some("列出系统里所有 HID 设备，从里面挑你的遥控器"),
                    ))
                    .child(primary_btn("adapt-scan", "扫描").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.scan_hid(cx);
                        },
                    ))),
                );
            }
            Some(list) if list.is_empty() => {
                g = g.child(hline()).child(
                    group_row()
                        .child(row_icon("triangle-alert"))
                        .child(row_text("没扫到任何设备", Some("确认遥控器已经在系统蓝牙里连上了")))
                        .child(mini2("adapt-rescan", "重新扫描").on_click(cx.listener(
                            |this, _, _, cx| {
                                this.scan_hid(cx);
                            },
                        ))),
                );
            }
            Some(list) => {
                for (i, d) in list.iter().enumerate() {
                    let chosen = d.vid == cur_vid && d.pid == cur_pid;
                    let dd = d.clone();
                    g = g.child(hline()).child(
                        group_row()
                            .child(row_icon(if chosen { "circle-check" } else { "keyboard" }))
                            .child(row_text2(d.label(), format!("{} · {}", d.ids(), if d.vendor.is_empty() { "厂商未知".into() } else { d.vendor.clone() })))
                            .child(if chosen {
                                div()
                                    .text_size(px(12.))
                                    .font_weight(w(560.))
                                    .text_color(c(OK))
                                    .child("正在使用")
                                    .into_any_element()
                            } else {
                                div()
                                    .id(("adapt-pick", i))
                                    .flex_none()
                                    .px(px(11.))
                                    .py(px(5.))
                                    .rounded(px(7.))
                                    .border_1()
                                    .border_color(c(LINE_STRONG))
                                    .bg(c(SURFACE))
                                    .text_size(px(12.))
                                    .text_color(c(INK))
                                    .cursor_pointer()
                                    .hover(|s| s.border_color(c(ACCENT)).text_color(c(ACCENT_INK)))
                                    .child("用这个")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.pick_device(&dd, cx);
                                    }))
                                    .into_any_element()
                            }),
                    );
                }
                g = g.child(hline()).child(
                    group_row().child(row_icon("refresh-cw")).child(row_text(
                        "没看到你的遥控器？",
                        Some("在系统蓝牙里连上它，然后重新扫描"),
                    ))
                    .child(mini2("adapt-rescan2", "重新扫描").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.scan_hid(cx);
                        },
                    ))),
                );
            }
        }
        g.into_any_element()
    }

    /// 逐键测绘
    fn mapping_group(&self, connected: bool, cx: &mut Context<Self>) -> AnyElement {
        let Some(i) = self.mapping else {
            return group()
                .child(
                    group_row()
                        .child(row_icon("inspector"))
                        .child(row_text(
                            "按键对不上时才需要",
                            Some("会让你依次按遥控器上的每个键，重新记下它们的编号"),
                        ))
                        .when(!connected, |d| {
                            d.child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(c(WARN))
                                    .child("先连上遥控器"),
                            )
                        })
                        .when(connected, |d| {
                            d.child(primary_btn("adapt-map", "开始认键").on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.mapping = Some(0);
                                    this.rt.set_learn(true);
                                    cx.notify();
                                },
                            )))
                        }),
                )
                .into_any_element();
        };

        let total = Slot::ALL.len();
        let done = i >= total;
        group()
            .child(
                group_row()
                    .child(row_icon(if done { "circle-check" } else { "inspector" }))
                    .child(if done {
                        row_text("认键完成", Some("21 个键都记下来了，返回主界面试试"))
                    } else {
                        row_text2(
                            format!("现在请按：{}", crate::cards::card_title(Slot::ALL[i])),
                            format!("第 {} / {total} 个 —— 按一下遥控器上对应的键", i + 1),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .gap(px(6.))
                            .when(!done, |d| {
                                d.child(mini2("adapt-skip", "跳过这个").on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.mapping = this.mapping.map(|x| x + 1);
                                        cx.notify();
                                    },
                                )))
                            })
                            .child(mini2("adapt-stop", if done { "完成" } else { "结束" }).on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.rt.set_learn(false);
                                    this.mapping = None;
                                    this.save();
                                    cx.notify();
                                }),
                            )),
                    ),
            )
            .into_any_element()
    }
}
