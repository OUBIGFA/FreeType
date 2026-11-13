//! 工具栏（全拼下只打 v 再按空格）和表情面板（快捷键、状态栏打开），照 WowType 的 Linux 版：
//!
//! | 页 | 内容 | 样子 |
//! |---|---|---|
//! | 工具栏 | 剪贴板、常用语、符号、表情四项 | 一行 |
//! | 剪贴板 | 最近复制的文字，最新的在前（设置里打开才记） | 一行一条 |
//! | 常用语 | 快捷短语，按文件里的顺序 | 一行一条 |
//! | 符号、表情 | 全部内置符号 / emoji；打拼音、英文搜 | 网格 |
//!
//! 数字选（网格里选光标所在行的第几个），方向键移光标，空格、回车选光标处的，
//! 退格先删搜索词、再回上一层，Esc 关掉。带 Ctrl、Alt、Win 的键关掉面板、交给应用。

use freetype_protocol::{keysym, modifier, Key, Panel};
use wowtype_core::Engine;

/// 列表一页几条。
const LIST_PAGE: usize = 9;
/// 网格一行几个、一屏几行。
pub const GRID_COLS: usize = 8;
const GRID_ROWS: usize = 5;
const GRID_SCREEN: usize = GRID_COLS * GRID_ROWS;
/// 表情搜索最多给多少个。
const MAX_EMOJI: usize = 400;
/// 列表里一行最多显示多少个字。
const LINE_CHARS: usize = 40;

const MENU: [&str; 4] = ["剪贴板", "常用语", "符号", "表情"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Menu,
    Clipboard,
    Phrases,
    Symbols,
    Emoji,
}

/// 面板要用的数据（每次重算时由宿主给）。
pub struct Sources<'a> {
    pub engine: Option<&'a Engine>,
    /// 常用语：(编码, 上屏的文字)。
    pub phrases: &'a [(String, String)],
    /// 最近用过的 emoji，最近的在前。
    pub recent: &'a [String],
    /// 剪贴板历史打开了没有（没打开时列表为空，标题说明怎么打开）。
    pub clipboard_on: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Item {
    shown: String,
    commit: String,
}

#[derive(Clone, Debug)]
pub struct Tool {
    pub page: Page,
    /// 从工具栏点进来的（退格回工具栏）；快捷键直接打开的表情面板退格关掉。
    from_menu: bool,
    pub query: String,
    items: Vec<Item>,
    /// 光标（全部条目里的序号）。
    cursor: usize,
    /// 列表在第几页（网格按光标算屏）。
    list_page: usize,
    clipboard_on: bool,
}

/// 按了一个键以后。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// 面板还开着（数据可能要重算：搜索词变了、换了页）。
    Stay { refresh: bool },
    Close,
    /// 上屏这段文字并关掉面板；`emoji`：记进最近用过的 emoji。
    Commit { text: String, emoji: bool },
    /// 关掉面板，这个键交给应用。
    Pass,
}

/// 一屏的内容（宿主翻成 Frame）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct View {
    pub title: String,
    pub items: Vec<String>,
    pub labels: Vec<String>,
    pub cursor: Option<usize>,
    pub has_prev: bool,
    pub has_next: bool,
    pub panel: Panel,
    pub columns: usize,
    /// 编辑区：搜索词（没有时显示页名，让应用知道在组字）。
    pub preedit: String,
}

fn line(text: &str) -> String {
    let mut out = String::new();
    for (i, c) in text.chars().enumerate() {
        if i == LINE_CHARS {
            out.push('…');
            break;
        }
        out.push(match c {
            '\n' => '⏎',
            '\t' | '\r' => ' ',
            c => c,
        });
    }
    out
}

impl Tool {
    pub fn open(page: Page, sources: &Sources) -> Self {
        let mut tool = Self {
            page,
            from_menu: false,
            query: String::new(),
            items: Vec::new(),
            cursor: 0,
            list_page: 0,
            clipboard_on: sources.clipboard_on,
        };
        tool.refresh(sources);
        tool
    }

    fn grid(&self) -> bool {
        matches!(self.page, Page::Symbols | Page::Emoji)
    }

    fn searchable(&self) -> bool {
        self.grid()
    }

