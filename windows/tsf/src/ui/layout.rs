//! 候选窗排版：拼音行、候选项、标签、箭头和提示的位置。
use super::*;

pub(super) fn rect(x: i32, y: i32, w: i32, h: i32) -> RECT {
    RECT {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
    }
}

pub(super) fn layout(view: &View, fonts: &Fonts, scale: f32) -> Result<Layout> {
    let s = |v: f32| (v * scale).round() as i32;
    let pad = s(8.0);
    let mut out = Layout::default();
    {
        if let Some(hint) = &view.hint {
            let text = fonts.hint.text(hint)?;
            let size = text.size;
            let side = size.cx.max(size.cy);
            out.width = side + 2 * pad;
            out.height = size.cy + 2 * pad;
            // 提示只有一两个字：墨迹放在窗口正中
            let (x, y) = text.centered_in(&rect(0, 0, out.width, out.height));
            out.parts.push(Piece::new(Part::Hint, text, x, y));
            return Ok(out);
        }
        let mut y = pad;
        let mut width = 0;
        if let Some(sentence) = &view.sentence {
            let limit = if view.available_width > 0 {
                view.available_width - 2 * pad
            } else {
                s(900.0)
            };
            let mut x = pad;
            let height = fonts.main.text("中")?.size.cy + s(8.0);
            for (i, b) in sentence.blocks.iter().enumerate() {
                let text = fonts.main.text(&b.text)?;
                let w = text.size.cx + s(14.0);
                if x > pad && x + w > limit {
                    x = pad;
                    y += height + s(3.0);
                }
                let area = rect(x, y, w, height);
                out.sentence_hits.push((area, b.to));
                out.parts.push(Piece::new(
                    Part::Sentence(i),
                    text.clone(),
                    (x + s(7.0)) as f32,
                    text.centered_y(&area),
                ));
                x += w + s(3.0);
                width = width.max(x);
            }
            y += height + s(4.0);
            // The raw-key row is the pointer equivalent of keyboard caret editing.
            let mut x = pad;
            for (at, ch) in sentence.raw.char_indices() {
                let text = fonts.small.text(&ch.to_string())?;
                let w = text.size.cx.max(s(7.0));
                if x > pad && x + w > limit {
                    x = pad;
                    y += text.size.cy + s(4.0);
                }
                let area = rect(x, y, w, text.size.cy + s(4.0));
                out.letter_hits.push((area, at + ch.len_utf8()));
                out.parts
                    .push(Piece::new(Part::Letter, text, x as f32, y as f32));
                x += w;
                width = width.max(x);
            }
            y += fonts.small.text("a")?.size.cy + s(7.0);
        }
        for (text, part) in [
            (view.title.clone().unwrap_or_default(), Part::Title),
            (view.preedit.clone(), Part::Preedit),
        ] {
            if text.is_empty() {
                continue;
            }
            let font = if part == Part::Preedit {
                &fonts.preedit
            } else if view.sentence.is_some() {
                &fonts.caption
            } else {
                &fonts.small
            };
            let text = font.text(&text)?;
            let size = text.size;
            out.parts
                .push(Piece::new(part, text, (pad + s(4.0)) as f32, y as f32));
            width = width.max(size.cx + s(8.0));
            y += size.cy + s(4.0);
        }
        let line = fonts.main.text("中")?.size.cy;
        let item_pad = s(6.0);
        // 排版结果保留到绘制时使用；emoji 回退字体可能比中文行高，不能按“中”的高度裁切。
        let texts: Vec<(Option<Text>, Text)> = view
            .candidates
            .iter()
            .enumerate()
            .map(|(i, text)| -> Result<_> {
                let label = view.label(i);
                let label = if label.is_empty() {
                    None
                } else {
                    Some(fonts.small.text(&label)?)
                };
                let mut display = match view.badges.get(i).filter(|badge| !badge.is_empty()) {
                    Some(badge) => format!("{text} {badge}"),
                    None => text.clone(),
                };
                if let Some(annotation) = view.annotations.get(i).filter(|s| !s.is_empty()) {
                    display.push_str("  ·  ");
                    display.push_str(annotation);
                }
                // 长释义不能把窗口撑出屏幕；仅截断显示，不改变服务端的提交文字。
                if view.available_width > 0 {
                    let budget = (view.available_width - 4 * pad - s(32.0)).max(s(80.0));
                    if fonts.main.text(&display)?.size.cx > budget {
                        let chars: Vec<_> = display.chars().collect();
                        let (mut low, mut high) = (0, chars.len());
                        while low < high {
                            let mid = (low + high + 1) / 2;
                            let shortened = chars[..mid].iter().collect::<String>() + "…";
                            if fonts.main.text(&shortened)?.size.cx <= budget {
                                low = mid;
                            } else {
                                high = mid - 1;
                            }
                        }
                        display = chars[..low].iter().collect::<String>() + "…";
                    }
                }
                Ok((label, fonts.main.text(&display)?))
            })
            .collect::<Result<_>>()?;
        // 序号和正文分别按实际字形居中，只需为最高的墨迹留出空间。
        let ink_height = |text: &Text| text.ink.bottom - text.ink.top;
        let tallest = texts.iter().fold(0, |height, (label, body)| {
            height
                .max(ink_height(body))
                .max(label.as_ref().map_or(0, ink_height))
        });
        // 普通候选保持统一的行高；更高的字形（例如回退的表情）再撑开。
        let h = (line + s(6.0)).max(tallest + s(2.0));
        // 一个候选的内容从标号的墨迹左边到正文的墨迹右边，两头留一样的空；
        // 标号和正文之间照字体自己的字距再加一点
        let content = |(label, body): &(Option<Text>, Text)| match label {
            Some(label) => label.size.cx - label.ink.left + s(4.0) + body.ink.right,
            None => body.ink.right - body.ink.left,
        };
        let item_width = |item: &(Option<Text>, Text)| item_pad + content(item) + item_pad;
        // 格子里（表情面板）色块比内容宽，内容放中间；横排时色块正好包住内容；竖排靠左
        let centered = view.panel == Panel::Grid;
        let place = |out: &mut Layout, i: usize, x: i32, y: i32, w: i32| {
            let (label, body) = &texts[i];
            let item = rect(x, y, w, h);
            out.items.push(item);
            let spare = if centered {
                (w - item_width(&texts[i])).max(0) / 2
            } else {
                0
            };
            let mut left = (x + item_pad + spare) as f32;
            if let Some(label) = label {
                let at = left - label.ink.left as f32;
                out.parts.push(Piece::new(
                    Part::Label(i),
                    label.clone(),
                    at,
                    label.centered_y(&item),
                ));
                left = at + (label.size.cx + s(4.0)) as f32;
            } else {
                left -= body.ink.left as f32;
            }
            out.parts.push(Piece::new(
                Part::Candidate(i),
                body.clone(),
                left,
                body.centered_y(&item),
            ));
        };
        let prev = fonts.main.text("‹")?;
        let next = fonts.main.text("›")?;
        let arrow_w = prev.size.cx.max(next.size.cx) + s(12.0);
        let paging = view.has_prev || view.has_next;
        let horizontal = view.panel == Panel::Normal && !view.vertical;
        if horizontal {
            let mut x = pad;
            for i in 0..texts.len() {
                let w = item_width(&texts[i]);
                if view.wrap_width > 0 && x > pad && x + w + pad > view.wrap_width {
                    width = width.max(x - pad);
                    x = pad;
                    y += h;
                }
                place(&mut out, i, x, y, w);
                x += w + s(2.0);
            }
            width = width.max(x - pad);
            if paging {
                let mut ax = x + s(4.0);
                if view.wrap_width > 0 && ax + 2 * arrow_w + pad > view.wrap_width {
                    ax = pad;
                    y += h;
                }
                out.arrows
                    .push((rect(ax, y, arrow_w, h), false, prev.clone()));
                out.arrows
                    .push((rect(ax + arrow_w, y, arrow_w, h), true, next.clone()));
                width = width.max(ax + 2 * arrow_w - pad);
            }
            if !texts.is_empty() {
                y += h;
            }
        } else {
            let columns = if view.panel == Panel::Grid {
                view.columns.max(1)
            } else {
                1
            };
            let cell = texts.iter().map(item_width).max().unwrap_or(0);
            let cell = if view.panel == Panel::Grid {
                cell.max(h)
            } else {
                cell.max(s(120.0))
            };
            for i in 0..texts.len() {
                let (row, col) = (i / columns, i % columns);
                place(
                    &mut out,
                    i,
                    pad + col as i32 * (cell + s(2.0)),
                    y + row as i32 * h,
                    cell,
                );
            }
            let rows = texts.len().div_ceil(columns) as i32;
            let cols = columns.min(texts.len().max(1)) as i32;
            width = width.max(cols * (cell + s(2.0)) - s(2.0));
            y += rows * h;
            if paging {
                let ax = pad + width - 2 * arrow_w;
                out.arrows.push((rect(ax, y, arrow_w, h), false, prev));
                out.arrows
                    .push((rect(ax + arrow_w, y, arrow_w, h), true, next));
                width = width.max(2 * arrow_w);
                y += h;
            }
        }
        out.width = width.max(s(40.0)) + 2 * pad;
        out.height = y + pad;
    }
    Ok(out)
}
