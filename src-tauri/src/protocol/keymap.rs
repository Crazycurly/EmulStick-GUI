//! `rdev::Key` → HID Usage ID / modifier-bit mapping.
//!
//! The usage IDs follow the Appendix table in `docs/protocol.md`. Modifier
//! keys are routed to byte-0 bits instead of the key slots so they combine
//! correctly (see [`super::keyboard::KeyboardState`]).

use super::keyboard::modifier;
use rdev::Key;

/// How a physical key contributes to the F801 report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mapping {
    /// A modifier key contributing one bit to byte 0.
    Modifier(u8),
    /// A regular key contributing a HID usage id to bytes 2..7.
    Usage(u8),
}

/// Map an `rdev::Key` to its F801 contribution, or `None` if the key has no
/// standard HID usage (e.g. macOS `fn`, or platform `Unknown` scancodes).
pub fn map_key(key: &Key) -> Option<Mapping> {
    use Mapping::{Modifier, Usage};
    let mapping = match key {
        // ── Modifiers (byte 0) ───────────────────────────────────────────
        Key::ControlLeft => Modifier(modifier::LEFT_CTRL),
        Key::ShiftLeft => Modifier(modifier::LEFT_SHIFT),
        Key::Alt => Modifier(modifier::LEFT_ALT),
        Key::MetaLeft => Modifier(modifier::LEFT_GUI),
        Key::ControlRight => Modifier(modifier::RIGHT_CTRL),
        Key::ShiftRight => Modifier(modifier::RIGHT_SHIFT),
        Key::AltGr => Modifier(modifier::RIGHT_ALT),
        Key::MetaRight => Modifier(modifier::RIGHT_GUI),

        // ── Letters ──────────────────────────────────────────────────────
        Key::KeyA => Usage(4),
        Key::KeyB => Usage(5),
        Key::KeyC => Usage(6),
        Key::KeyD => Usage(7),
        Key::KeyE => Usage(8),
        Key::KeyF => Usage(9),
        Key::KeyG => Usage(10),
        Key::KeyH => Usage(11),
        Key::KeyI => Usage(12),
        Key::KeyJ => Usage(13),
        Key::KeyK => Usage(14),
        Key::KeyL => Usage(15),
        Key::KeyM => Usage(16),
        Key::KeyN => Usage(17),
        Key::KeyO => Usage(18),
        Key::KeyP => Usage(19),
        Key::KeyQ => Usage(20),
        Key::KeyR => Usage(21),
        Key::KeyS => Usage(22),
        Key::KeyT => Usage(23),
        Key::KeyU => Usage(24),
        Key::KeyV => Usage(25),
        Key::KeyW => Usage(26),
        Key::KeyX => Usage(27),
        Key::KeyY => Usage(28),
        Key::KeyZ => Usage(29),

        // ── Number row ───────────────────────────────────────────────────
        Key::Num1 => Usage(30),
        Key::Num2 => Usage(31),
        Key::Num3 => Usage(32),
        Key::Num4 => Usage(33),
        Key::Num5 => Usage(34),
        Key::Num6 => Usage(35),
        Key::Num7 => Usage(36),
        Key::Num8 => Usage(37),
        Key::Num9 => Usage(38),
        Key::Num0 => Usage(39),

        // ── Whitespace / editing / punctuation ───────────────────────────
        Key::Return => Usage(40),
        Key::Escape => Usage(41),
        Key::Backspace => Usage(42),
        Key::Tab => Usage(43),
        Key::Space => Usage(44),
        Key::Minus => Usage(45),
        Key::Equal => Usage(46),
        Key::LeftBracket => Usage(47),
        Key::RightBracket => Usage(48),
        Key::BackSlash => Usage(49),
        Key::SemiColon => Usage(51),
        Key::Quote => Usage(52),
        Key::BackQuote => Usage(53),
        Key::Comma => Usage(54),
        Key::Dot => Usage(55),
        Key::Slash => Usage(56),
        Key::CapsLock => Usage(57),

        // ── Function row ─────────────────────────────────────────────────
        Key::F1 => Usage(58),
        Key::F2 => Usage(59),
        Key::F3 => Usage(60),
        Key::F4 => Usage(61),
        Key::F5 => Usage(62),
        Key::F6 => Usage(63),
        Key::F7 => Usage(64),
        Key::F8 => Usage(65),
        Key::F9 => Usage(66),
        Key::F10 => Usage(67),
        Key::F11 => Usage(68),
        Key::F12 => Usage(69),

        // ── Navigation cluster ───────────────────────────────────────────
        Key::PrintScreen => Usage(70),
        Key::ScrollLock => Usage(71),
        Key::Pause => Usage(72),
        Key::Insert => Usage(73),
        Key::Home => Usage(74),
        Key::PageUp => Usage(75),
        Key::Delete => Usage(76),
        Key::End => Usage(77),
        Key::PageDown => Usage(78),
        Key::RightArrow => Usage(79),
        Key::LeftArrow => Usage(80),
        Key::DownArrow => Usage(81),
        Key::UpArrow => Usage(82),

        // ── Keypad ───────────────────────────────────────────────────────
        Key::NumLock => Usage(83),
        Key::KpDivide => Usage(84),
        Key::KpMultiply => Usage(85),
        Key::KpMinus => Usage(86),
        Key::KpPlus => Usage(87),
        Key::KpReturn => Usage(88),
        Key::Kp1 => Usage(89),
        Key::Kp2 => Usage(90),
        Key::Kp3 => Usage(91),
        Key::Kp4 => Usage(92),
        Key::Kp5 => Usage(93),
        Key::Kp6 => Usage(94),
        Key::Kp7 => Usage(95),
        Key::Kp8 => Usage(96),
        Key::Kp9 => Usage(97),
        Key::Kp0 => Usage(98),
        Key::KpDelete => Usage(99),
        Key::IntlBackslash => Usage(100),

        // No standard HID usage (macOS `fn`, unknown scancodes).
        Key::Function | Key::Unknown(_) => return None,
    };
    Some(mapping)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_match_appendix() {
        assert_eq!(map_key(&Key::KeyA), Some(Mapping::Usage(4)));
        assert_eq!(map_key(&Key::KeyS), Some(Mapping::Usage(22))); // 0x16, used in Win+Shift+S
        assert_eq!(map_key(&Key::KeyZ), Some(Mapping::Usage(29)));
    }

    #[test]
    fn arrows_match_appendix() {
        assert_eq!(map_key(&Key::RightArrow), Some(Mapping::Usage(0x4F)));
        assert_eq!(map_key(&Key::LeftArrow), Some(Mapping::Usage(0x50)));
        assert_eq!(map_key(&Key::DownArrow), Some(Mapping::Usage(0x51)));
        assert_eq!(map_key(&Key::UpArrow), Some(Mapping::Usage(0x52)));
    }

    #[test]
    fn modifiers_route_to_byte0_bits() {
        assert_eq!(map_key(&Key::MetaRight), Some(Mapping::Modifier(0x80)));
        assert_eq!(map_key(&Key::ShiftRight), Some(Mapping::Modifier(0x20)));
        assert_eq!(map_key(&Key::ShiftLeft), Some(Mapping::Modifier(0x02)));
        assert_eq!(map_key(&Key::ControlLeft), Some(Mapping::Modifier(0x01)));
    }

    #[test]
    fn keypad_and_numbers() {
        assert_eq!(map_key(&Key::Num1), Some(Mapping::Usage(30)));
        assert_eq!(map_key(&Key::Num0), Some(Mapping::Usage(39)));
        assert_eq!(map_key(&Key::Kp0), Some(Mapping::Usage(98)));
        assert_eq!(map_key(&Key::Kp9), Some(Mapping::Usage(97)));
    }

    #[test]
    fn unmapped_keys_return_none() {
        assert_eq!(map_key(&Key::Function), None);
        assert_eq!(map_key(&Key::Unknown(9999)), None);
    }

    /// Every common typing key must resolve, or lock mode would silently drop
    /// it (plan §13 "assert no unmapped common keys").
    #[test]
    fn all_common_keys_are_mapped() {
        let common = [
            Key::KeyA, Key::KeyB, Key::KeyC, Key::KeyD, Key::KeyE, Key::KeyF,
            Key::KeyG, Key::KeyH, Key::KeyI, Key::KeyJ, Key::KeyK, Key::KeyL,
            Key::KeyM, Key::KeyN, Key::KeyO, Key::KeyP, Key::KeyQ, Key::KeyR,
            Key::KeyS, Key::KeyT, Key::KeyU, Key::KeyV, Key::KeyW, Key::KeyX,
            Key::KeyY, Key::KeyZ,
            Key::Num0, Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5,
            Key::Num6, Key::Num7, Key::Num8, Key::Num9,
            Key::Return, Key::Escape, Key::Backspace, Key::Tab, Key::Space,
            Key::Minus, Key::Equal, Key::LeftBracket, Key::RightBracket,
            Key::BackSlash, Key::SemiColon, Key::Quote, Key::BackQuote,
            Key::Comma, Key::Dot, Key::Slash, Key::CapsLock,
            Key::F1, Key::F2, Key::F3, Key::F4, Key::F5, Key::F6, Key::F7,
            Key::F8, Key::F9, Key::F10, Key::F11, Key::F12,
            Key::Home, Key::End, Key::PageUp, Key::PageDown, Key::Insert,
            Key::Delete, Key::UpArrow, Key::DownArrow, Key::LeftArrow,
            Key::RightArrow,
            Key::ControlLeft, Key::ShiftLeft, Key::Alt, Key::MetaLeft,
            Key::ControlRight, Key::ShiftRight, Key::AltGr, Key::MetaRight,
        ];
        for key in &common {
            assert!(map_key(key).is_some(), "common key not mapped: {key:?}");
        }
    }
}