    /// 重新取这一页的条目，光标回到开头。
    pub fn refresh(&mut self, sources: &Sources) {
        self.clipboard_on = sources.clipboard_on;
        let item = |text: &str| Item {
            shown: line(text),
            commit: text.to_string(),
        };
        self.items = match self.page {
            Page::Menu => MENU.iter().map(|m| item(m)).collect(),
            Page::Clipboard => wowtype_core::clipboard::history().iter().map(|t| item(t)).collect(),
            Page::Phrases => sources
                .phrases
                .iter()
                .map(|(code, text)| Item {
                    shown: if code.is_empty() { line(text) } else { format!("{}  {code}", line(text)) },
                    commit: text.clone(),
                })
                .collect(),
            Page::Symbols if self.query.is_empty() => {
                wowtype_core::symbols::SYMBOLS.iter().map(|s| item(s.text)).collect()
            }
            Page::Symbols | Page::Emoji => {
                let mut out: Vec<Item> = Vec::new();
                if self.page == Page::Emoji && self.query.is_empty() {
                    out.extend(sources.recent.iter().map(|e| item(e)));
                }
                for text in wowtype_core::emoji::panel_search(sources.engine, &self.query, MAX_EMOJI) {
                    if !out.iter().any(|i| i.commit == text) {
                        out.push(item(&text));
                    }
                }
                out
            }
        };
        self.cursor = 0;
        self.list_page = 0;
    }

    fn per_page(&self) -> usize {
        match self.page {
            Page::Menu => MENU.len(),
            _ if self.grid() => GRID_SCREEN,
            _ => LIST_PAGE,
        }
    }

    /// 当前屏在全部条目里的起点。
    fn start(&self) -> usize {
        if self.grid() {
            self.cursor / GRID_SCREEN * GRID_SCREEN
        } else {
            self.list_page * self.per_page()
        }
    }

    fn select(&mut self, index: usize, sources: &Sources) -> Outcome {
        let Some(item) = self.items.get(index).cloned() else {
            return Outcome::Stay { refresh: false };
        };
        if self.page == Page::Menu {
            self.page = match index {
                0 => Page::Clipboard,
                1 => Page::Phrases,
                2 => Page::Symbols,
                _ => Page::Emoji,
            };
            self.from_menu = true;
            self.query.clear();
            self.refresh(sources);
            return Outcome::Stay { refresh: true };
        }
        Outcome::Commit {
            text: item.commit,
            emoji: self.page == Page::Emoji,
        }
    }

    /// 鼠标点了这一屏的第 index 个。
    pub fn click(&mut self, index: usize, sources: &Sources) -> Outcome {
        let start = self.start();
        if start + index >= self.items.len() {
            return Outcome::Stay { refresh: false };
        }
        self.select(start + index, sources)
    }

    /// 翻页按钮。
    pub fn page(&mut self, next: bool) {
        self.turn(next);
    }

    fn turn(&mut self, next: bool) {
        let n = self.items.len();
        if n == 0 {
            return;
        }
        if self.grid() {
            self.cursor = if next {
                (self.cursor + GRID_SCREEN).min(n - 1)
            } else {
                self.cursor.saturating_sub(GRID_SCREEN)
            };
            return;
        }
        let pages = n.div_ceil(self.per_page());
        if next && self.list_page + 1 < pages {
            self.list_page += 1;
        } else if !next && self.list_page > 0 {
            self.list_page -= 1;
        } else {
            return;
        }
        self.cursor = self.list_page * self.per_page();
    }

    fn step(&mut self, delta: isize) {
        let n = self.items.len() as isize;
        if n == 0 {
            return;
        }
        let next = (self.cursor as isize + delta).clamp(0, n - 1) as usize;
        self.cursor = next;
        if !self.grid() {
            self.list_page = self.cursor / self.per_page();
        }
    }

