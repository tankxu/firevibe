//! 设计 token —— 与 `design/mockup.html` 的 `:root` 变量一一对应。
//! 改这里等于改设计稿，两边要一起动。
#![allow(dead_code)]

use gpui::{
    hsla, linear_color_stop, linear_gradient, px, rgb, Background, BoxShadow, Hsla, Point, Rgba,
};

// ── 页面 ──
pub const BG: u32 = 0xfbfbfc;
pub const SURFACE: u32 = 0xffffff;
pub const LINE: u32 = 0xececf0;
pub const LINE_STRONG: u32 = 0xdcdce3;
/// 卡片内短按/长按之间那条分隔线。设计稿是虚线，gpui 没有 border-dashed，
/// 用更淡的实线代替，视觉重量一致。
pub const LINE_SOFT: u32 = 0xf2f2f6;

// ── 文字 ──
pub const INK: u32 = 0x0f1115;
pub const INK2: u32 = 0x5b6070;
pub const INK3: u32 = 0x9aa0b0;

// ── 主题色（Fire TV 蓝）──
pub const ACCENT: u32 = 0x00a8e1;
pub const ACCENT_HOVER: u32 = 0x0093c6;
pub const ACCENT_SOFT: u32 = 0xe4f5fd;
pub const ACCENT_INK: u32 = 0x0080ad;

/// 语音类动作的标记色（琥珀）—— 只用来在动作类型列表里把「要用麦克风的那几个」
/// 一眼挑出来。选中态下图标改用 SURFACE，蓝底上才看得清。
pub const MIC_MARK: u32 = 0xe0a020;

// ── 语义色 ──
pub const OK: u32 = 0x12a150;
pub const OK_SOFT: u32 = 0xe8f6ee;
pub const WARN: u32 = 0xb4690e;
pub const WARN_LINE: u32 = 0xf0dcae;
pub const WARN_BG: u32 = 0xfffdf6;
pub const ERR: u32 = 0xd0342c;
pub const ERR_SOFT: u32 = 0xfdeceb;
pub const ERR_LINE: u32 = 0xf0c3c0;

// ── 交互底色 ──
pub const HOVER: u32 = 0xf1f2f6;
pub const MENU_HOVER: u32 = 0xf2f3f7;
pub const FOOT_BG: u32 = 0xfbfbfc;
pub const CODE_BG: u32 = 0xfafbfc;
pub const STAGE_BG: u32 = 0xe9ebef;
pub const ALT_ROW: u32 = 0xf7fcff;
pub const OFF_CARD: u32 = 0xfcfcfd;
pub const TAG_BG: u32 = 0xf0f0f4;
/// 卡片「…」按钮没悬停时的颜色
pub const MORE_IDLE: u32 = 0xc9ccd6;

// ── 遥控器机身 ──
/// 外壳渐变四段停靠色（对应 CSS 的 0% / 18% / 62% / 100%）
pub const SHELL: [(u32, f32); 4] =
    [(0x3a3d47, 0.0), (0x22242b, 0.18), (0x101216, 0.62), (0x1c1e25, 1.0)];
/// 渐变方向。CSS 那条是 x1=0,y1=0 → x2=0.45,y2=1，算出来 172.3°，
/// 但**这里必须写 180**：gpui 的渐变按元素自己的包围盒投影算长度，
/// 而外壳被切成了三条扁横带 —— 172.3° 在 316×188 的扁盒里只能走完约 82%，
/// 第一段就永远暗不下来（实测上部整体偏亮）。CSS 那条轴本身 99.1% 是竖直的，
/// 横向分量整幅只贡献 4%，直接当竖直用，误差比切带带来的误差小一个量级。
pub const SHELL_ANGLE: f32 = 180.0;
pub const WELL: u32 = 0x0b0c10;
pub const MIC_SLIT_C: u32 = 0x4d515c;
pub const BTN_TOP: u32 = 0x3b3e49;
pub const BTN_BOT: u32 = 0x272a33;
pub const BTN_TOP_H: u32 = 0x474b58;
pub const BTN_BOT_H: u32 = 0x31343e;
pub const BTN_INK: u32 = 0xdcdee5;
pub const OK_TOP: u32 = 0x2a2d36;
pub const OK_BOT: u32 = 0x15171c;
pub const DPAD_INK: u32 = 0x9aa0ad;
/// Alexa 蓝（麦克风键）—— 高光已按要求压过一轮
pub const MIC_TOP: u32 = 0x35bde8;
pub const MIC_BOT: u32 = 0x0b78ad;
pub const MIC_INK: u32 = 0xeaf9ff;

