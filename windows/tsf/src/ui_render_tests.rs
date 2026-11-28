//! 在私有桌面离屏绘制真实候选窗；不注册输入法，不接触用户窗口或输入焦点。
use super::*;
use windows::Win32::Graphics::Gdi::{
    CreateDIBSection, GdiFlush, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
};
use windows::Win32::System::StationsAndDesktops::{
    CreateDesktopW, GetThreadDesktop, SetThreadDesktop, DESKTOP_CONTROL_FLAGS,
    DESKTOP_CREATEMENU, DESKTOP_HOOKCONTROL, DESKTOP_CREATEWINDOW, DESKTOP_READOBJECTS, DESKTOP_WRITEOBJECTS, HDESK,
};
use windows::Win32::System::Threading::GetCurrentThreadId;

#[test]
fn sentence_editor_does_not_change_candidate_geometry_or_content() {
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    let _desktop = PrivateDesktop::new();
    let instance = HINSTANCE(unsafe { GetModuleHandleW(None).unwrap() }.0);
    let window = CandidateWindow::new(instance).unwrap();
    for (vertical, editor_above) in [(false, false), (false, true), (true, false), (true, true)] {
        window.hide();
        let mut frame = Frame { preedit: "wo'xi'huan'ni".into(), candidates: vec!["我喜欢你".into(), "我".into()],
            vertical, sentence_editor_above: editor_above, font_size: 20, font_family: "Microsoft YaHei".into(), page_revision: 1, ..Default::default() };
        let caret = rect(300, 400, 2, 24);
        window.update(&frame, Some(caret));
        let mut original = RECT::default();
        unsafe { GetWindowRect(window.hwnd, &mut original).unwrap(); }
        let items = data_of(window.hwnd).unwrap().layout.items.clone();
        for text in ["我喜欢你", "这是一个比普通候选窗更长的整句编辑区域"] {
            frame.sentence = Some(freetype_protocol::SentenceView { text: text.into(), raw: "woxihuanni".into(),
                blocks: vec![freetype_protocol::SentenceBlock { text: text.into(), to: 10, ..Default::default() }],
                cursor: Some(10), alternatives: vec!["局部备选".into()], ..Default::default() });
            frame.page_revision += 1;
            window.update(&frame, Some(caret));
            let mut current = RECT::default();
            unsafe { GetWindowRect(window.hwnd, &mut current).unwrap(); }
            assert_eq!(current, original, "编辑区不能改变普通候选窗的位置或尺寸");
            let data = data_of(window.hwnd).unwrap();
            assert_eq!(data.layout.items, items);
            assert!(data.layout.parts.iter().any(|p| p.part == Part::Preedit));
            assert_eq!(data.view.candidates, frame.candidates);
            let editor = window.editor.borrow().as_ref().unwrap().hwnd;
            let mut above = RECT::default();
            unsafe {
                assert!(IsWindowVisible(editor).as_bool());
                GetWindowRect(editor, &mut above).unwrap();
                assert_eq!(GetWindowLongPtrW(editor, GWLP_HWNDPARENT), GetWindowLongPtrW(window.hwnd, GWLP_HWNDPARENT));
                use windows::Win32::UI::WindowsAndMessaging::{GetWindow, GW_HWNDPREV, GetClassLongW, GCL_STYLE};
                assert_eq!(GetWindow(editor, GW_HWNDPREV).unwrap(), window.hwnd, "候选窗始终在辅助区之上");
                assert_eq!(GetClassLongW(editor, GCL_STYLE) & CS_DROPSHADOW.0, 0, "辅助区不得投影");
            }
            if editor_above { assert!(above.bottom < original.top); }
            else { assert!(above.top > original.bottom); }
            if !vertical {
                let (layout, pixels) = rendered(data.view.clone(), data.scale);
                save_render("ordinary-with-editor", &layout, &pixels);
                let editor = data_of(editor).unwrap();
                let (layout, pixels) = rendered(editor.view.clone(), editor.scale);
                save_render(if editor_above { "independent-editor-above" } else { "independent-editor-below" }, &layout, &pixels);
            }
        }
        frame.sentence = None;
        window.update(&frame, None);
        let editor = window.editor.borrow().as_ref().unwrap().hwnd;
        assert!(!unsafe { IsWindowVisible(editor).as_bool() });
        window.reposition(rect(320, 420, 2, 24));
        assert!(!unsafe { IsWindowVisible(editor).as_bool() }, "关闭后挪动光标不能复活编辑区");
    }
}

