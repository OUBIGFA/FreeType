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

/// 单按 Shift 切换中英文：Shift 按下到松开之间没有别的键，且不超过这么久。
pub const SHIFT_TAP_MS: u64 = 600;

#[derive(Default)]
pub struct ShiftTap {
    down_at: Option<u64>,
}

impl ShiftTap {
    /// 有键按下（`repeat`：按住不放的自动重复）。
    pub fn key_down(&mut self, vk: u16, now_ms: u64, repeat: bool) {
        if is_shift(vk) {
            if !repeat && self.down_at.is_none() {
                self.down_at = Some(now_ms);
            }
        } else {
            self.down_at = None;
        }
    }

    /// 有键松开：是一次干净的单按 Shift 返回 true。
    pub fn key_up(&mut self, vk: u16, now_ms: u64) -> bool {
        if !is_shift(vk) {
            return false;
        }
        match self.down_at.take() {
            Some(at) => now_ms.saturating_sub(at) <= SHIFT_TAP_MS,
            None => false,
        }
    }

    pub fn cancel(&mut self) {
        self.down_at = None;
    }
}

fn is_shift(vk: u16) -> bool {
    matches!(vk, 0x10 | 0xa0 | 0xa1)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn shift_tap_rules() {
        let mut tap = ShiftTap::default();
        tap.key_down(0x10, 0, false);
        tap.key_down(0x10, 30, true);
        assert!(tap.key_up(0x10, 100), "干净的单按");
        tap.key_down(0x10, 0, false);
        tap.key_down(0x41, 10, false);
        assert!(!tap.key_up(0x10, 50), "中间按了 A：是 Shift+A");
        tap.key_down(0x10, 0, false);
        assert!(!tap.key_up(0x10, 2000), "按太久不算");
        assert!(!tap.key_up(0x10, 10), "没按下过");
    }

    #[test]
    fn visible_ascii_only() {
        assert_eq!(char_keysym('a'), Some('a' as u32));
        assert_eq!(char_keysym(' '), None);
        assert_eq!(char_keysym('é'), None);
    }
}
