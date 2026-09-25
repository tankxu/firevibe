//! 使用统计页 —— 总览 + 按键排行 + 动作类型分布 + 语音用量。
//! 数据来自 `Config::stats`（动作触发时累加、落盘），这里只读不写。

use crate::theme::*;
use crate::widget::*;
use crate::{FireVibe, Screen};
use firevibe_core::config::Stats;
use firevibe_core::layout::Slot;
use gpui::{
    canvas, div, point, prelude::*, px, relative, AnyElement, Context, PathBuilder, SharedString,
};
use gpui_component::tooltip::Tooltip;

/// 最近 N 天用来画折线图。`by_day` 只存有活动的日子，这里从最后一天往回补齐
/// 连续 N 个自然日（没活动的天补 0），这样折线才反映真实的「用/没用」。
const RECENT_DAYS: usize = 14;

fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

/// "YYYY-MM-DD" 的前一天。整数运算，不依赖 chrono，也不 spawn `date`。
fn prev_day(s: &str) -> Option<String> {
    let p: Vec<&str> = s.split('-').collect();
    if p.len() != 3 {
        return None;
    }
    let (mut y, mut m, mut d) = (
        p[0].parse::<i32>().ok()?,
        p[1].parse::<u32>().ok()?,
        p[2].parse::<u32>().ok()?,
    );
    if d > 1 {
        d -= 1;
    } else {
        if m > 1 {
            m -= 1;
        } else {
            m = 12;
            y -= 1;
        }
        d = days_in_month(y, m);
    }
    Some(format!("{y:04}-{m:02}-{d:02}"))
}

/// "YYYY-MM-DD" -> "M/D"，x 轴标签用
fn short_date(s: &str) -> String {
    let p: Vec<&str> = s.split('-').collect();
    if p.len() == 3 {
        let m = p[1].trim_start_matches('0');
        let d = p[2].trim_start_matches('0');
        format!("{m}/{d}")
    } else {
        s.to_string()
    }
}

/// 从 by_day 取最近 RECENT_DAYS 天（补 0），返回按时间正序的 (日期, 次数)。
fn recent_series(st: &Stats) -> Vec<(String, u64)> {
    let Some(last) = st.by_day.keys().last().cloned() else {
        return Vec::new();
    };
    let mut days: Vec<String> = vec![last.clone()];
    let mut cur = last;
    for _ in 1..RECENT_DAYS {
        match prev_day(&cur) {
            Some(p) => {
                days.push(p.clone());
                cur = p;
            }
            None => break,
        }
    }
    days.reverse();
    days.into_iter()
        .map(|d| {
            let n = st.by_day.get(&d).copied().unwrap_or(0);
            (d, n)
        })
        .collect()
}