#[test]
fn sentence_editor_lifecycle_and_pointer_routes_are_independent() {
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{SendMessageW, IsWindow};
    let _desktop = PrivateDesktop::new();
    let instance = HINSTANCE(unsafe { GetModuleHandleW(None).unwrap() }.0);
    let owner = popup(instance, CLASS_NAME, None).unwrap();
    let mut window = CandidateWindow::new(instance).unwrap();
    window.set_owner(owner).unwrap();
    let clicks = Rc::new(RefCell::new(Vec::new()));
    let capture = clicks.clone();
    set_click_handler(Some(Box::new(move |c| capture.borrow_mut().push(c))));
    let mut frame = Frame { preedit: "ni'hao".into(), candidates: vec!["你好".into(), "你".into()],
        has_next: true, candidate_wheel: true, sentence_editor_above: true, font_size: 20, font_family: "Microsoft YaHei".into(), page_revision: 31,
        sentence: Some(freetype_protocol::SentenceView { raw: "nihao".into(), text: "你好".into(), cursor: Some(2),
            blocks: vec![freetype_protocol::SentenceBlock { text: "你".into(), to: 2, ..Default::default() }],
            alternatives: vec!["泥".into()], has_next: true, ..Default::default() }), ..Default::default() };
    window.update(&frame, Some(rect(300, 400, 2, 24)));
    let editor = window.editor.borrow().as_ref().unwrap().hwnd;
    let point = |r: RECT| LPARAM((((r.top+r.bottom)/2)<<16 | ((r.left+r.right)/2)) as isize);
    let click = |hwnd: HWND, p: LPARAM| unsafe {
        SendMessageW(hwnd, WM_LBUTTONDOWN, Some(WPARAM(0)), Some(p));
        SendMessageW(hwnd, WM_LBUTTONUP, Some(WPARAM(0)), Some(p));
    };
    for (hwnd, expected) in [(window.hwnd, CandidateAction::Select { index: 0 }), (editor, CandidateAction::SentenceChoose { index: 0 })] {
        clicks.borrow_mut().clear();
        click(hwnd, point(data_of(hwnd).unwrap().layout.items[0]));
        assert_eq!(*clicks.borrow(), vec![Click::Candidate { action: expected, revision: 31 }]);
    }
    for (hwnd, expected) in [(window.hwnd, CandidateAction::Page { next: true }), (editor, CandidateAction::SentencePage { next: true })] {
        clicks.borrow_mut().clear();
        unsafe { SendMessageW(hwnd, WM_MOUSEWHEEL, Some(WPARAM(((-120i16 as u16 as u32)<<16) as usize)), Some(LPARAM(0))); }
        assert_eq!(*clicks.borrow(), vec![Click::Candidate { action: expected, revision: 31 }]);
    }
    // 更新后的页面不能接收上一页按下、这一页松开的词块点击。
    clicks.borrow_mut().clear();
    let block = point(data_of(editor).unwrap().layout.sentence_hits[0].0);
    unsafe { SendMessageW(editor, WM_LBUTTONDOWN, Some(WPARAM(0)), Some(block)); }
    frame.page_revision += 1;
    window.update(&frame, Some(rect(300, 400, 2, 24)));
    unsafe { SendMessageW(editor, WM_LBUTTONUP, Some(WPARAM(0)), Some(block)); }
    assert!(clicks.borrow().is_empty());
    // 顶部空间不足只收起辅助窗；主窗仍按原来的光标规则定位。
    let work = work_area(300, 400);
    window.reposition(rect(work.left+20, work.top, 2, 24));
    assert!(!unsafe { IsWindowVisible(editor).as_bool() });
    let mut r = RECT::default();
    unsafe { GetWindowRect(window.hwnd, &mut r).unwrap(); }
    assert_eq!(r.top, work.top+28);
    window.reposition(rect(300, 400, 2, 24));
    assert!(unsafe { IsWindowVisible(editor).as_bool() });
    // 下方不足也只收起辅助窗；候选窗保持普通翻转后的矩形。
    frame.sentence_editor_above = false;
    frame.page_revision += 1;
    let bottom = rect(work.left + 20, work.bottom - 24, 2, 24);
    window.update(&frame, Some(bottom));
    assert!(!unsafe { IsWindowVisible(editor).as_bool() });
    frame.sentence_editor_above = true;
    window.update(&frame, Some(rect(300, 400, 2, 24)));
    window.hide();
    assert!(!window.visible());
    assert!(!unsafe { IsWindowVisible(editor).as_bool() });
    assert_eq!(data_of(editor).unwrap().painted_revision, 0);
    window.set_owner(owner).unwrap();
    window.update(&frame, Some(rect(300, 400, 2, 24)));
    unsafe { DestroyWindow(owner).unwrap(); }
    assert!(!unsafe { IsWindow(Some(editor)).as_bool() });
    window.set_owner(HWND::default()).unwrap();
    window.update(&frame, Some(rect(300, 400, 2, 24)));
    assert!(window.visible());
    assert!(window.editor.borrow().as_ref().unwrap().alive.get());
    set_click_handler(None);
}

