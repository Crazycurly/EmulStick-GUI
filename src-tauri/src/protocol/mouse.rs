//! F803 mouse report encoder (6 bytes, no Report ID).
//!
//! Layout: `[Buttons, X_low, X_high, Y_low, Y_high, Wheel]`.
//! X/Y are signed 16-bit little-endian, valid range −2047..+2047.
//! Wheel is signed 8-bit, valid range −127..+127. See `docs/protocol.md` §IV.

/// Length of an F803 mouse report in bytes.
pub const MOUSE_REPORT_LEN: usize = 6;

/// Inclusive bounds for a single packet's X/Y axis value.
pub const AXIS_MIN: i32 = -2047;
pub const AXIS_MAX: i32 = 2047;

/// Inclusive bounds for the wheel field.
pub const WHEEL_MIN: i32 = -127;
pub const WHEEL_MAX: i32 = 127;

// Compile-time guard: these bounds must fit the wire types `encode_report` casts
// to (`i16` for the axes, `i8` for the wheel), so widening a constant past the
// field width fails the build instead of silently wrapping at runtime.
const _: () = assert!(AXIS_MAX <= i16::MAX as i32 && AXIS_MIN >= i16::MIN as i32);
const _: () = assert!(WHEEL_MAX <= i8::MAX as i32 && WHEEL_MIN >= i8::MIN as i32);

/// Mouse button bit masks for byte 0.
pub mod button {
    pub const LEFT: u8 = 1 << 0;
    pub const RIGHT: u8 = 1 << 1;
    pub const MIDDLE: u8 = 1 << 2;
    pub const BUTTON4: u8 = 1 << 3;
    pub const BUTTON5: u8 = 1 << 4;
}

/// Encode a single F803 report. `dx`/`dy` are clamped to ±2047 and `wheel`
/// to ±127 — use [`split_reports`] when an accumulated delta may exceed the
/// per-packet axis range and must not be truncated.
pub fn encode_report(buttons: u8, dx: i32, dy: i32, wheel: i32) -> [u8; MOUSE_REPORT_LEN] {
    let dx = dx.clamp(AXIS_MIN, AXIS_MAX) as i16;
    let dy = dy.clamp(AXIS_MIN, AXIS_MAX) as i16;
    let wheel = wheel.clamp(WHEEL_MIN, WHEEL_MAX) as i8;
    let [x_low, x_high] = dx.to_le_bytes();
    let [y_low, y_high] = dy.to_le_bytes();
    [buttons, x_low, x_high, y_low, y_high, wheel as u8]
}

/// Split an accumulated delta into one or more reports, **never truncating**
/// the axes: an axis exceeding ±2047 is emitted across multiple packets so fast
/// flicks track 1:1 (plan §6.3). The wheel is clamped to ±127 here as a safety
/// net and ridden only on the first packet — the caller is responsible for
/// pre-clamping it and carrying any remainder forward (see the writer's
/// `flush_mouse`), so wheel ticks beyond a single packet aren't lost either.
/// Buttons are carried on every emitted packet. Always returns at least one
/// packet, so a button-only or wheel-only change still flushes.
pub fn split_reports(
    buttons: u8,
    mut dx: i32,
    mut dy: i32,
    wheel: i32,
) -> Vec<[u8; MOUSE_REPORT_LEN]> {
    let mut out = Vec::new();
    let mut wheel = wheel.clamp(WHEEL_MIN, WHEEL_MAX);
    loop {
        let step_x = dx.clamp(AXIS_MIN, AXIS_MAX);
        let step_y = dy.clamp(AXIS_MIN, AXIS_MAX);
        out.push(encode_report(buttons, step_x, step_y, wheel));
        dx -= step_x;
        dy -= step_y;
        wheel = 0; // wheel only rides the first packet
        if dx == 0 && dy == 0 {
            break;
        }
    }
    out
}

/// The all-buttons-up, no-motion report (`00 × 6`). Sent on
/// disconnect/lock-exit/panic so no button is left stuck pressed (plan §9).
pub const MOUSE_RELEASE_ALL: [u8; MOUSE_REPORT_LEN] = [0u8; MOUSE_REPORT_LEN];

#[cfg(test)]
mod tests {
    use super::button::*;
    use super::*;

    /// docs/protocol.md Example 2 — the four worked mouse reports.
    #[test]
    fn worked_examples() {
        assert_eq!(encode_report(0, 6, 12, 0), [0x00, 0x06, 0x00, 0x0C, 0x00, 0x00]);
        assert_eq!(encode_report(0, -2, 3, 0), [0x00, 0xFE, 0xFF, 0x03, 0x00, 0x00]);
        assert_eq!(encode_report(LEFT, 0, 0, 0), [0x01, 0x00, 0x00, 0x00, 0x00, 0x00]);
        assert_eq!(encode_report(0, 0, 0, 0), [0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn negative_y_and_wheel_little_endian() {
        // (-2047, -2047), wheel -1 → both axes at the negative limit.
        assert_eq!(
            encode_report(0, -2047, -2047, -1),
            [0x00, 0x01, 0xF8, 0x01, 0xF8, 0xFF]
        );
    }

    #[test]
    fn single_packet_clamps_axes_and_wheel() {
        let r = encode_report(0, 5000, -5000, 200);
        assert_eq!(r, [0x00, 0xFF, 0x07, 0x01, 0xF8, 0x7F]); // +2047, -2047, +127
    }

    #[test]
    fn split_emits_one_packet_when_in_range() {
        let packets = split_reports(RIGHT, 100, -50, 3);
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0], encode_report(RIGHT, 100, -50, 3));
    }

    #[test]
    fn split_never_truncates_large_delta() {
        let packets = split_reports(0, 5000, 0, 0);
        // 5000 = 2047 + 2047 + 906, summed back exactly with no loss.
        let total: i32 = packets
            .iter()
            .map(|p| i16::from_le_bytes([p[1], p[2]]) as i32)
            .sum();
        assert_eq!(total, 5000);
        assert_eq!(packets.len(), 3);
    }

    #[test]
    fn split_wheel_only_on_first_packet() {
        let packets = split_reports(0, 4100, 0, 5);
        assert_eq!(packets[0][5] as i8, 5);
        for p in &packets[1..] {
            assert_eq!(p[5], 0);
        }
    }

    #[test]
    fn split_button_only_still_flushes_once() {
        let packets = split_reports(LEFT, 0, 0, 0);
        assert_eq!(packets, vec![[0x01, 0, 0, 0, 0, 0]]);
    }
}