/// 最近使用折线图：面积渐隐 + 折线 + 数据点。峰值当 y 轴最大值标左上角，
/// x 轴均匀铺几个日期（正常折线图画法）。
fn usage_chart(days: &[(String, u64)]) -> AnyElement {
    let peak = days.iter().map(|(_, v)| *v).max().unwrap_or(1).max(1);
    let vals: Vec<f32> = days.iter().map(|(_, v)| *v as f32 / peak as f32).collect();
    let n = vals.len();

    // ⚠️ canvas 元素默认 0×0（remote.rs 那处只用 bounds.origin 按绝对坐标画，
    // 所以没暴露这点）。这里要用 bounds.size，必须给 canvas **显式撑满**，
    // 否则读到 0×0、所有点被压到顶部成一条平线。
    let plot = div().h(px(96.)).w_full().child(
        canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                if n < 2 {
                    return;
                }
                let o = bounds.origin;
                let w = f32::from(bounds.size.width);
                let h = f32::from(bounds.size.height);
                let (padx, padtop, padbot) = (6.0f32, 10.0f32, 6.0f32);
                let plot_w = (w - padx * 2.0).max(1.0);
                let plot_h = (h - padtop - padbot).max(1.0);
                let base_y = padtop + plot_h;
                let xat = |i: usize| padx + plot_w * (i as f32 / (n - 1) as f32);
                // 峰值不顶到最上沿，留一点头
                let yat = |v: f32| padtop + plot_h * (1.0 - v * 0.92);
                let pt = |x: f32, y: f32| point(o.x + px(x), o.y + px(y));

                // 面积（accent 半透明，往下渐隐靠 alpha 两段实现不了，就用一层浅填充）
                let mut area = PathBuilder::fill();
                area.move_to(pt(xat(0), base_y));
                for i in 0..n {
                    area.line_to(pt(xat(i), yat(vals[i])));
                }
                area.line_to(pt(xat(n - 1), base_y));
                area.close();
                if let Ok(p) = area.build() {
                    window.paint_path(p, hsla_of(ACCENT, 0.13));
                }
                // 折线
                let mut line = PathBuilder::stroke(px(2.0));
                line.move_to(pt(xat(0), yat(vals[0])));
                for i in 1..n {
                    line.line_to(pt(xat(i), yat(vals[i])));
                }
                if let Ok(p) = line.build() {
                    window.paint_path(p, hsla_of(ACCENT, 1.0));
                }
                // 数据点：小方点（gpui 画圆麻烦，2.6px 圆角方点在这尺寸看着就是圆点）
                for i in 0..n {
                    let (cx, cy) = (xat(i), yat(vals[i]));
                    let mut dot = PathBuilder::fill();
                    let r = 2.2;
                    dot.move_to(pt(cx - r, cy));
                    dot.line_to(pt(cx, cy - r));
                    dot.line_to(pt(cx + r, cy));
                    dot.line_to(pt(cx, cy + r));
                    dot.close();
                    if let Ok(p) = dot.build() {
                        window.paint_path(p, hsla_of(ACCENT, 1.0));
                    }
                }
            },
        )
        .size_full(),
    );

    // 图区：折线 + 左上角 y 轴峰值参考（0 在底、peak 在顶）
    let chart = div()
        .relative()
        .child(plot)
        .child(
            div()
                .absolute()
                .top(px(-2.))
                .left(px(0.))
                .text_size(px(10.5))
                .text_color(c(INK3))
                .child(SharedString::from(peak.to_string())),
        )
        .child(
            div()
                .absolute()
                .bottom(px(-2.))
                .left(px(0.))
                .text_size(px(10.5))
                .text_color(c(INK3))
                .child("0"),
        );

    // x 轴：均匀取 5 个日期，按位置铺开（justify_between：首左、尾右、中间散开）
    let mut axis = div()
        .flex()
        .justify_between()
        .mt(px(8.))
        .text_size(px(11.))
        .text_color(c(INK3));
    for k in 0..5 {
        let i = (k * (n - 1) / 4).min(n - 1);
        axis = axis.child(SharedString::from(short_date(&days[i].0)));
    }

    group()
        .p(px(14.))
        .child(chart)
        .child(axis)
        .into_any_element()
}

/// `YYYY-MM-DD HH:MM` -> 以天为单位的连续坐标（含小数）。
///
/// 用 Howard Hinnant 那个 civil→days 算法，闰年和月长都是对的。
/// ⚠️ 必须按**真实时间**定位横坐标，不能按点的下标均分 —— 电量采样是
/// 「变了才记」，点与点的间隔天差地别（可能几分钟，也可能好几天），
/// 均分会把一段几天没动的平台期画得和一次急跌一样宽。
fn day_coord(t: &str) -> f64 {
    let b = t.as_bytes();
    if b.len() < 10 {
        return 0.0;
    }
    let num = |a: usize, z: usize| t[a..z].parse::<i64>().unwrap_or(0);
    let (y, m, d) = (num(0, 4), num(5, 7), num(8, 10));
    if m == 0 {
        return 0.0;
    }
    // days_from_civil
    let y2 = y - i64::from(m <= 2);
    let era = if y2 >= 0 { y2 } else { y2 - 399 } / 400;
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    // 时分（有就用，没有当 0 点）
    let frac = if b.len() >= 16 {
        (num(11, 13) * 60 + num(14, 16)) as f64 / 1440.0
    } else {
        0.0
    };
    days as f64 + frac
}

/// 把 `YYYY-MM-DD HH:MM` 缩成 `MM-DD`
fn short_stamp(t: &str) -> String {
    if t.len() >= 10 { t[5..10].to_string() } else { t.to_string() }
}