#[test]
fn prediction_highlight_preserves_window_and_item_rectangles() {
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    let _desktop = PrivateDesktop::new();
    let instance = HINSTANCE(unsafe { GetModuleHandleW(None).unwrap() }.0);
    let window = CandidateWindow::new(instance).unwrap();
    for vertical in [false, true] {
        window.hide();
        let mut frame = Frame {
            candidates: vec!["好".into(), "一起去".into(), "星期六下午".into()],
            labels: vec![String::new(); 3],
            cursor: Some(0), predict: true, vertical, page_revision: 1,
            font_size: 20, font_family: "Microsoft YaHei".into(),
            ..Frame::default()
        };
        window.update(&frame, Some(rect(140, 150, 2, 24)));
        let mut original = RECT::default();
        unsafe { GetWindowRect(window.hwnd, &mut original).unwrap(); }
        let items = data_of(window.hwnd).unwrap().layout.items.clone();
        for cursor in [1, 2, 1, 0] {
            frame.cursor = Some(cursor);
            frame.page_revision += 1;
            // A rich editor may report a different text extent during an unchanged
            // prediction page. Moving the highlight must not use it to place the popup.
            window.update(&frame, Some(rect(180 + cursor as i32 * 20, 175, 2, 31)));
            window.reposition(rect(310, 280, 2, 20));
            let mut current = RECT::default();
            unsafe { GetWindowRect(window.hwnd, &mut current).unwrap(); }
            assert_eq!(current, original, "同一批联想只移动高亮，窗口位置和尺寸保持不变");
            let data = data_of(window.hwnd).unwrap();
            assert_eq!(data.layout.items, items, "每个词的点击区域也保持不变");
            assert_eq!(data.view.cursor, Some(cursor));
            assert_eq!(data.painted_revision, frame.page_revision);
        }
        frame.commit = Some("一起去".into());
        frame.candidates = vec!["公园".into(), "吃饭".into()];
        frame.cursor = Some(0);
        window.update(&frame, Some(rect(400, 200, 2, 24)));
        let mut next = RECT::default();
        unsafe { GetWindowRect(window.hwnd, &mut next).unwrap(); }
        assert_ne!(next, original, "上屏后新的联想页应使用新的内容和位置");
    }
}

