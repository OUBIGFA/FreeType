//! Windows 虚拟键 → 服务进程的平台无关键码。

use freetype_protocol::{keysym, modifier};

/// 不靠键盘布局就能翻的键：功能键、方向键、小键盘。可见字符返回 None（交给 ToUnicode）。
pub fn named_keysym(vk: u16) -> Option<u32> {
    Some(match vk {
        0x08 => keysym::BACKSPACE,
        0x09 => keysym::TAB,
        0x0d => keysym::RETURN,
        0x1b => keysym::ESCAPE,
        0x20 => 0x20,
        0x21 => keysym::PAGE_UP,
        0x22 => keysym::PAGE_DOWN,
        0x23 => keysym::END,
        0x24 => keysym::HOME,
        0x25 => keysym::LEFT,
        0x26 => keysym::UP,
        0x27 => keysym::RIGHT,
        0x28 => keysym::DOWN,
        0x2e => keysym::DELETE,
        0x60..=0x69 => keysym::KP_0 + u32::from(vk - 0x60),
        0x6a => keysym::KP_MULTIPLY,
        0x6b => keysym::KP_ADD,
        0x6d => keysym::KP_SUBTRACT,
        0x6e => keysym::KP_DECIMAL,
        0x6f => keysym::KP_DIVIDE,
        _ => return None,
    })
}

/// 修饰键本身（只记状态，不当按键发给服务进程）。
pub fn is_modifier(vk: u16) -> bool {
    matches!(
        vk,
        0x10 | 0x11 | 0x12 | 0x14 | 0x5b | 0x5c | 0x90 | 0xa0..=0xa5
    )
}

pub const VK_SHIFT: u16 = 0x10;

/// GetKeyState 的结果翻成修饰键位掩码。`state(vk)` 返回 GetKeyState(vk)。
pub fn modifiers(state: impl Fn(u16) -> i16) -> u32 {
    let down = |vk| state(vk) < 0;
    let toggled = |vk| state(vk) & 1 != 0;
    let mut m = 0;
    if down(0x10) {
        m |= modifier::SHIFT;
    }
    if down(0x11) {
        m |= modifier::CTRL;
    }
    if down(0x12) {
        m |= modifier::ALT;
    }
    if down(0x5b) || down(0x5c) {
        m |= modifier::SUPER;
    }
    if toggled(0x14) {
        m |= modifier::CAPS;
    }
    if toggled(0x90) {
        m |= modifier::NUMLOCK;
    }
    m
}

/// ToUnicode 给出的字符：只收可见 ASCII（别的字符原样交给应用）。
pub fn char_keysym(c: char) -> Option<u32> {
    c.is_ascii_graphic().then_some(c as u32)
}

/// 按下、松开 Shift 那一刻的情况。
///
/// 和常见输入法一样：Shift 按下到松开之间没有按别的键就切换，按多久都算。
/// 只有和 Ctrl、Alt、Win 一起按（Ctrl+Shift 是切换键盘布局）不算。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Moment {
    /// Ctrl、Alt、Win 有没有按着。
    pub other_modifiers: bool,
}

/// 左边还是右边的 Shift。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    pub fn name(self) -> &'static str {
        match self {
            Side::Left => "left",
            Side::Right => "right",
        }
    }
}

/// 认单按 Shift。
///
/// 系统对同一次按下、松开可能问几遍（先 OnTestKey*、再 OnKey*；浏览器内核的程序空闲时
/// 还会先看一眼消息再取走，多问一遍）。所以按下可以重复报，松开只有第一次报到时算数。
#[derive(Default)]
pub struct ShiftTap {
    /// Shift 按下了，之后还没按过别的键。
    down: Option<Moment>,
}

impl ShiftTap {
    /// 有键按下（`repeat`：按住不放的自动重复）。
    pub fn key_down(&mut self, vk: u16, at: Moment, repeat: bool) {
        if is_shift(vk) {
            if repeat || self.down.is_some() {
                return;
            }
            self.down = (!at.other_modifiers).then_some(at);
        } else {
            self.down = None;
        }
    }

    /// 有键松开：是一次干净的单按 Shift 返回 true。同一次松开再报，返回 false。
    pub fn key_up(&mut self, vk: u16, at: Moment) -> bool {
        if !is_shift(vk) {
            return false;
        }
        match self.down.take() {
            Some(_) => !at.other_modifiers,
            None => false,
        }
    }

    pub fn cancel(&mut self) {
        self.down = None;
    }
}

fn is_shift(vk: u16) -> bool {
    matches!(vk, 0x10 | 0xa0 | 0xa1)
}

/// 哪边的 Shift：虚拟键分得出就用它，分不出（0x10）看扫描码（右 Shift 是 0x36）。
pub fn shift_side(vk: u16, scan: u32) -> Side {
    match vk {
        0xa1 => Side::Right,
        0xa0 => Side::Left,
        _ if scan == 0x36 => Side::Right,
        _ => Side::Left,
    }
}

/// FreeType 自己发给应用的键（成对标点后的 ←）带这个记号（`dwExtraInfo`）。按键回调认出来
/// 就直接放行：不当成用户按的键发给服务进程，也不影响单按 Shift 的判定。
pub const OWN_INPUT_TAG: usize = 0x4654_5950;