/// 一台设备的电量曲线。y 轴固定 0–100%（电量本来就是百分比，
/// 用峰值当上限会让「从 95 掉到 90」看起来像悬崖）。
///
/// ⚠️ **坐标用「占整块区域的比例」算，不用像素 padding**：canvas 里画线和上面那层
/// hover 区必须严丝合缝地对齐，而 hover 区是用 `relative()` 百分比定位的 ——
/// 两边只要有一边掺了像素 padding 就会错位。所以留白也做成比例（上下各 8%、
/// 左右各 3%），两处共用 `xfrac` / `yfrac` 同一个公式。
fn battery_chart(points: &[(String, i32)]) -> AnyElement {
    let n = points.len();
    let xs: Vec<f64> = points.iter().map(|(t, _)| day_coord(t)).collect();
    let (x0, x1) = (xs[0], xs[n - 1]);
    let span = (x1 - x0).max(1.0 / 24.0); // 全挤在一小时内也别除零
    // 点在整块区域里的相对位置（0~1），canvas 和 hover 区共用
    let xfrac: Vec<f32> = xs
        .iter()
        .map(|x| 0.03 + 0.94 * ((x - x0) / span) as f32)
        .collect();
    let yfrac: Vec<f32> = points
        .iter()
        .map(|(_, v)| 0.08 + 0.84 * (1.0 - *v as f32 / 100.0))
        .collect();

    let (fx, fy) = (xfrac.clone(), yfrac.clone());
    let plot = canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            if n < 2 {
                return;
            }
            let o = bounds.origin;
            let w = f32::from(bounds.size.width);
            let h = f32::from(bounds.size.height);
            let pt = |i: usize| point(o.x + px(w * fx[i]), o.y + px(h * fy[i]));
            let base = |i: usize| point(o.x + px(w * fx[i]), o.y + px(h));

            let mut area = PathBuilder::fill();
            area.move_to(base(0));
            for i in 0..n {
                area.line_to(pt(i));
            }
            area.line_to(base(n - 1));
            area.close();
            if let Ok(p) = area.build() {
                window.paint_path(p, hsla_of(ACCENT, 0.13));
            }
            let mut line = PathBuilder::stroke(px(2.0));
            line.move_to(pt(0));
            for i in 1..n {
                line.line_to(pt(i));
            }
            if let Ok(p) = line.build() {
                window.paint_path(p, hsla_of(ACCENT, 1.0));
            }
            // 数据点：采样是「变了才记」，标出来才看得清哪几天在掉电
            for i in 0..n {
                let c = pt(i);
                let r = px(2.2);
                let mut dot = PathBuilder::fill();
                dot.move_to(point(c.x - r, c.y));
                dot.line_to(point(c.x, c.y - r));
                dot.line_to(point(c.x + r, c.y));
                dot.line_to(point(c.x, c.y + r));
                dot.close();
                if let Ok(p) = dot.build() {
                    window.paint_path(p, hsla_of(ACCENT, 1.0));
                }
            }
        },
    )
    // ⚠️ 少了 `.size_full()`，canvas 读到的 bounds 是 0×0、所有点被压到顶部 ——
    // 画出来就是一条贴着 100% 的水平直线。CLAUDE.md 记过这个坑，还是漏过一次。
    .size_full();

    // hover 层：每个数据点铺一块透明的命中区，鼠标放上去用 Tooltip 报具体数值。
    // 命中区做得比点大（18px），不然 2.2px 的点根本对不准。
    let mut layer = div().absolute().size_full();
    for i in 0..n {
        let (t, v) = (points[i].0.clone(), points[i].1);
        let tip = SharedString::from(format!("{}  {}%", stamp_label(&t), v));
        layer = layer.child(
            div()
                .id(SharedString::from(format!("bp-{i}-{t}")))
                .absolute()
                .left(relative(xfrac[i]))
                .top(relative(yfrac[i]))
                .ml(px(-9.))
                .mt(px(-9.))
                .size(px(18.))
                .rounded_full()
                .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx)),
        );
    }

    let chart = div()
        .relative()
        .h(px(88.))
        .w_full()
        .child(plot)
        .child(layer)
        .child(
            div().absolute().top(px(-2.)).left(px(0.))
                .text_size(px(10.5)).text_color(c(INK3)).child("100%"),
        )
        .child(
            div().absolute().bottom(px(-2.)).left(px(0.))
                .text_size(px(10.5)).text_color(c(INK3)).child("0"),
        );

    let axis = div()
        .flex()
        .justify_between()
        .mt(px(8.))
        .text_size(px(11.))
        .text_color(c(INK3))
        .child(SharedString::from(short_stamp(&points[0].0)))
        .child(SharedString::from(short_stamp(&points[n - 1].0)));

    div().child(chart).child(axis).into_any_element()
}