#[test]
fn sentence_blocks_and_raw_keys_have_real_layout_and_pointer_targets(){
    let _desktop=PrivateDesktop::new();
    let sentence=freetype_protocol::SentenceView{text:"我喜欢你".into(),raw:"woxihuanni".into(),cursor:Some(8),blocks:vec![
        freetype_protocol::SentenceBlock{from:0,to:2,text:"我".into(),confirmed:true,..Default::default()},
        freetype_protocol::SentenceBlock{from:2,to:8,text:"喜欢".into(),focused:true,..Default::default()},
        freetype_protocol::SentenceBlock{from:8,to:10,text:"你".into(),..Default::default()},
    ],alternatives:vec!["西环".into()],..Default::default()};
    for scale in [1.0,1.5,2.0] {
        let view=View{sentence:Some(sentence.clone()),candidates:vec!["西环".into(),"西幻".into()],font_size:20,font_family:"Microsoft YaHei".into(),available_width:600,revision:4,..Default::default()};
        let (layout,pixels)=rendered(view,scale);
        assert_eq!(layout.sentence_hits.len(),3);assert_eq!(layout.letter_hits.len(),10);
        assert!(layout.sentence_hits.iter().all(|(r,_)|r.left>=0&&r.right<=layout.width&&r.bottom<=layout.height));
        assert_eq!(layout.sentence_hits[1].1,8);
        save_render(&format!("sentence-{scale}"),&layout,&pixels);
    }
}

#[test]
fn sentence_pointer_selects_a_block_without_committing_the_sentence(){
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::SendMessageW;
    let _desktop=PrivateDesktop::new();let instance=HINSTANCE(unsafe{GetModuleHandleW(None).unwrap()}.0);
    let window=CandidateWindow::new(instance).unwrap();let clicks=Rc::new(RefCell::new(Vec::new()));let capture=clicks.clone();
    set_click_handler(Some(Box::new(move|c|capture.borrow_mut().push(c))));
    let frame=Frame{sentence:Some(freetype_protocol::SentenceView{text:"你好".into(),raw:"nihao".into(),blocks:vec![freetype_protocol::SentenceBlock{from:0,to:5,text:"你好".into(),..Default::default()}],..Default::default()}),candidates:vec!["你好".into()],font_size:20,font_family:"Microsoft YaHei".into(),page_revision:17,..Default::default()};
    window.update(&frame,Some(rect(100,400,2,24)));
    let editor = window.editor.borrow().as_ref().unwrap().hwnd;
    let area=data_of(editor).unwrap().layout.sentence_hits[0].0;
    let point=LPARAM((((area.top+area.bottom)/2)<<16|((area.left+area.right)/2))as isize);
    unsafe{SendMessageW(editor,WM_LBUTTONDOWN,Some(WPARAM(0)),Some(point));SendMessageW(editor,WM_LBUTTONUP,Some(WPARAM(0)),Some(point));}
    assert!(matches!(clicks.borrow().as_slice(),[Click::Candidate{action:CandidateAction::SentenceFocus{at:5},..}]));
    window.hide();set_click_handler(None);
}

#[test]
fn dictionary_annotations_fit_screen_width_without_changing_commit_candidates() {
    let _desktop = PrivateDesktop::new();
    for scale in [1.0, 1.5, 2.0] {
        let view = View { candidates: vec!["connectivity".into(), "in spite of".into(), "晶".into()],
            annotations: vec!["kənekˈtɪvəti  连通性；连接性能".into(), "尽管；不顾；虽然".repeat(30), "jīng".into()],
            vertical:true, cursor:Some(1), font_size:20, font_family:"Microsoft YaHei".into(), available_width:600,
            ..View::default() };
        let (layout, pixels) = rendered(view.clone(), scale);
        assert!(layout.width <= 600, "长释义不可撑出显示范围：{}", layout.width);
        assert_eq!(layout.items.len(), 3);
        assert!(!pixels.is_empty());
        assert_eq!(view.candidates, ["connectivity", "in spite of", "晶"]);
    }
}