/// 四个 App 键的品牌渐变 (上, 下, 字色)
pub const APP_TINT: [(u32, u32, u32); 4] = [
    (0x2f7fd4, 0x1d5aa8, 0xffffff),
    (0xe5312b, 0xc01f1a, 0xffffff),
    (0x2b3a8f, 0x1b2568, 0xffffff),
    (0x2fe38f, 0x16bb70, 0x08301d),
];

/// 卡片左上角键徽章的渐变 (上, 下, 字色)，160°
pub const BADGE_DEFAULT: (u32, u32, u32) = (0x2b2d36, 0x14161c, 0xffffff);
pub const BADGE_TINT: [(u32, u32, u32); 4] = [
    (0x2f7fd4, 0x1d5aa8, 0xffffff),
    (0xe5312b, 0xb91c1c, 0xffffff),
    (0x3b4bb5, 0x1b2568, 0xffffff),
    (0x2fe38f, 0x0f8a45, 0x06301b),
];
/// 麦克风卡片徽章（Alexa 青）
pub const BADGE_MIC: (u32, u32, u32) = (0x22c9f8, 0x0284c7, 0xffffff);

// ── 圆角 ──
pub const R_SM: f32 = 8.;
pub const R: f32 = 12.;
pub const R_LG: f32 = 16.;

pub fn c(v: u32) -> Rgba {
    rgb(v)
}

/// 两色按 t∈[0,1] 线性插值。gpui 没有 CSS transition，
/// 所有过渡都得自己按帧算，这是最常用的一块。
pub fn mix(a: u32, b: u32, t: f32) -> u32 {
    let t = t.clamp(0., 1.);
    let ch = |sh: u32| {
        let (x, y) = (((a >> sh) & 0xff) as f32, ((b >> sh) & 0xff) as f32);
        ((x + (y - x) * t).round() as u32).min(255) << sh
    };
    ch(16) | ch(8) | ch(0)
}

/// sh1 → sh2 之间插值，卡片 hover 时阴影长起来
pub fn sh_lerp(t: f32) -> Vec<BoxShadow> {
    let t = t.clamp(0., 1.);
    let l = |a: f32, b: f32| a + (b - a) * t;
    vec![
        sh(l(1., 4.), l(2., 12.), l(0., -2.), l(0.05, 0.08)),
        sh(l(1., 2.), l(1., 4.), l(0., -2.), l(0.04, 0.06)),
    ]
}

/// 缓出，收尾软一点，别像线性那样一停就停
pub fn ease_out(p: f32) -> f32 {
    let p = p.clamp(0., 1.);
    1. - (1. - p).powi(3)
}

/// 按亮度把颜色压成灰。gpui 没有 `filter: grayscale()`，
/// 设计稿里禁用卡片的徽章要求 `grayscale(1) opacity(.5)`，只能自己算。
pub fn desaturate(v: u32) -> u32 {
    let (r, g, b) = ((v >> 16) & 0xff, (v >> 8) & 0xff, v & 0xff);
    let y = (299 * r + 587 * g + 114 * b) / 1000;
    (y << 16) | (y << 8) | y
}

/// 把 token 颜色转成带透明度的 Hsla，用来做光圈这类叠加
pub fn hsla_of(v: u32, alpha: f32) -> Hsla {
    let mut h: Hsla = c(v).into();
    h.a = alpha;
    h
}

/// 半透明黑/白，用来堆内阴影和高光
pub fn black(a: f32) -> Hsla {
    hsla(0., 0., 0., a)
}
pub fn white(a: f32) -> Hsla {
    hsla(0., 0., 1., a)
}

/// 竖直渐变（CSS 的 `linear-gradient(180deg, a, b)`）
pub fn grad_v(top: u32, bot: u32) -> Background {
    linear_gradient(180., linear_color_stop(c(top), 0.), linear_color_stop(c(bot), 1.))
}

/// 任意角度两段渐变
pub fn grad(angle: f32, from: u32, to: u32) -> Background {
    linear_gradient(angle, linear_color_stop(c(from), 0.), linear_color_stop(c(to), 1.))
}