/// 成对标点上屏以后，等着把光标挪到两半中间的这一次。
///
/// 不在上屏的当时发 ←，而是回到应用的消息循环以后再发：
/// - 应用这时已经把字落好了，← 不会抢在上屏前面；
/// - （、《、“ 是按着 Shift 打的，Shift 按着时应用把 ← 当成 Shift+←（选中右半边），
///   Ctrl+← 是跳一个词、Win+← 是把窗口贴到左边。所以等用户松开这些键再发。
///
/// 等的时候焦点换了、用户又按了别的键、或者等得太久，这一次就作废，光标留在原处。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaretMove {
    steps: usize,
    /// 过了这个时刻（毫秒）就作废。
    deadline: u64,
}

/// 现在该拿 [`CaretMove`] 怎么办。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaretStep {
    /// 修饰键还按着：过一会儿再看。
    Wait,
    /// 发这么多下 ←。
    Send(usize),
    /// 作废。
    Drop,
}

impl CaretMove {
    /// 最多等多久（按住 Shift 不放的人，过后光标已经不在原处了）。
    pub const TIMEOUT_MS: u64 = 2000;
    /// 一次最多挪几下。
    const MAX_STEPS: usize = 4;

    pub fn new(steps: usize, now: u64) -> Self {
        Self {
            steps: steps.min(Self::MAX_STEPS),
            deadline: now + Self::TIMEOUT_MS,
        }
    }

    /// `focused`：这个线程还有输入焦点；`modifiers_held`：Shift、Ctrl、Alt、Win 有按着的。
    pub fn step(&self, now: u64, focused: bool, modifiers_held: bool) -> CaretStep {
        if !focused || now > self.deadline {
            CaretStep::Drop
        } else if modifiers_held {
            CaretStep::Wait
        } else {
            CaretStep::Send(self.steps)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shift_tap_survives_repeated_queries() {
        // 浏览器内核的程序对同一次按键会多问几遍（OnTestKeyDown / OnTestKeyUp 重复调用）
        let mut tap = ShiftTap::default();
        tap.key_down(0x10, PLAIN, false);
        tap.key_down(0x10, PLAIN, false);
        assert!(tap.key_up(0x10, PLAIN), "第一次问到松开就切换");
        assert!(!tap.key_up(0x10, PLAIN), "同一次松开再问：不再切换");
        assert!(!tap.key_up(0x10, PLAIN));
        tap.key_down(0x10, PLAIN, false);
        assert!(tap.key_up(0x10, PLAIN), "下一次单按照常切换");
    }

    #[test]
    fn caret_move_waits_for_modifiers() {
        let pending = CaretMove::new(1, 1000);
        assert_eq!(pending.step(1000, true, true), CaretStep::Wait, "Shift 还按着：等");
        assert_eq!(pending.step(1100, true, false), CaretStep::Send(1), "松手了：发 ←");
        assert_eq!(pending.step(1100, false, false), CaretStep::Drop, "焦点换走了：不发");
        assert_eq!(pending.step(1000 + CaretMove::TIMEOUT_MS, true, true), CaretStep::Wait);
        assert_eq!(pending.step(1001 + CaretMove::TIMEOUT_MS, true, false), CaretStep::Drop, "等太久：作废");
        assert_eq!(CaretMove::new(99, 0).step(0, true, false), CaretStep::Send(4), "一次最多挪 4 下");
    }

    #[test]
    fn named_keys() {
        assert_eq!(named_keysym(0x08), Some(keysym::BACKSPACE));
        assert_eq!(named_keysym(0x63), Some(keysym::KP_0 + 3));
        assert_eq!(named_keysym(0x41), None, "字母交给 ToUnicode");
    }

    #[test]
    fn modifier_mask() {
        let m = modifiers(|vk| match vk {
            0x10 => -128,
            0x14 => 1,
            _ => 0,
        });
        assert_eq!(m, modifier::SHIFT | modifier::CAPS);
    }

    const PLAIN: Moment = Moment { other_modifiers: false };
    const CTRL: Moment = Moment { other_modifiers: true };

    #[test]
    fn shift_tap_rules() {
        let mut tap = ShiftTap::default();
        tap.key_down(0x10, PLAIN, false);
        tap.key_down(0x10, PLAIN, true);
        assert!(tap.key_up(0x10, PLAIN), "干净的单按，按住久了自动重复也算");
        tap.key_down(0x10, PLAIN, false);
        tap.key_down(0x41, PLAIN, false);
        assert!(!tap.key_up(0x10, PLAIN), "中间按了 A：是 Shift+A");
        assert!(!tap.key_up(0x10, PLAIN), "没按下过");
    }

    #[test]
    fn shift_with_ctrl_alt_win_is_not_a_tap() {
        let mut tap = ShiftTap::default();
        tap.key_down(0x10, CTRL, false);
        assert!(!tap.key_up(0x10, PLAIN), "Ctrl 先按着：Ctrl+Shift");
        tap.key_down(0x10, PLAIN, false);
        assert!(!tap.key_up(0x10, CTRL), "松开前按了 Ctrl");
    }

    #[test]
    fn shift_sides() {
        assert_eq!(shift_side(0x10, 0x36), Side::Right);
        assert_eq!(shift_side(0x10, 0x2a), Side::Left);
        assert_eq!(shift_side(0xa1, 0), Side::Right);
        assert_eq!(Side::Left.name(), "left");
    }

    #[test]
    fn visible_ascii_only() {
        assert_eq!(char_keysym('a'), Some('a' as u32));
        assert_eq!(char_keysym(' '), None);
        assert_eq!(char_keysym('é'), None);
    }
}
