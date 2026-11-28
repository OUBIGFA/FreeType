//! 候选窗配色：内置皮肤、自定义配色、浅深色外观与对比度。
use super::*;

pub(super) fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16))
}

pub(super) fn channels(color: COLORREF) -> (u8, u8, u8) {
    (
        (color.0 & 0xff) as u8,
        ((color.0 >> 8) & 0xff) as u8,
        ((color.0 >> 16) & 0xff) as u8,
    )
}

pub(super) fn parse_color(hex: &str, fallback: COLORREF) -> COLORREF {
    parse_hex_rgb(hex).map_or(fallback, |(r, g, b)| rgb(r, g, b))
}

pub(super) fn blend_rgb(a: COLORREF, b: COLORREF, t: f32) -> COLORREF {
    let (ar, ag, ab) = channels(a);
    let (br, bg, bb) = channels(b);
    let mix = |x: u8, y: u8| -> u8 {
        (f32::from(x) * (1.0 - t) + f32::from(y) * t)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    rgb(mix(ar, br), mix(ag, bg), mix(ab, bb))
}

pub(super) fn contrast_ratio(a: COLORREF, b: COLORREF) -> f64 {
    fn luminance(color: COLORREF) -> f64 {
        let linear = |value: u8| {
            let value = f64::from(value) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        let (r, g, b) = channels(color);
        0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
    }
    let (first, second) = (luminance(a), luminance(b));
    (first.max(second) + 0.05) / (first.min(second) + 0.05)
}

pub(super) fn light_mode() -> bool {
    let mut value = 1u32;
    let mut size = 4u32;
    unsafe {
        let _ = RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        );
    }
    value != 0
}

/// 按设置里的候选皮肤（`Config::candidate_style` / `Config::candidate_skin`）和系统浅色 / 深色配色。
/// 设置页的预览（windows/app/pages.go 的 resolveCandidateTheme）用同样的颜色，改一边要改另一边。
pub(super) fn theme(style: &str, custom: Option<&CandidateSkin>) -> Theme {
    theme_for_appearance(style, custom, light_mode())
}

// 显式接收外观，便于在不修改用户系统主题的情况下验证两套候选文字颜色。
pub(super) fn theme_for_appearance(
    style: &str,
    custom: Option<&CandidateSkin>,
    light: bool,
) -> Theme {
    let white = rgb(0xff, 0xff, 0xff);
    if style == "custom" {
        let defaults = CandidateSkin::default();
        let skin = custom.unwrap_or(&defaults);
        let background = parse_color(&skin.background, white);
        let border = parse_color(&skin.border, rgb(0xd6, 0xd6, 0xd6));
        let text = parse_color(&skin.text, rgb(0x1f, 0x1f, 0x1f));
        let muted = parse_color(&skin.muted, rgb(0x61, 0x61, 0x61));
        let accent_rgb = parse_color(&skin.accent, rgb(0x2b, 0x6c, 0xe6));
        let accent_text = parse_color(&skin.accent_text, white);
        let hover = blend_rgb(background, text, 0.08);
        let accent = skin.accent_fill.then_some(accent_rgb);
        let emphasis_candidate = if skin.accent_fill {
            accent_rgb
        } else {
            accent_text
        };
        let emphasis = if contrast_ratio(emphasis_candidate, background) >= 3.0 {
            emphasis_candidate
        } else {
            text
        };
        return Theme {
            background,
            border,
            text,
            muted,
            hover,
            accent,
            accent_text,
            emphasis,
        };
    }
    let wechat_green = rgb(0x03, 0xd4, 0x68);
    // (底色块, 块上的字, 提示字色)
    let (accent, accent_text, emphasis) = match (style, light) {
        ("blue", true) => (Some(rgb(0x2b, 0x6c, 0xe6)), white, rgb(0x2b, 0x6c, 0xe6)),
        ("blue", false) => (
            Some(rgb(0x3d, 0x7f, 0xf0)),
            rgb(0x18, 0x18, 0x1b),
            rgb(0x6c, 0x9e, 0xff),
        ),
        // 微信绿候选保留绿底白字；浅色主题的单字提示仍用深绿。
        ("green", true) => (Some(wechat_green), white, rgb(0x06, 0x82, 0x4a)),
        ("green", false) => (Some(wechat_green), white, wechat_green),
        ("sogou", true) => (None, rgb(0x1d, 0x66, 0xdf), rgb(0x1d, 0x66, 0xdf)),
        ("sogou", false) => (None, rgb(0x6c, 0x9e, 0xff), rgb(0x6c, 0x9e, 0xff)),
        // 默认：品牌粉色。粉色很浅，块上用深色字（和设置窗口一样）；提示字用深一档的粉
        (_, true) => (
            Some(rgb(0xff, 0xb3, 0xfa)),
            rgb(0x18, 0x18, 0x1b),
            rgb(0xc0, 0x3c, 0xb4),
        ),
        (_, false) => (
            Some(rgb(0xff, 0xb3, 0xfa)),
            rgb(0x18, 0x18, 0x1b),
            rgb(0xff, 0xb3, 0xfa),
        ),
    };
    // 鼠标停留的底用中性灰，几种强调色都配得上
    if light {
        Theme {
            background: white,
            border: rgb(0xd6, 0xd6, 0xd6),
            text: rgb(0x1f, 0x1f, 0x1f),
            muted: rgb(0x61, 0x61, 0x61),
            hover: rgb(0xf0, 0xf0, 0xf0),
            accent,
            accent_text,
            emphasis,
        }
    } else {
        Theme {
            background: rgb(0x2b, 0x2b, 0x2b),
            border: rgb(0x48, 0x48, 0x48),
            text: rgb(0xee, 0xee, 0xee),
            muted: rgb(0x9a, 0x9a, 0x9a),
            hover: rgb(0x3a, 0x3a, 0x3a),
            accent,
            accent_text,
            emphasis,
        }
    }
}