// ── 阴影（对应 --sh-1 / --sh-2 / --sh-3）──
fn sh(y: f32, blur: f32, spread: f32, a: f32) -> BoxShadow {
    BoxShadow {
        color: hsla(226. / 360., 0.27, 0.086, a),
        offset: Point { x: px(0.), y: px(y) },
        blur_radius: px(blur),
        spread_radius: px(spread),
    }
}

pub fn sh1() -> Vec<BoxShadow> {
    vec![sh(1., 2., 0., 0.05), sh(1., 1., 0., 0.04)]
}
pub fn sh2() -> Vec<BoxShadow> {
    vec![sh(4., 12., -2., 0.08), sh(2., 4., -2., 0.06)]
}
pub fn sh3() -> Vec<BoxShadow> {
    vec![sh(18., 40., -12., 0.22), sh(6., 14., -6., 0.12)]
}

/// 遥控器上普通按钮的凸起阴影
pub fn btn_shadow() -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: black(0.35),
        offset: Point { x: px(0.), y: px(2.) },
        blur_radius: px(4.),
        spread_radius: px(0.),
    }]
}

/// OK 键更深的落影
pub fn ok_shadow() -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: black(0.5),
        offset: Point { x: px(0.), y: px(3.) },
        blur_radius: px(8.),
        spread_radius: px(0.),
    }]
}

/// 贴边一圈纯色光圈，等价于 CSS 的 `box-shadow: 0 0 0 <spread>px <color>`。
/// **别用 border 代替** —— gpui 的边框是往元素**内**收的，
/// 7px 的点加 3px 边框只剩中间 1px 是本色，看着等于没亮。
pub fn ring(color: Hsla, spread: f32) -> Vec<BoxShadow> {
    vec![BoxShadow {
        color,
        offset: Point { x: px(0.), y: px(0.) },
        blur_radius: px(0.),
        spread_radius: px(spread),
    }]
}

/// 麦克风键的蓝色光晕
pub fn mic_shadow() -> Vec<BoxShadow> {
    vec![
        // 贴边那一圈 2px（CSS 的 0 0 0 2px rgba(26,169,221,.16)）
        BoxShadow {
            color: hsla(196. / 360., 0.79, 0.48, 0.16),
            offset: Point { x: px(0.), y: px(0.) },
            blur_radius: px(0.),
            spread_radius: px(2.),
        },
        BoxShadow {
            color: hsla(196. / 360., 0.79, 0.48, 0.20),
            offset: Point { x: px(0.), y: px(0.) },
            blur_radius: px(9.),
            spread_radius: px(1.),
        },
        BoxShadow {
            color: black(0.38),
            offset: Point { x: px(0.), y: px(2.) },
            blur_radius: px(5.),
            spread_radius: px(0.),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mix_interpolates() {
        assert_eq!(mix(0x000000, 0xffffff, 0.0), 0x000000);
        assert_eq!(mix(0x000000, 0xffffff, 1.0), 0xffffff);
        assert_eq!(mix(0x000000, 0xffffff, 0.5), 0x808080);
        // 每个通道独立插值，不串色
        assert_eq!(mix(0xff0000, 0x0000ff, 0.5), 0x800080);
        // 越界会被夹住
        assert_eq!(mix(0x102030, 0x405060, -1.0), 0x102030);
        assert_eq!(mix(0x102030, 0x405060, 9.0), 0x405060);
    }

    #[test]
    fn ease_out_is_monotonic_and_bounded() {
        assert_eq!(ease_out(0.0), 0.0);
        assert_eq!(ease_out(1.0), 1.0);
        let mut prev = -1.0;
        for i in 0..=20 {
            let v = ease_out(i as f32 / 20.0);
            assert!(v >= prev, "不单调：{v} < {prev}");
            assert!((0.0..=1.0).contains(&v));
            prev = v;
        }
        // 缓出：前半段就该走完一多半
        assert!(ease_out(0.5) > 0.8, "缓出曲线不对：{}", ease_out(0.5));
    }

    #[test]
    fn shadow_grows_with_t() {
        let a = sh_lerp(0.0);
        let b = sh_lerp(1.0);
        assert!(b[0].blur_radius > a[0].blur_radius);
        assert_eq!(sh_lerp(0.0)[0].blur_radius, sh1()[0].blur_radius);
        assert_eq!(sh_lerp(1.0)[0].blur_radius, sh2()[0].blur_radius);
    }
}
