//! 候选文字的系统排版和绘制。测量与绘制共用同一个 DirectWrite layout，
//! 让变体选择符、组合字符、ZWJ 表情和字体回退在两边得到同一组字形。
use unicode_segmentation::UnicodeSegmentation;
use windows::core::{w, Interface, Result, HSTRING, PCWSTR};
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
    DWriteCreateFactory, IDWriteFactory, IDWriteFactory2, IDWriteFontCollection,
    IDWriteFontFallback, IDWriteTextFormat, IDWriteTextLayout, IDWriteTextLayout2,
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD,
    DWRITE_LINE_METRICS, DWRITE_TEXT_METRICS, DWRITE_TEXT_RANGE, DWRITE_UNICODE_RANGE,
    DWRITE_WORD_WRAPPING_NO_WRAP,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::HDC;
use windows_numerics::Vector2;

pub struct Font {
    factory: IDWriteFactory,
    format: IDWriteTextFormat,
    fallback: Option<IDWriteFontFallback>,
}

pub struct Fonts {
    pub main: Font,
    pub small: Font,
    pub preedit: Font,
    pub hint: Font,
    pub caption: Font,
}

impl Fonts {
    pub fn new(size: f32, family: &str) -> Result<Self> {
        Self::styled(size, family, 0.0, "", 0.0)
    }

    pub fn styled(
        size: f32,
        family: &str,
        preedit_size: f32,
        fallbacks: &str,
        caption_size: f32,
    ) -> Result<Self> {
        let factory: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        let family = if family.trim().is_empty() {
            freetype_protocol::DEFAULT_FONT
        } else {
            family
        };
        let fallback = if fallbacks.is_empty() {
            None
        } else {
            Some(font_fallback(&factory, fallbacks)?)
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
                fallback: fallback.clone(),
            })
        };
        Ok(Self {
            main: make(size, DWRITE_FONT_WEIGHT_NORMAL)?,
            small: make((size * 0.78).round(), DWRITE_FONT_WEIGHT_NORMAL)?,
            preedit: make(
                if preedit_size > 0.0 {
                    preedit_size
                } else {
                    (size * 0.78).round()
                },
                DWRITE_FONT_WEIGHT_NORMAL,
            )?,
            hint: make(size, DWRITE_FONT_WEIGHT_SEMI_BOLD)?,
            caption: make(
                if caption_size > 0.0 {
                    caption_size
                } else {
                    size * 0.72
                },
                DWRITE_FONT_WEIGHT_NORMAL,
            )?,
        })
    }
}

fn font_fallback(factory: &IDWriteFactory, families: &str) -> Result<IDWriteFontFallback> {
    let factory: IDWriteFactory2 = factory.cast()?;
    let names: Vec<HSTRING> = families.split(',').map(HSTRING::from).collect();
    let names: Vec<_> = names.iter().map(|name| name.as_ptr()).collect();
    let ranges = [(0x2e80, 0x9fff), (0xf900, 0xfaff), (0x20000, 0x323af)]
        .map(|(first, last)| DWRITE_UNICODE_RANGE { first, last });
    unsafe {
        let builder = factory.CreateFontFallbackBuilder()?;
        builder.AddMapping(
            &ranges,
            &names,
            None::<&IDWriteFontCollection>,
            PCWSTR::null(),
            PCWSTR::null(),
            1.0,
        )?;
        builder.AddMappings(&factory.GetSystemFontFallback()?)?;
        builder.CreateFontFallback()
    }
}

/// 排好的一段文字。
///
/// 行框里的留白随字体变化，回退字体和不同字形的墨迹也不在同一高度。
/// 记录本段文字实际的墨迹与基线，供候选内容和图标在各自区域内对齐。
#[derive(Clone)]
pub struct Text {
    layout: IDWriteTextLayout,
    /// 行框加上溢出框外的墨迹；留位置、画的时候都按这个框。
    pub size: SIZE,
    offset: Vector2,
    /// 墨迹在框里的范围（像素，向外取整）。全是空白的文字没有墨迹，这时是整个框。
    pub ink: RECT,
    /// 基线在框里的高度。
    baseline: f32,
    /// 墨迹上下的中线在框里的高度。
    middle: f32,
}

