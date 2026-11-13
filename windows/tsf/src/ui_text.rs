//! 候选文字的系统排版和绘制。测量与绘制共用同一个 DirectWrite layout，
//! 让变体选择符、组合字符、ZWJ 表情和字体回退在两边得到同一组字形。
use unicode_segmentation::UnicodeSegmentation;
use windows::core::{w, Result, HSTRING};
use windows::Win32::Foundation::{COLORREF, RECT, SIZE};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_IGNORE, D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_RECT_F,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1DCRenderTarget, ID2D1Factory, ID2D1SolidColorBrush,
    D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_SOFTWARE, D2D1_ROUNDED_RECT,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat, IDWriteTextLayout,
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD,
    DWRITE_TEXT_METRICS, DWRITE_TEXT_RANGE, DWRITE_WORD_WRAPPING_NO_WRAP,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::HDC;
use windows_numerics::Vector2;

pub struct Font {
    factory: IDWriteFactory,
    format: IDWriteTextFormat,
}

pub struct Fonts {
    pub main: Font,
    pub small: Font,
    pub hint: Font,
}

impl Fonts {
    pub fn new(size: f32, family: &str) -> Result<Self> {
        let factory: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        let family = if family.trim().is_empty() {
            freetype_protocol::DEFAULT_FONT
        } else {
            family
        };
        let make = |size, weight: DWRITE_FONT_WEIGHT| -> Result<Font> {
            let format = unsafe {
                factory.CreateTextFormat(
                    &HSTRING::from(family),
                    None,
                    weight,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    w!("zh-CN"),
                )?
            };
            unsafe {
                format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            }
            Ok(Font {
                factory: factory.clone(),
                format,
            })
        };
        Ok(Self {
            main: make(size, DWRITE_FONT_WEIGHT_NORMAL)?,
            small: make((size * 0.78).round(), DWRITE_FONT_WEIGHT_NORMAL)?,
            hint: make(size, DWRITE_FONT_WEIGHT_SEMI_BOLD)?,
        })
    }
}

#[derive(Clone)]
pub struct Text {
    layout: IDWriteTextLayout,
    pub size: SIZE,
    offset: Vector2,
}

impl Font {
    pub fn text(&self, text: &str) -> Result<Text> {
        unsafe {
            let layout = self.factory.CreateTextLayout(
                &text.encode_utf16().collect::<Vec<_>>(),
                &self.format,
                32768.0,
                32768.0,
            )?;
            // 部分中文字体也收录数字和组合框；系统默认回退会把 8 + VS16 + keycap
            // 分到不同字体。按完整字素指定 Emoji 字体，正文仍保留用户选择的字体。
            let mut start = 0;
            for cluster in text.graphemes(true) {
                let length = cluster.encode_utf16().count() as u32;
                let emoji = !cluster.contains('\u{fe0e}')
                    && (cluster.contains(['\u{fe0f}', '\u{20e3}'])
                        || cluster
                            .chars()
                            .any(|c| ('\u{1f000}'..='\u{1faff}').contains(&c)));
                if emoji {
                    layout.SetFontFamilyName(
                        w!("Segoe UI Emoji"),
                        DWRITE_TEXT_RANGE {
                            startPosition: start,
                            length,
                        },
                    )?;
                }
                start += length;
            }
            let mut metrics = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut metrics)?;
            layout.SetMaxWidth(metrics.widthIncludingTrailingWhitespace.max(1.0))?;
            layout.SetMaxHeight(metrics.height.max(1.0))?;
            let overhang = layout.GetOverhangMetrics()?;
            let left = overhang.left.max(0.0).ceil();
            let top = overhang.top.max(0.0).ceil();
            Ok(Text {
                layout,
                size: SIZE {
                    cx: (metrics.widthIncludingTrailingWhitespace + left + overhang.right.max(0.0))
                        .ceil() as i32,
                    cy: (metrics.height + top + overhang.bottom.max(0.0)).ceil() as i32,
                },
                offset: Vector2 { X: left, Y: top },
            })
        }
    }
}

fn color(c: COLORREF) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: (c.0 & 255) as f32 / 255.0,
        g: ((c.0 >> 8) & 255) as f32 / 255.0,
        b: ((c.0 >> 16) & 255) as f32 / 255.0,
        a: 1.0,
    }
}

fn bounds(r: &RECT) -> D2D_RECT_F {
    D2D_RECT_F {
        left: r.left as f32,
        top: r.top as f32,
        right: r.right as f32,
        bottom: r.bottom as f32,
    }
}

pub struct Painter {
    target: ID2D1DCRenderTarget,
    brush: ID2D1SolidColorBrush,
}

impl Painter {
    pub fn new(dc: HDC, area: &RECT, background: COLORREF) -> Result<Self> {
        unsafe {
            let factory: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let target = factory.CreateDCRenderTarget(&D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_IGNORE,
                },
                // 外层已经按实际屏幕 DPI 缩放坐标，不能让 Direct2D 再缩放一次。
                dpiX: 96.0,
                dpiY: 96.0,
                ..Default::default()
            })?;
            target.BindDC(dc, area)?;
            let brush = target.CreateSolidColorBrush(&color(background), None)?;
            target.BeginDraw();
            target.Clear(Some(&color(background)));
            Ok(Self { target, brush })
        }
    }

    pub fn border(&self, area: &RECT, c: COLORREF) {
        unsafe {
            self.brush.SetColor(&color(c));
            let mut r = bounds(area);
            r.left += 0.5;
            r.top += 0.5;
            r.right -= 0.5;
            r.bottom -= 0.5;
            self.target.DrawRectangle(&r, &self.brush, 1.0, None);
        }
    }

    pub fn fill_round(&self, area: &RECT, c: COLORREF, radius: f32) {
        unsafe {
            self.brush.SetColor(&color(c));
            self.target.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: bounds(area),
                    radiusX: radius,
                    radiusY: radius,
                },
                &self.brush,
            );
        }
    }

    pub fn text(&self, area: &RECT, text: &Text, c: COLORREF, center: bool) {
        let x = area.left
            + if center {
                ((area.right - area.left - text.size.cx) / 2).max(0)
            } else {
                0
            };
        let y = area.top + ((area.bottom - area.top - text.size.cy) / 2).max(0);
        unsafe {
            self.brush.SetColor(&color(c));
            // Windows 不默认启用彩色字形；这里必须显式打开，不能再退回 GDI 拆画 emoji。
            self.target.DrawTextLayout(
                Vector2 {
                    X: x as f32 + text.offset.X,
                    Y: y as f32 + text.offset.Y,
                },
                &text.layout,
                &self.brush,
                D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT,
            );
        }
    }

    pub fn finish(self) -> Result<()> {
        unsafe { self.target.EndDraw(None, None) }
    }
}
