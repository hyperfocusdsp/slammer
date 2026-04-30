//! P0.4 — Sequencer persist-restore regression.
//!
//! Documents the contract that the audio thread must observe after a DAW
//! reload, captured here so any P2.1 / P2.9 refactor of the persist surface
//! immediately fails if it breaks the contract.
//!
//! ## The race
//!
//! `nih-plug` deserializes a plugin's `#[persist]` fields BEFORE
//! `Plugin::initialize()` runs. The sequencer's step + accent atomics are
//! NOT in the persist surface — only the `Arc<Mutex<u16>>` bitmask mirror
//! is. The atomics are populated either:
//!
//!   1. By `Sequencer::new(persist_mirror, accent_persist_mirror)` reading
//!      the (already-deserialized) bitmask at construction time, OR
//!   2. By an explicit `restore_from_persist()` call once the audio thread
//!      is up.
//!
//! Both paths must agree. The bug class this guards against:
//! `Sequencer::new` is called BEFORE the persist deserialization (e.g. in
//! `Plugin::default()`), atomics get default bits, and the first
//! `process()` reads a stale pattern. Recovery requires `restore_from_persist`
//! after the host has populated the mirror.

use niner::sequencer::{Sequencer, STEPS};
use parking_lot::Mutex;
use std::sync::Arc;

/// Non-trivial pattern: alternate every-2nd step plus an extra hit at 11.
const STEP_BITS: u16 = 0b1010_1010_1010_1010 | (1 << 11);
/// Accents on steps 0 and 11 only (must coincide with enabled steps).
const ACCENT_BITS: u16 = (1 << 0) | (1 << 11);

#[test]
fn sequencer_new_reads_persist_mirror_at_construction() {
    let persist = Arc::new(Mutex::new(STEP_BITS));
    let accent = Arc::new(Mutex::new(ACCENT_BITS));
    let seq = Sequencer::new(persist.clone(), accent.clone());

    for i in 0..STEPS {
        let want_step = (STEP_BITS >> i) & 1 != 0;
        let want_accent = (ACCENT_BITS >> i) & 1 != 0;
        assert_eq!(
            seq.is_step_on(i),
            want_step,
            "step {i} did not reflect persist mirror at Sequencer::new"
        );
        assert_eq!(
            seq.is_step_accented(i),
            want_accent,
            "accent {i} did not reflect persist mirror at Sequencer::new"
        );
    }
}

#[test]
fn restore_from_persist_recovers_pattern_after_late_mirror_update() {
    // Simulate the race: Sequencer constructed with default-zero mirrors
    // (mimicking `Niner::default()` running before nih-plug's persist
    // deserialize), then the host updates the mirror, then the audio thread
    // calls restore_from_persist() in `Plugin::initialize()`.
    let persist = Arc::new(Mutex::new(0u16));
    let accent = Arc::new(Mutex::new(0u16));
    let seq = Sequencer::new(persist.clone(), accent.clone());

    // Confirm baseline — no steps active.
    for i in 0..STEPS {
        assert!(!seq.is_step_on(i), "baseline: step {i} should be off");
        assert!(
            !seq.is_step_accented(i),
            "baseline: accent {i} should be off"
        );
    }

    // Host populates the mirror after Sequencer construction (the actual
    // race window in real plugins).
    *persist.lock() = STEP_BITS;
    *accent.lock() = ACCENT_BITS;

    // Atomics are still stale at this point.
    assert!(
        !seq.is_step_on(1),
        "atomics should still be stale before restore_from_persist"
    );

    // Recovery path — what `Plugin::initialize()` MUST call before the
    // first `process()`.
    seq.restore_from_persist();

    for i in 0..STEPS {
        let want_step = (STEP_BITS >> i) & 1 != 0;
        let want_accent = (ACCENT_BITS >> i) & 1 != 0;
        assert_eq!(
            seq.is_step_on(i),
            want_step,
            "step {i} should be {want_step} after restore_from_persist"
        );
        assert_eq!(
            seq.is_step_accented(i),
            want_accent,
            "accent {i} should be {want_accent} after restore_from_persist"
        );
    }
}

#[test]
fn toggle_step_writes_through_to_persist_mirror() {
    // The reverse path: UI edits to steps must propagate back into the
    // persist mirror so they survive a DAW project save+reload.
    let persist = Arc::new(Mutex::new(0u16));
    let accent = Arc::new(Mutex::new(0u16));
    let seq = Sequencer::new(persist.clone(), accent.clone());

    seq.toggle_step(3);
    assert_eq!(*persist.lock(), 1u16 << 3);

    seq.toggle_step(7);
    assert_eq!(*persist.lock(), (1u16 << 3) | (1u16 << 7));

    // Toggling a step OFF must also clear its accent in BOTH the atomic
    // and the persist mirror — guards the regression that landed when
    // accents were first added.
    seq.toggle_accent(3);
    assert_eq!(*accent.lock(), 1u16 << 3);
    seq.toggle_step(3); // turn step 3 OFF
    assert!(
        !seq.is_step_accented(3),
        "accent must clear when step turns off"
    );
    assert_eq!(
        *accent.lock() & (1u16 << 3),
        0,
        "persist accent bit must clear"
    );
}

#[test]
fn clear_pattern_zeros_both_mirrors() {
    let persist = Arc::new(Mutex::new(STEP_BITS));
    let accent = Arc::new(Mutex::new(ACCENT_BITS));
    let seq = Sequencer::new(persist.clone(), accent.clone());

    seq.clear_pattern();

    for i in 0..STEPS {
        assert!(!seq.is_step_on(i));
        assert!(!seq.is_step_accented(i));
    }
    assert_eq!(*persist.lock(), 0);
    assert_eq!(*accent.lock(), 0);
}
