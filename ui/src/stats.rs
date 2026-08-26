//! 使用统计页 —— 总览 + 按键排行 + 动作类型分布 + 语音用量。
//! 数据来自 `Config::stats`（动作触发时累加、落盘），这里只读不写。

use crate::theme::*;
use crate::widget::*;
use crate::{FireVibe, Screen};
use firevibe_core::config::Stats;
use firevibe_core::layout::Slot;
use gpui::{div, prelude::*, px, relative, AnyElement, Context, SharedString};

/// slot id -> 界面显示名（用当前语言）
fn slot_name(l: &crate::i18n::L, id: &str) -> String {
    Slot::ALL
        .into_iter()
        .find(|s| s.id() == id)
        .map(|s| l.slot_label(s).to_string())
        .unwrap_or_else(|| id.to_string())
}

/// ActionType 的 debug 名 -> 界面显示名
fn action_name(l: &crate::i18n::L, dbg: &str) -> String {
    l.action_type_name(dbg)
}

impl FireVibe {
    pub fn stats_page(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let l = self.l();
        let st: Stats = self.rt.cfg.read().stats.clone();
        let batt = self.battery();

        // ── 头部：返回 + 标题 ──
        let header = div()
            .flex()
            .items_center()
            .gap(px(11.))
            .mb(px(4.))
            .child(icon_btn_sm("stats-back", "chevron-left").on_click(cx.listener(
                |this, _, _, cx| {
                    this.screen = Screen::Main;
                    cx.notify();
                },
            )))
            .child(
                div()
                    .text_size(px(22.))
                    .font_weight(w(640.))
                    .child(SharedString::from(l.stats_title())),
            );

        // 空态：一次都没用过
        if st.total == 0 {
            return div()
                .max_w(px(720.))
                .flex()
                .flex_col()
                .child(header)
                .child(
                    div()
                        .mt(px(40.))
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(10.))
                        .text_color(c(INK3))
                        .child(icon("chart-pie", 34.))
                        .child(
                            div()
                                .text_size(px(13.))
                                .child(SharedString::from(l.stats_empty())),
                        ),
                );
        }

        // ── 今天 / 本周活跃 ──
        let today = st.by_day.keys().last().cloned().unwrap_or_default();
        let today_n = st.by_day.get(&today).copied().unwrap_or(0);
        let active_days = st.by_day.len();

        // ── 总览卡片行 ──
        let stat_tile = |label: String, value: String| -> AnyElement {
            div()
                .flex_1()
                .min_w(px(0.))
                .bg(c(SURFACE))
                .border_1()
                .border_color(c(LINE))
                .rounded(px(R))
                .shadow(sh1())
                .px(px(16.))
                .py(px(14.))
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(
                    div()
                        .text_size(px(20.))
                        .font_weight(w(680.))
                        .text_color(c(INK))
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .child(SharedString::from(value)),
                )
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(c(INK3))
                        .child(SharedString::from(label)),
                )
                .into_any_element()
        };

        let overview = div()
            .flex()
            .gap(px(10.))
            .child(stat_tile(l.stats_total(), st.total.to_string()))
            .child(stat_tile(l.stats_today(), today_n.to_string()))
            .child(stat_tile(l.stats_active_days(), active_days.to_string()))
            .child(stat_tile(
                l.stats_since(),
                if st.since.is_empty() { "—".into() } else { st.since.clone() },
            ));

        // ── 按键排行（水平条）──
        let mut by_slot: Vec<(String, u64)> = st.by_slot.iter().map(|(k, v)| (k.clone(), *v)).collect();
        by_slot.sort_by(|a, b| b.1.cmp(&a.1));
        let slot_max = by_slot.first().map(|x| x.1).unwrap_or(1).max(1);
        let mut slot_rows = div().flex().flex_col().gap(px(9.));
        for (id, n) in by_slot.iter().take(12) {
            slot_rows = slot_rows.child(bar_row(&slot_name(&l, id), *n, slot_max, ACCENT));
        }

        // ── 动作类型分布 ──
        let mut by_act: Vec<(String, u64)> = st.by_action.iter().map(|(k, v)| (k.clone(), *v)).collect();
        by_act.sort_by(|a, b| b.1.cmp(&a.1));
        let act_max = by_act.first().map(|x| x.1).unwrap_or(1).max(1);
        let mut act_rows = div().flex().flex_col().gap(px(9.));
        for (dbg, n) in by_act.iter() {
            act_rows = act_rows.child(bar_row(&action_name(&l, dbg), *n, act_max, 0x8b5cf6));
        }

        // ── 语音用量 ──
        let mins = (st.voice_seconds / 60.0).floor() as u64;
        let secs = (st.voice_seconds % 60.0).round() as u64;
        let voice_dur = if mins > 0 {
            format!("{mins}m {secs}s")
        } else {
            format!("{secs}s")
        };
        let voice = div()
            .flex()
            .gap(px(12.))
            .child(stat_tile(l.stats_voice_count(), st.voice_count.to_string()))
            .child(stat_tile(l.stats_voice_dur(), voice_dur))
            .child(stat_tile(l.stats_battery(), if batt > 0 { format!("{batt}%") } else { "—".into() }));

        div()
            .max_w(px(720.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(header)
            .child(section_lab(l.stats_overview()).mt(px(20.)).mb(px(8.)))
            .child(overview)
            .child(section_lab(l.stats_by_key()).mt(px(22.)).mb(px(8.)))
            .child(group().p(px(16.)).child(slot_rows))
            .child(section_lab(l.stats_by_action()).mt(px(22.)).mb(px(8.)))
            .child(group().p(px(16.)).child(act_rows))
            .child(section_lab(l.stats_voice()).mt(px(22.)).mb(px(8.)))
            .child(voice)
    }
}

/// 一条水平条：左标签 + 进度条 + 右数值
fn bar_row(label: &str, value: u64, max: u64, color: u32) -> AnyElement {
    let frac = (value as f32 / max as f32).clamp(0.02, 1.0);
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .child(
            div()
                .w(px(96.))
                .flex_none()
                .text_size(px(12.5))
                .text_color(c(INK))
                .overflow_hidden()
                .child(SharedString::from(label.to_string())),
        )
        .child(
            div()
                .flex_1()
                .h(px(10.))
                .rounded(px(5.))
                .bg(c(LINE_SOFT))
                .child(
                    div()
                        .h_full()
                        .w(relative(frac))
                        .rounded(px(5.))
                        .bg(c(color)),
                ),
        )
        .child(
            div()
                .w(px(44.))
                .flex_none()
                .text_size(px(12.5))
                .font_weight(w(560.))
                .text_color(c(INK2))
                .text_right()
                .child(SharedString::from(value.to_string())),
        )
        .into_any_element()
}