    pub fn key(&mut self, key: &Key, sources: &Sources) -> Outcome {
        if key.modifiers & (modifier::CTRL | modifier::ALT | modifier::SUPER) != 0 {
            return Outcome::Pass;
        }
        let sym = key.keysym;
        let digit = match sym {
            0x31..=0x39 => Some((sym - 0x30) as usize),
            s if (keysym::KP_0 + 1..=keysym::KP_9).contains(&s) => Some((s - keysym::KP_0) as usize),
            _ => None,
        };
        let columns = if self.grid() { GRID_COLS } else { 1 };
        match sym {
            keysym::ESCAPE => return Outcome::Close,
            keysym::BACKSPACE => {
                if self.query.pop().is_some() {
                    self.refresh(sources);
                    return Outcome::Stay { refresh: true };
                }
                if self.from_menu && self.page != Page::Menu {
                    self.page = Page::Menu;
                    self.from_menu = false;
                    self.refresh(sources);
                    return Outcome::Stay { refresh: true };
                }
                return Outcome::Close;
            }
            0x20 | keysym::RETURN | keysym::KP_ENTER => {
                if self.items.is_empty() {
                    return Outcome::Close;
                }
                return self.select(self.cursor, sources);
            }
            keysym::LEFT => self.step(-1),
            keysym::RIGHT => self.step(1),
            keysym::UP => self.step(-(columns as isize)),
            keysym::DOWN => self.step(columns as isize),
            keysym::PAGE_UP => self.turn(false),
            keysym::PAGE_DOWN => self.turn(true),
            0x2d if !self.searchable() || self.query.is_empty() => self.turn(false),
            0x3d if !self.searchable() || self.query.is_empty() => self.turn(true),
            _ => {
                if let Some(d) = digit {
                    let index = if self.grid() {
                        self.cursor / GRID_COLS * GRID_COLS + d - 1
                    } else {
                        self.start() + d - 1
                    };
                    let row_end = if self.grid() {
                        self.cursor / GRID_COLS * GRID_COLS + GRID_COLS
                    } else {
                        self.start() + self.per_page()
                    };
                    if index < row_end && index < self.items.len() {
                        return self.select(index, sources);
                    }
                    return Outcome::Stay { refresh: false };
                }
                if self.searchable() {
                    if let Some(c) = char::from_u32(sym).filter(|c| c.is_ascii_alphabetic() || *c == '\'') {
                        self.query.push(c.to_ascii_lowercase());
                        self.refresh(sources);
                        return Outcome::Stay { refresh: true };
                    }
                }
            }
        }
        Outcome::Stay { refresh: false }
    }

    fn title(&self) -> String {
        let base = match self.page {
            Page::Menu => "工具栏".to_string(),
            Page::Clipboard if !self.clipboard_on => "剪贴板 · 在设置「功能」里打开「记录剪贴板历史」".to_string(),
            Page::Clipboard if self.items.is_empty() => "剪贴板 · 还没有记录".to_string(),
            Page::Clipboard => "剪贴板".to_string(),
            Page::Phrases if self.items.is_empty() => "常用语 · 在设置「常用语」里添加".to_string(),
            Page::Phrases => "常用语".to_string(),
            Page::Symbols => "符号 · 打拼音或英文搜".to_string(),
            Page::Emoji => "表情 · 打拼音或英文搜".to_string(),
        };
        if self.query.is_empty() {
            base
        } else {
            let found = if self.items.is_empty() { " · 没有找到" } else { "" };
            format!("{} · {}{found}", base.split(" · ").next().unwrap_or(""), self.query)
        }
    }

