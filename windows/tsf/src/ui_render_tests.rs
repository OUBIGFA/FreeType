//! 在私有桌面离屏绘制真实候选窗；不注册输入法，不接触用户窗口或输入焦点。
use super::*;
use windows::Win32::Graphics::Gdi::{
    CreateDIBSection, GdiFlush, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
};
use windows::Win32::System::StationsAndDesktops::{
    CloseDesktop, CreateDesktopW, GetThreadDesktop, SetThreadDesktop, DESKTOP_CONTROL_FLAGS,
    DESKTOP_CREATEWINDOW, DESKTOP_READOBJECTS, DESKTOP_WRITEOBJECTS, HDESK,
};
use windows::Win32::System::Threading::GetCurrentThreadId;

struct PrivateDesktop {
    previous: HDESK,
    desktop: HDESK,
}

impl PrivateDesktop {
    fn new() -> Self {
        unsafe {
            let tid = GetCurrentThreadId();
            let previous = GetThreadDesktop(tid).unwrap();
            let name = HSTRING::from(format!("FreeType-render-{}-{tid}", std::process::id()));
            let desktop = CreateDesktopW(
                &name,
                PCWSTR::null(),
                None,
                DESKTOP_CONTROL_FLAGS(0),
                DESKTOP_CREATEWINDOW.0 | DESKTOP_READOBJECTS.0 | DESKTOP_WRITEOBJECTS.0,
                None,
            )
            .expect("必须能建立独立桌面，不能退回用户桌面测试");
            SetThreadDesktop(desktop).unwrap();
            Self { previous, desktop }
        }
    }
}

impl Drop for PrivateDesktop {
    fn drop(&mut self) {
        unsafe {
            SetThreadDesktop(self.previous).unwrap();
            CloseDesktop(self.desktop).unwrap();
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
        .find(|(_, part, _)| *part == Part::Candidate(3))
        .unwrap()
        .0;
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
        .find(|(_, part, _)| *part == Part::Label(3))
        .unwrap()
        .0;
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
                assert!(
                    text.size.cx as f32 <= 18.0 * scale * 1.8,
                    "完整表情不能拆成多个字符：{family} {scale} {emoji} {:?}",
                    text.size
                );
            }
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
                for (r, part, _) in &layout.parts {
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
    assert!(matches!(clicks.borrow().as_slice(), [Click::Select(1)]));
    window.update(&Frame::default(), None);
    assert!(!window.visible());
    set_click_handler(None);
}