#[test]
fn candidate_text_palette_matches_styles_in_both_appearances() {
    for light in [true, false] {
        for style in ["pink", "blue", "green", "sogou"] {
            let palette = theme_for_appearance(style, None, light);
            if style == "pink" {
                assert_eq!(palette.accent, Some(rgb(0xff, 0xb3, 0xfa)));
                assert_eq!(palette.accent_text, rgb(0x18, 0x18, 0x1b));
            }
            if style == "green" {
                assert_eq!(palette.accent, Some(rgb(0x03, 0xd4, 0x68)));
                assert_eq!(palette.accent_text, rgb(0xff, 0xff, 0xff));
            }
            for (name, text, background) in [
                ("selected", palette.accent_text, palette.accent.unwrap_or(palette.background)),
                ("muted", palette.muted, palette.background),
                ("text", palette.text, palette.background),
                ("emphasis", palette.emphasis, palette.background),
            ] {
                let ratio = contrast_ratio(text, background);
                // 保留鲜亮绿底白字，同时防止对比度退回原先较浅的绿色。
                let minimum = if style == "green" && name == "selected" { 1.95 } else { 4.5 };
                assert!(ratio >= minimum, "{style} light={light} {name}: {ratio:.2}:1");
            }
        }
    }
    let custom_skin = CandidateSkin {
        background: "#fbf8f1".into(),
        border: "#e2d9c8".into(),
        text: "#2c261f".into(),
        muted: "#7a7062".into(),
        accent: "#b45309".into(),
        accent_text: "#ffffff".into(),
        accent_fill: true,
    };
    let custom = theme_for_appearance("custom", Some(&custom_skin), true);
    assert_eq!(custom.background, rgb(0xfb, 0xf8, 0xf1));
    assert_eq!(custom.border, rgb(0xe2, 0xd9, 0xc8));
    assert_eq!(custom.text, rgb(0x2c, 0x26, 0x1f));
    assert_eq!(custom.muted, rgb(0x7a, 0x70, 0x62));
    assert_eq!(custom.accent, Some(rgb(0xb4, 0x53, 0x09)));
    assert_eq!(custom.accent_text, rgb(0xff, 0xff, 0xff));
    assert_eq!(custom.hover, blend_rgb(custom.background, custom.text, 0.08));
    assert_eq!(custom.emphasis, rgb(0xb4, 0x53, 0x09));

    let no_fill = CandidateSkin { accent_fill: false, ..custom_skin.clone() };
    let no_fill_theme = theme_for_appearance("custom", Some(&no_fill), true);
    assert_eq!(no_fill_theme.accent, None);
    assert_eq!(no_fill_theme.emphasis, no_fill_theme.text, "浅色高亮字对比度不足时提示字应回退到正文色");
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
                    // 联想点击监视要在桌面上装本线程鼠标钩子
                    DESKTOP_CREATEMENU.0 | DESKTOP_CREATEWINDOW.0 | DESKTOP_HOOKCONTROL.0
                        | DESKTOP_READOBJECTS.0 | DESKTOP_WRITEOBJECTS.0,
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
        let fonts = Fonts::styled(view.font_size.max(12) as f32 * scale, &view.font_family,
            view.preedit_size as f32 * scale, &view.font_fallbacks,
            (view.font_size as f32 * 0.72).clamp(11.0, 14.0) * scale).unwrap();
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
            alive: Rc::new(Cell::new(true)),
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

#[test]
fn custom_candidate_skin_renders_configured_colors_and_block_toggle() {
    let _desktop = PrivateDesktop::new();
    let assert_close = |actual: [i32; 3], expected: [i32; 3], what: &str| {
        for (a, e) in actual.iter().zip(expected.iter()) {
            assert!((a - e).abs() <= 2, "{what}: actual={actual:?} expected={expected:?}");
        }
    };
    let skin = CandidateSkin {
        background: "#fbf8f1".into(),
        border: "#e2d9c8".into(),
        text: "#2c261f".into(),
        muted: "#7a7062".into(),
        accent: "#b45309".into(),
        accent_text: "#ffffff".into(),
        accent_fill: true,
    };
    let view = View {
        preedit: "ni".into(),
        candidates: vec!["你".into(), "拟".into()],
        cursor: Some(0),
        font_size: 18,
        font_family: "Microsoft YaHei".into(),
        candidate_style: "custom".into(),
        candidate_skin: Some(skin.clone()),
        ..View::default()
    };
    let (layout, pixels) = rendered(view.clone(), 1.0);
    // DIB pixels are BGR; check window background, border, and selected block fill.
    assert_close(pixel_at(&layout, &pixels, 2, 2), [0xf1, 0xf8, 0xfb], "窗口背景");
    assert_close(pixel_at(&layout, &pixels, layout.width / 2, 0), [0xc8, 0xd9, 0xe2], "窗口边框");
    let item = layout.items[0];
    assert_close(pixel_at(&layout, &pixels, item.left + 2, item.top + 2), [0x09, 0x53, 0xb4], "选中底色块");

    let no_fill = View {
        candidate_skin: Some(CandidateSkin { accent_fill: false, ..skin }),
        ..view
    };
    let (layout_no_fill, pixels_no_fill) = rendered(no_fill, 1.0);
    let item_no_fill = layout_no_fill.items[0];
    assert_close(
        pixel_at(&layout_no_fill, &pixels_no_fill, item_no_fill.left + 2, item_no_fill.top + 2),
        [0xf1, 0xf8, 0xfb],
        "关闭底色块后选中项周围保持窗口背景",
    );
}

#[test]
fn prediction_dismisses_on_clicks_outside_the_candidate_window() {
    use std::rc::Rc;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, PeekMessageW, SendMessageW, MSG, PM_REMOVE, WM_ACTIVATEAPP,
    };
    let _desktop = PrivateDesktop::new();
    let instance = HINSTANCE(unsafe { GetModuleHandleW(None).unwrap() }.0);
    let window = CandidateWindow::new(instance).unwrap();
    let other = popup(instance, CLASS_NAME, None).unwrap();
    let clicks = Rc::new(RefCell::new(Vec::new()));
    let captured = clicks.clone();
    set_click_handler(Some(Box::new(move |click| captured.borrow_mut().push(click))));
    // 私有桌面收不到真实鼠标输入：按系统调用钩子的方式把按下交给它，再处理投递的消息
    let press = |target: HWND, msg: u32| unsafe {
        let info = MOUSEHOOKSTRUCT { hwnd: target, ..Default::default() };
        outside_click_proc(HC_ACTION as i32, WPARAM(msg as usize), LPARAM(&info as *const _ as isize));
        let mut msg = MSG::default();
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() { DispatchMessageW(&msg); }
    };
    let watching = || OUTSIDE.with(Cell::get).is_some();
    let mut frame = Frame {
        candidates: vec!["好".into(), "一起".into()],
        cursor: Some(0), predict: true, page_revision: 1,
        font_size: 18, font_family: "Microsoft YaHei".into(),
        ..Frame::default()
    };
    window.update(&frame, Some(rect(100, 100, 2, 20)));
    assert!(watching(), "联想显示时装上点击监视");
    press(window.hwnd, WM_LBUTTONDOWN);
    press(other, WM_MOUSEMOVE);
    assert!(clicks.borrow().is_empty(), "点在联想窗里面或只移动鼠标都不收起");

    press(other, WM_LBUTTONDOWN);
    assert_eq!(clicks.borrow().as_slice(), [Click::Dismiss], "点在联想窗外面就收起");
    assert!(!watching());
    clicks.borrow_mut().clear();

    frame.page_revision += 1;
    frame.commit = Some("好".into());
    window.update(&frame, Some(rect(100, 100, 2, 20)));
    press(other, WM_NCRBUTTONDOWN);
    assert_eq!(clicks.borrow().as_slice(), [Click::Dismiss], "右键标题栏等非客户区也收起");
    clicks.borrow_mut().clear();

    window.update(&frame, Some(rect(100, 100, 2, 20)));
    unsafe { SendMessageW(window.hwnd, WM_ACTIVATEAPP, Some(WPARAM(0)), Some(LPARAM(0))); }
    assert_eq!(clicks.borrow().as_slice(), [Click::Dismiss], "切到别的程序也收起");
    clicks.borrow_mut().clear();

    window.update(&Frame { predict: false, ..frame.clone() }, Some(rect(100, 100, 2, 20)));
    assert!(!watching(), "普通候选不装点击监视");
    unsafe { SendMessageW(window.hwnd, WM_ACTIVATEAPP, Some(WPARAM(0)), Some(LPARAM(0))); }
    assert!(clicks.borrow().is_empty(), "普通候选不受影响");
    window.update(&frame, Some(rect(100, 100, 2, 20)));
    window.hide();
    assert!(!watching(), "收起窗口时卸下点击监视");
    set_click_handler(None);
    unsafe { let _ = DestroyWindow(other); }
}