/// tooltip 里的时间写法：`09-20 03:15`（去掉年份，年份在这种图上没信息量）
fn stamp_label(t: &str) -> String {
    if t.len() >= 16 {
        t[5..16].to_string()
    } else {
        t.to_string()
    }
}

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
            .child(section_lab(l.stats_recent()).mt(px(22.)).mb(px(8.)))
            .child(usage_chart(&recent_series(&st)))
            .child(section_lab(l.stats_by_key()).mt(px(22.)).mb(px(8.)))
            .child(group().p(px(16.)).child(slot_rows))
            .child(section_lab(l.stats_by_action()).mt(px(22.)).mb(px(8.)))
            .child(group().p(px(16.)).child(act_rows))
            .child(section_lab(l.stats_voice()).mt(px(22.)).mb(px(8.)))
            .child(voice)
            .children(self.battery_section(cx))
    }

    /// 电量变化：**每台设备一条曲线**，最近用过的排在上面。
    ///
    /// 为什么按设备分开：换过遥控器的人，两支的电量曲线混在一条线里毫无意义
    /// （一支 20% 一支 90%，看起来像反复充电）。key 用 HID 序列号 ——
    /// 同型号两支的 vid/pid 一模一样，只有序列号能分开。
    ///
    /// 用不上的设备（换掉的旧遥控器）可以「隐藏」，收进底部一栏，随时能展开取消。
    fn battery_section(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let l = self.l();
        let cfg = self.rt.cfg.read();
        if cfg.stats.battery.is_empty() {
            return Vec::new();
        }
        // 最近见到的排前面
        let mut all: Vec<(String, firevibe_core::config::BatteryLog)> =
            cfg.stats.battery.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        drop(cfg);
        all.sort_by(|a, b| b.1.last_seen.cmp(&a.1.last_seen));
        let hidden_n = all.iter().filter(|(_, v)| v.hidden).count();
        let show_hidden = self.show_hidden_batt;

        let card = |sn: String, log: &firevibe_core::config::BatteryLog, cx: &mut Context<Self>| {
            let now = log.points.last().map(|(_, v)| *v).unwrap_or(0);
            let title = if log.name.trim().is_empty() { sn.clone() } else { log.name.clone() };
            let hide_label = if log.hidden { l.batt_show() } else { l.batt_hide() };
            let sn2 = sn.clone();
            let head = div()
                .flex()
                .items_center()
                .gap(px(8.))
                .mb(px(6.))
                .child(
                    div().min_w(px(0.)).overflow_hidden()
                        .text_size(px(13.)).font_weight(w(600.)).text_color(c(INK))
                        .child(SharedString::from(title)),
                )
                // 隐藏按钮紧挨设备名（那是它作用的对象），电量数字推到最右边
                .child(
                    div()
                        .id(SharedString::from(format!("batt-hide-{sn2}")))
                        .px(px(8.)).py(px(3.)).rounded(px(6.))
                        .border_1().border_color(c(LINE))
                        .text_size(px(11.)).text_color(c(INK3))
                        .cursor_pointer()
                        .child(SharedString::from(hide_label.to_string()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            {
                                let mut g = this.rt.cfg.write();
                                if let Some(e) = g.stats.battery.get_mut(&sn2) {
                                    e.hidden = !e.hidden;
                                }
                                let _ = g.save();
                            }
                            cx.notify();
                        })),
                )
                .child(div().flex_1().min_w(px(0.)))
                .child(
                    div().text_size(px(13.)).font_weight(w(680.)).text_color(c(INK))
                        .child(SharedString::from(if now > 0 { format!("{now}%") } else { "—".into() })),
                );
            let body: AnyElement = if log.points.len() >= 2 {
                battery_chart(&log.points)
            } else {
                div().py(px(14.)).text_size(px(12.)).text_color(c(INK3))
                    .child(SharedString::from(l.batt_one_point().to_string()))
                    .into_any_element()
            };
            group()
                .p(px(14.))
                .mb(px(8.))
                .child(head)
                .child(body)
                .child(
                    div().mt(px(6.)).text_size(px(11.)).text_color(c(INK3))
                        .child(SharedString::from(l.batt_last_seen(&short_stamp(&log.last_seen)))),
                )
                .into_any_element()
        };

        let mut out: Vec<AnyElement> = vec![section_lab(l.stats_batt_curve()).mt(px(22.)).mb(px(8.)).into_any_element()];
        for (sn, log) in all.iter().filter(|(_, v)| !v.hidden) {
            out.push(card(sn.clone(), log, cx));
        }
        if hidden_n > 0 {
            out.push(
                div()
                    .id("batt-hidden-toggle")
                    .mt(px(2.)).mb(px(6.))
                    .text_size(px(11.5)).text_color(c(INK3))
                    .cursor_pointer()
                    .child(SharedString::from(format!(
                        "{} {}", if show_hidden { "▾" } else { "▸" }, l.batt_hidden_n(hidden_n)
                    )))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_hidden_batt = !this.show_hidden_batt;
                        cx.notify();
                    }))
                    .into_any_element(),
            );
            if show_hidden {
                for (sn, log) in all.iter().filter(|(_, v)| v.hidden) {
                    out.push(card(sn.clone(), log, cx));
                }
            }
        }
        out
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
