//! OS-level input hooking (plan §4.2, milestone M2 — not yet wired).
//!
//! This will host the dedicated `rdev::grab` thread. The grab callback runs
//! synchronously and must never block or do I/O (plan §8): it only maps OS
//! events to HID reports via [`crate::protocol`] and pushes them onto an
//! `mpsc`/`crossbeam` channel consumed by the BLE async task, which applies the
//! coalescing/clamping in [`crate::protocol::mouse::split_reports`].
//!
//! Key behaviours to implement here in M2:
//! * Return `None` from the grab callback to *consume* reserved combos
//!   (`Cmd+Tab`, `Win`) so they don't fire on the operator machine.
//! * Honour the passthrough flags: a channel that is off is not grabbed.
//! * Lock the cursor to window centre and capture relative deltas.
//! * RAII teardown so the grab is always removed and the cursor unlocked on
//!   lock exit / disconnect / panic.