    pub fn view(&self) -> View {
        let n = self.items.len();
        let start = self.start().min(n);
        let end = (start + self.per_page()).min(n);
        let items: Vec<String> = self.items[start..end].iter().map(|i| i.shown.clone()).collect();
        let cursor = (n > 0 && (start..end).contains(&self.cursor)).then(|| self.cursor - start);
        let (panel, columns, labels) = if self.grid() {
            // 光标所在行标 1–8
            let row = cursor.map(|c| c / GRID_COLS);
            let labels = (0..items.len())
                .map(|i| {
                    if Some(i / GRID_COLS) == row {
                        (i % GRID_COLS + 1).to_string()
                    } else {
                        String::new()
                    }
                })
                .collect();
            (Panel::Grid, GRID_COLS, labels)
        } else if self.page == Page::Menu {
            (Panel::Normal, 0, Vec::new())
        } else {
            (Panel::List, 1, Vec::new())
        };
        View {
            title: self.title(),
            items,
            labels,
            cursor,
            has_prev: start > 0,
            has_next: end < n,
            panel,
            columns,
            preedit: if self.query.is_empty() {
                String::new()
            } else {
                self.query.clone()
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(sym: u32) -> Key {
        Key {
            keysym: sym,
            modifiers: 0,
            test: false,
        }
    }

    fn sources<'a>(phrases: &'a [(String, String)], recent: &'a [String]) -> Sources<'a> {
        Sources {
            engine: None,
            phrases,
            recent,
            clipboard_on: true,
        }
    }

    #[test]
    fn menu_opens_pages_and_backspace_returns() {
        let phrases = vec![("dz".to_string(), "北京市\n海淀区".to_string())];
        let s = sources(&phrases, &[]);
        let mut tool = Tool::open(Page::Menu, &s);
        let view = tool.view();
        assert_eq!(view.items, vec!["剪贴板", "常用语", "符号", "表情"]);
        assert_eq!(view.title, "工具栏");
        assert_eq!(tool.key(&press('2' as u32), &s), Outcome::Stay { refresh: true });
        assert_eq!(tool.page, Page::Phrases);
        let view = tool.view();
        assert_eq!(view.items, vec!["北京市⏎海淀区  dz"]);
        assert_eq!(view.panel, Panel::List);
        assert_eq!(
            tool.key(&press(0x20), &s),
            Outcome::Commit {
                text: "北京市\n海淀区".into(),
                emoji: false
            }
        );
        tool.key(&press(keysym::BACKSPACE), &s);
        assert_eq!(tool.page, Page::Menu);
        assert_eq!(tool.key(&press(keysym::BACKSPACE), &s), Outcome::Close);
    }

    #[test]
    fn emoji_grid_search_and_digit_select() {
        let recent = vec!["🎉".to_string()];
        let s = sources(&[], &recent);
        let mut tool = Tool::open(Page::Emoji, &s);
        let view = tool.view();
        assert_eq!(view.items[0], "🎉", "最近用过的在前");
        assert_eq!(view.panel, Panel::Grid);
        assert_eq!(view.columns, GRID_COLS);
        assert_eq!(view.labels[0], "1");
        assert_eq!(view.labels[GRID_COLS], "", "只标光标所在行");
        assert!(view.has_next);
        for c in "smile".chars() {
            tool.key(&press(c as u32), &s);
        }
        let view = tool.view();
        assert!(view.title.contains("smile"), "{}", view.title);
        assert!(!view.items.is_empty());
        let first = view.items[0].clone();
        assert_eq!(
            tool.key(&press('1' as u32), &s),
            Outcome::Commit {
                text: first,
                emoji: true
            }
        );
        // 退格先删搜索词；直接打开的表情面板删完再退格关掉
        let mut tool = Tool::open(Page::Emoji, &s);
        tool.key(&press('a' as u32), &s);
        assert!(matches!(tool.key(&press(keysym::BACKSPACE), &s), Outcome::Stay { .. }));
        assert_eq!(tool.key(&press(keysym::BACKSPACE), &s), Outcome::Close);
    }

    #[test]
    fn grid_moves_by_rows_and_screens() {
        let s = sources(&[], &[]);
        let mut tool = Tool::open(Page::Symbols, &s);
        tool.key(&press(keysym::DOWN), &s);
        assert_eq!(tool.view().cursor, Some(GRID_COLS));
        assert_eq!(tool.view().labels[GRID_COLS], "1");
        tool.key(&press(keysym::PAGE_DOWN), &s);
        assert_eq!(tool.view().cursor, Some(GRID_COLS), "下一屏的同一位置");
        assert!(tool.view().has_prev);
    }

    #[test]
    fn ctrl_keys_pass_and_clipboard_title_explains() {
        let s = Sources {
            clipboard_on: false,
            ..sources(&[], &[])
        };
        let mut tool = Tool::open(Page::Clipboard, &s);
        assert!(tool.view().title.contains("记录剪贴板历史"));
        let ctrl_c = Key {
            keysym: 'c' as u32,
            modifiers: modifier::CTRL,
            test: false,
        };
        assert_eq!(tool.key(&ctrl_c, &s), Outcome::Pass);
    }
}