impl Text {
    /// 墨迹在 `area` 里上下左右都居中时，框的左上角该在哪。
    pub fn centered_in(&self, area: &RECT) -> (f32, f32) {
        let x = (area.left + area.right - self.ink.left - self.ink.right) as f32 / 2.0;
        (x.round(), self.centered_y(area))
    }

    /// 对齐本段墨迹的中线；不拿汉字样本的基线套用到英文、标点和回退字形。
    pub fn centered_y(&self, area: &RECT) -> f32 {
        let middle = (area.top + area.bottom) as f32 / 2.0;
        // Direct2D 绘制时会对齐基线，先在这里吸附到同一个像素网格。
        (middle + self.baseline - self.middle).round() - self.baseline
    }

    /// 框的左上角在 (x, y) 时墨迹占的地方（向外取整）。
    pub fn ink_at(&self, x: f32, y: f32) -> RECT {
        RECT {
            left: (x + self.ink.left as f32).floor() as i32,
            top: (y + self.ink.top as f32).floor() as i32,
            right: (x + self.ink.right as f32).ceil() as i32,
            bottom: (y + self.ink.bottom as f32).ceil() as i32,
        }
    }
}

#[cfg(test)]
impl Text {
    pub(super) fn cluster_lengths(&self, capacity: usize) -> Result<Vec<u16>> {
        use windows::Win32::Graphics::DirectWrite::DWRITE_CLUSTER_METRICS;

        let mut metrics = vec![DWRITE_CLUSTER_METRICS::default(); capacity];
        let mut count = 0;
        unsafe {
            self.layout
                .GetClusterMetrics(Some(&mut metrics), &mut count)?
        };
        metrics.truncate(count as usize);
        Ok(metrics.into_iter().map(|cluster| cluster.length).collect())
    }
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
            if let Some(fallback) = &self.fallback {
                layout
                    .cast::<IDWriteTextLayout2>()?
                    .SetFontFallback(fallback)?;
            }
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
            let (width, height) = (
                metrics.widthIncludingTrailingWhitespace.max(1.0),
                metrics.height.max(1.0),
            );
            layout.SetMaxWidth(width)?;
            layout.SetMaxHeight(height)?;
            let overhang = layout.GetOverhangMetrics()?;
            let left = overhang.left.max(0.0).ceil();
            let top = overhang.top.max(0.0).ceil();
            let size = SIZE {
                cx: (width + left + overhang.right.max(0.0)).ceil() as i32,
                cy: (height + top + overhang.bottom.max(0.0)).ceil() as i32,
            };
            // 候选都只有一行（换行已经画成 ⏎）；万一有几行，按第一行的基线
            let mut lines = [DWRITE_LINE_METRICS::default()];
            let mut count = 0;
            let _ = layout.GetLineMetrics(Some(&mut lines), &mut count);
            let baseline = top + if count > 0 { lines[0].baseline } else { height };
            // 溢出量是从行框的边往外算的：负数表示墨迹在框里面
            let ink = D2D_RECT_F {
                left: left - overhang.left,
                top: top - overhang.top,
                right: left + width + overhang.right,
                bottom: top + height + overhang.bottom,
            };
            let blank = ink.right <= ink.left || ink.bottom <= ink.top;
            Ok(Text {
                layout,
                size,
                offset: Vector2 { X: left, Y: top },
                ink: if blank {
                    RECT {
                        left: 0,
                        top: 0,
                        right: size.cx,
                        bottom: size.cy,
                    }
                } else {
                    RECT {
                        left: ink.left.floor() as i32,
                        top: ink.top.floor() as i32,
                        right: ink.right.ceil() as i32,
                        bottom: ink.bottom.ceil() as i32,
                    }
                },
                baseline,
                middle: if blank {
                    size.cy as f32 / 2.0
                } else {
                    (ink.top + ink.bottom) / 2.0
                },
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

    /// 把文字画出来，框的左上角在 (x, y)。
    pub fn text(&self, x: f32, y: f32, text: &Text, c: COLORREF) {
        unsafe {
            self.brush.SetColor(&color(c));
            // Windows 不默认启用彩色字形；这里必须显式打开，不能再退回 GDI 拆画 emoji。
            self.target.DrawTextLayout(
                Vector2 {
                    X: x + text.offset.X,
                    Y: y + text.offset.Y,
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
