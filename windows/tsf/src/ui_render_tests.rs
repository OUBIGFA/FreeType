//! 在私有桌面离屏绘制真实候选窗；不注册输入法，不接触用户窗口或输入焦点。
use super::*;
use windows::Win32::Graphics::Gdi::{
    CreateDIBSection, GdiFlush, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
};
use windows::Win32::System::StationsAndDesktops::{
    CreateDesktopW, GetThreadDesktop, SetThreadDesktop, DESKTOP_CONTROL_FLAGS,
    DESKTOP_CREATEMENU, DESKTOP_CREATEWINDOW, DESKTOP_READOBJECTS, DESKTOP_WRITEOBJECTS, HDESK,
};
use windows::Win32::System::Threading::GetCurrentThreadId;

#[test]
fn candidate_text_palette_matches_styles_in_both_appearances() {
    fn luminance(color: COLORREF) -> f64 {
        let linear = |value: u32| {
            let value = f64::from(value) / 255.0;
            if value <= 0.04045 { value / 12.92 } else { ((value + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * linear(color.0 & 255) + 0.7152 * linear((color.0 >> 8) & 255)
            + 0.0722 * linear((color.0 >> 16) & 255)
    }
    fn contrast(first: COLORREF, second: COLORREF) -> f64 {
        let (first, second) = (luminance(first), luminance(second));
        (first.max(second) + 0.05) / (first.min(second) + 0.05)
    }
    for light in [true, false] {
        for style in ["pink", "blue", "green", "sogou"] {
            let palette = theme_for_appearance(style, light);
            if style == "green" {
                assert_eq!(palette.accent, Some(rgb(0x03, 0xd4, 0x68)));
                assert_eq!(palette.accent_text, rgb(0xff, 0xff, 0xff));
            }
            for (name, text, background) in [
                ("selected", palette.accent_text, palette.accent.unwrap_or(palette.background)),
                ("muted", palette.muted, palette.background),
                ("text", palette.text, palette.background),
            ] {
                let ratio = contrast(text, background);
                // 保留鲜亮绿底白字，同时防止对比度退回原先较浅的绿色。
                let minimum = if style == "green" && name == "selected" { 1.95 } else { 4.5 };
                assert!(ratio >= minimum, "{style} light={light} {name}: {ratio:.2}:1");
            }
        }
    }
}

pub(crate) struct PrivateDesktop {
    previous: HDESK,
}

impl PrivateDesktop {
    pub(crate) fn new() -> Self {
        unsafe {
            let tid = GetCurrentThreadId();
            let previous = GetThreadDesktop(tid).unwrap();
            // 候选窗口类按进程共享，首次注册的桌面必须与窗口类保持相同生命期。
            // 所有测试线程借用同一个私有桌面；测试进程退出时 Windows 一并释放桌面和类。
            // 每个用例仍销毁自己的窗口，下面的 SetThreadDesktop 会拒绝遗留窗口或钩子。
            static DESKTOP: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
            let desktop = HDESK(*DESKTOP.get_or_init(|| {
                let name = HSTRING::from(format!("FreeType-render-{}", std::process::id()));
                CreateDesktopW(
                    &name,
                    PCWSTR::null(),
                    None,
                    DESKTOP_CONTROL_FLAGS(0),
                    DESKTOP_CREATEMENU.0 | DESKTOP_CREATEWINDOW.0 | DESKTOP_READOBJECTS.0 | DESKTOP_WRITEOBJECTS.0,
                    None,
                )
                .expect("必须能建立独立桌面，不能退回用户桌面测试").0 as usize
            }) as *mut _);
            SetThreadDesktop(desktop).unwrap();
            Self { previous }
        }
    }
}

impl Drop for PrivateDesktop {
    fn drop(&mut self) {
        unsafe {
            SetThreadDesktop(self.previous).unwrap();
        }
    }
}

fn rendered(view: View, scale: f32) -> (Layout, Vec<u8>) {
    unsafe {
        let dc = CreateCompatibleDC(None);
        let fonts = Fonts::new(view.font_size.max(12) as f32 * scale, &view.font_family).unwrap();
        let layout = layout(&view, &fonts, scale).unwrap();
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: layout.width,
                biHeight: -layout.height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let bitmap = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0).unwrap();
        let original = SelectObject(dc, bitmap.into());
        let data = Data {
            view,
            layout,
            scale,
            hovered: None,
            pressed: None,
            long_fired: false,
            wheel_delta: 0,
            painted_revision: 0,
            anchor: None,
        };
        draw(dc, &data).unwrap();
        assert!(GdiFlush().as_bool());
        let pixels = std::slice::from_raw_parts(
            bits as *const u8,
            (data.layout.width * data.layout.height * 4) as usize,
        )
        .to_vec();
        SelectObject(dc, original);
        assert!(DeleteObject(bitmap.into()).as_bool());
        assert!(DeleteDC(dc).as_bool());
        (data.layout, pixels)
    }
}

fn save_render(name: &str, layout: &Layout, pixels: &[u8]) {
    if let Some(dir) = std::env::var_os("FREETYPE_RENDER_DIR") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut ppm = format!("P6\n{} {}\n255\n", layout.width, layout.height).into_bytes();
        for pixel in pixels.chunks_exact(4) {
            ppm.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
        }
        std::fs::write(dir.join(format!("{name}.ppm")), ppm).unwrap();
    }
}

#[test]
fn keycap_candidate_is_one_colored_glyph_with_clear_neighbors() {
    let _desktop = PrivateDesktop::new();
    let view = View {
        preedit: "ba".into(),
        candidates: ["巴", "把", "八", "8️⃣", "白", "班", "爸"]
            .map(str::to_string)
            .to_vec(),
        cursor: Some(0),
        font_size: 18,
        font_family: "Microsoft YaHei".into(),
        has_next: true,
        ..View::default()
    };
    let (layout, pixels) = rendered(view, 2.0);
    save_render("keycap", &layout, &pixels);
    let body = layout
        .parts
        .iter()
        .find(|piece| piece.part == Part::Candidate(3))
        .unwrap()
        .ink;
    let mut blue = 0;
    for y in body.top..body.bottom {
        for x in body.left..body.right {
            let offset = ((y * layout.width + x) * 4) as usize;
            let (b, g, r) = (
                pixels[offset] as i32,
                pixels[offset + 1] as i32,
                pixels[offset + 2] as i32,
            );
            if b > r + 60 && b > g + 35 {
                blue += 1;
            }
        }
    }
    let area = (body.right - body.left) * (body.bottom - body.top);
    assert!(
        blue * 10 > area,
        "8️⃣ 应画成蓝底白字的完整按键表情，不能拆成 8 和方框；蓝色像素 {blue}/{area}"
    );
    let label = layout
        .parts
        .iter()
        .find(|piece| piece.part == Part::Label(3))
        .unwrap()
        .ink;
    assert!(label.right < body.left);
    assert!(body.right < layout.items[4].left);
}

#[test]
fn emoji_and_text_fit_in_all_candidate_layouts_fonts_and_scales() {
    let _desktop = PrivateDesktop::new();
    for family in ["Microsoft YaHei", "SimSun", "Arial"] {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let fonts = Fonts::new(18.0 * scale, family).unwrap();
            for emoji in ["1️⃣", "8️⃣", "🔟", "👍🏽", "❤️", "👨‍👩‍👧‍👦", "🧑🏽‍💻"]
            {
                let text = fonts.main.text(emoji).unwrap();
                let length = emoji.encode_utf16().count();
                // Shaping must keep the emoji together; glyph width varies between system fonts.
                assert_eq!(
                    text.cluster_lengths(length).unwrap(),
                    vec![length as u16],
                    "完整表情必须保持单一排版簇：{family} {scale} {emoji} {:?}",
                    text.size,
                );
            }
            let separate = "1️⃣8️⃣";
            assert_eq!(
                fonts.main.text(separate).unwrap()
                    .cluster_lengths(separate.encode_utf16().count()).unwrap(),
                vec![3, 3],
                "两个独立表情不能被合并成同一排版簇：{family} {scale}",
            );
            for (name, vertical, panel) in [
                ("row", false, Panel::Normal),
                ("column", true, Panel::Normal),
                ("grid", false, Panel::Grid),
            ] {
                let view = View {
                    preedit: "emoji".into(),
                    candidates: [
                        "你",
                        "1️⃣",
                        "8️⃣",
                        "🔟",
                        "👍🏽",
                        "❤️",
                        "👨‍👩‍👧‍👦",
                        "🧑🏽‍💻",
                        "e\u{301}",
                        "𠮷",
                    ]
                    .map(str::to_string)
                    .to_vec(),
                    labels: (0..10)
                        .map(|i| {
                            if panel == Panel::Grid && i >= 5 {
                                String::new()
                            } else {
                                ((i + 1) % 10).to_string()
                            }
                        })
                        .collect(),
                    font_family: family.into(),
                    font_size: 18,
                    vertical,
                    panel,
                    columns: 5,
                    cursor: Some(0),
                    has_prev: true,
                    has_next: true,
                    ..View::default()
                };
                let (layout, pixels) = rendered(view, scale);
                for Piece { ink: r, part, .. } in &layout.parts {
                    assert!(
                        r.left >= 0
                            && r.top >= 0
                            && r.right <= layout.width
                            && r.bottom <= layout.height,
                        "文字不能超出窗口"
                    );
                    if let Part::Candidate(i) = part {
                        let item = layout.items[*i];
                        assert!(
                            r.left >= item.left
                                && r.right <= item.right
                                && r.top >= item.top
                                && r.bottom <= item.bottom,
                            "候选正文必须完整落在自己的点击区域内"
                        );
                    }
                }
                for pair in layout.items.windows(2) {
                    assert!(
                        pair[0].right <= pair[1].left || pair[0].bottom <= pair[1].top,
                        "候选点击区域不能重叠"
                    );
                }
                save_render(&format!("{name}-{family}-{scale}"), &layout, &pixels);
            }
        }
    }
}

/// 一块区域里和底色 `ground` 明显不同的像素（文字的墨迹）的外框；没有墨迹返回 None。
///
/// 直接数画出来的像素，不用排版时算的墨迹范围：排版算错了这里才看得出来。
fn ink_bounds(layout: &Layout, pixels: &[u8], area: RECT, ground: [i32; 3]) -> Option<RECT> {
    let mut ink: Option<RECT> = None;
    for y in area.top..area.bottom {
        for x in area.left..area.right {
            let pixel = pixel_at(layout, pixels, x, y);
            if (0..3).map(|c| (pixel[c] - ground[c]).abs()).max().unwrap() < 96 {
                continue;
            }
            let b = ink.get_or_insert(rect(x, y, 1, 1));
            b.left = b.left.min(x);
            b.top = b.top.min(y);
            b.right = b.right.max(x + 1);
            b.bottom = b.bottom.max(y + 1);
        }
    }
    ink
}

fn pixel_at(layout: &Layout, pixels: &[u8], x: i32, y: i32) -> [i32; 3] {
    let offset = ((y * layout.width + x) * 4) as usize;
    [pixels[offset] as i32, pixels[offset + 1] as i32, pixels[offset + 2] as i32]
}

#[test]
fn selected_candidate_ink_is_centered_in_its_block_for_any_font() {
    let _desktop = PrivateDesktop::new();
    // 行框、基线位置各不相同的字体：中文黑体、宋体、楷体、仿宋，西文无衬线、等宽、衬线，日文、韩文。
    // 没装的字体系统会换成别的，同样要居中。
    let families = [
        "Microsoft YaHei", "SimSun", "KaiTi", "FangSong", "SimHei", "DengXian", "Microsoft JhengHei",
        "Segoe UI", "Arial", "Consolas", "Times New Roman", "Yu Gothic", "Malgun Gothic", "Cascadia Mono",
        "Georgia", "Impact", "MingLiU", "Courier New", "Open Sans",
    ];
    // 第一组复现 Open Sans 下「1 字体」偏上的截图；其余覆盖不同高度、回退和混排。
    let samples = ["字体", "你好", "小", "gyp", "FreeType", "，", "8️⃣", "混排🧑🏽‍💻hj"];
    let mut failures = Vec::new();
    for family in families {
        for scale in [1.0f32, 1.25, 1.5, 2.0] {
            for (sample, word) in samples.into_iter().enumerate() {
                let view = View {
                    preedit: "zi't".into(),
                    candidates: [word, "姿态", "字帖", "自体", "自他"].map(str::to_string).to_vec(),
                    labels: vec![(sample + 1).to_string(), "2".into(), "3".into(), "4".into(), "5".into()],
                    cursor: Some(0),
                    font_size: 18,
                    font_family: family.into(),
                    candidate_style: if sample == 0 { "green" } else { "pink" }.into(),
                    ..View::default()
                };
                let (layout, pixels) = rendered(view, scale);
                save_render(&format!("center-{family}-{scale}-{sample}"), &layout, &pixels);
                let item = layout.items[0];
                let part = |wanted: Part| {
                    layout.parts.iter().find(|piece| piece.part == wanted).unwrap().ink
                };
                let (label, body) = (part(Part::Label(0)), part(Part::Candidate(0)));
                // 避开色块四个圆角：深色主题下，圆角外的底色也可能被识别为墨迹。
                let inset = (4.0 * scale).ceil() as i32;
                let ground = pixel_at(&layout, &pixels, item.left + 2, (item.top + item.bottom) / 2);
                let split = (label.right + body.left) / 2;
                let strip = |left: i32, right: i32| rect(left, item.top, right - left, item.bottom - item.top);
                let label_ink = ink_bounds(&layout, &pixels, strip(item.left + inset, split), ground);
                let body_ink = ink_bounds(&layout, &pixels, strip(split, item.right - inset), ground);
                let (Some(label_ink), Some(body_ink)) = (label_ink, body_ink) else {
                    panic!("{family} x{scale}「{word}」：色块里没有画出标号或正文");
                };
                let top_diff = |ink: RECT| ink.top + ink.bottom - item.top - item.bottom;
                let side_diff = label_ink.left + body_ink.right - item.left - item.right;
                let body_diff = top_diff(body_ink);
                let label_diff = top_diff(label_ink);
                // 以半个像素为单位，允许字体栅格化和抗锯齿的边缘造成轻微误差。
                if body_diff.abs() > 3 || label_diff.abs() > 3 || side_diff.abs() > 2 {
                    failures.push(format!(
                        "{family} x{scale}「{word}」：正文偏 {body_diff}，序号偏 {label_diff}，水平偏 {side_diff} 个半像素"
                    ));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "选中的候选没有在色块里居中（{} 例，以下是前 24 例）：\n{}",
        failures.len(),
        failures.iter().take(24).map(String::as_str).collect::<Vec<_>>().join("\n"),
    );
}

#[test]
fn centered_grid_cells_and_arrows_keep_ink_in_the_middle() {
    let _desktop = PrivateDesktop::new();
    for family in ["Microsoft YaHei", "KaiTi", "Yu Gothic", "Consolas"] {
        let view = View {
            preedit: "a".into(),
            candidates: ["国", "，", "果然"].map(str::to_string).to_vec(),
            labels: vec![String::new(); 3],
            cursor: Some(0),
            font_size: 18,
            font_family: family.into(),
            panel: Panel::Grid,
            columns: 3,
            has_prev: true,
            has_next: true,
            ..View::default()
        };
        let (layout, pixels) = rendered(view, 2.0);
        save_render(&format!("grid-{family}"), &layout, &pixels);
        // 格子比「国」宽（按「果然」定的）：选中的「国」左右留的空要一样
        let item = layout.items[0];
        let ground = pixel_at(&layout, &pixels, item.left + 2, (item.top + item.bottom) / 2);
        let ink = ink_bounds(&layout, &pixels, item, ground).unwrap();
        let (left, right) = (ink.left - item.left, item.right - ink.right);
        let (top, bottom) = (ink.top - item.top, item.bottom - ink.bottom);
        assert!((left - right).abs() <= 2 && (top - bottom).abs() <= 2, "{family}: 左 {left} 右 {right} 上 {top} 下 {bottom}");
        // 翻页箭头在自己的格子里上下左右居中
        for (area, _, _) in &layout.arrows {
            let ground = pixel_at(&layout, &pixels, area.left + 1, area.top + 1);
            let ink = ink_bounds(&layout, &pixels, *area, ground).unwrap();
            let (left, right) = (ink.left - area.left, area.right - ink.right);
            let (top, bottom) = (ink.top - area.top, area.bottom - ink.bottom);
            assert!((left - right).abs() <= 2 && (top - bottom).abs() <= 2, "{family} 箭头: 左 {left} 右 {right} 上 {top} 下 {bottom}");
        }
    }
}

#[test]
fn candidate_window_updates_clicks_and_hides_on_a_private_desktop() {
    use std::rc::Rc;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::SendMessageW;
    let _desktop = PrivateDesktop::new();
    let instance = HINSTANCE(unsafe { GetModuleHandleW(None).unwrap() }.0);
    let window = CandidateWindow::new(instance).expect("创建真实候选窗");
    let clicks = Rc::new(RefCell::new(Vec::new()));
    let captured = clicks.clone();
    set_click_handler(Some(Box::new(move |click| {
        captured.borrow_mut().push(click)
    })));
    let frame = Frame {
        preedit: "ba".into(),
        candidates: vec!["八".into(), "8️⃣".into()],
        cursor: Some(0),
        font_size: 18,
        font_family: "Microsoft YaHei".into(),
        has_next: true,
        ..Frame::default()
    };
    window.update(&frame, Some(rect(100, 100, 2, 20)));
    assert!(window.visible());
    let mut first = RECT::default();
    unsafe {
        GetWindowRect(window.hwnd, &mut first).unwrap();
    }
    window.update(&frame, Some(rect(100, 100, 2, 20)));
    let mut again = RECT::default();
    unsafe {
        GetWindowRect(window.hwnd, &mut again).unwrap();
    }
    assert_eq!(first, again, "同一候选和位置不能先缩小或漂移");
    let item = data_of(window.hwnd).unwrap().layout.items[1];
    let point =
        LPARAM((((item.top + item.bottom) / 2) << 16 | ((item.left + item.right) / 2)) as isize);
    unsafe {
        SendMessageW(window.hwnd, WM_LBUTTONDOWN, Some(WPARAM(0)), Some(point));
        SendMessageW(window.hwnd, WM_LBUTTONUP, Some(WPARAM(0)), Some(point));
    }
    assert!(matches!(clicks.borrow().as_slice(), [Click::Candidate { action: CandidateAction::Select { index: 1 }, .. }]));
    window.update(&Frame::default(), None);
    assert!(!window.visible());
    set_click_handler(None);
}
