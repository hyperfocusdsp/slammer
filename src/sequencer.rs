//! Simple 16-step pattern sequencer.
//!
//! State is shared between the UI (click to toggle steps, read playhead for
//! highlight) and the audio thread (advance counter, read steps, fire
//! triggers) via plain atomics — no locks, RT-safe.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicUsize, Ordering};

use parking_lot::Mutex;

pub const STEPS: usize = 16;

/// Default pattern: four-on-the-floor (steps 0, 4, 8, 12).
pub const DEFAULT_STEP_BITS: u16 = 0x1111;
const DEFAULT_BPM: f32 = 120.0;
const MIN_BPM: f32 = 40.0;
const MAX_BPM: f32 = 240.0;

pub struct Sequencer {
    pub steps: [AtomicBool; STEPS],
    /// Per-step flam state. 0 = Off, 1 = Flam (2 hits), 2 = Ruff (3 hits),
    /// 3 = Roll (4 hits). Written by the UI thread via `cycle_flam_state`,
    /// read by the audio thread on step-boundary to decide how many hits
    /// to schedule via `KickEngine::schedule_group`.
    pub flam_state: [AtomicU8; STEPS],
    /// User-controlled run flag (standalone only — ignored when `host_synced`).
    pub running: AtomicBool,
    /// Standalone BPM stored as milli-BPM so we can use an integer atomic.
    bpm_milli: AtomicU32,
    /// Last step played — audio thread writes, UI reads for playhead display.
    pub current_step: AtomicUsize,
    /// True once the audio thread has detected a real DAW transport (i.e.
    /// transport.playing has ever reported false). Set by the audio thread;
    /// read by the UI to disable standalone controls and hide the spacebar.
    pub host_synced: AtomicBool,
    /// BPM the UI should display: host tempo when synced, standalone BPM
    /// otherwise. Stored as milli-BPM.
    display_bpm_milli: AtomicU32,
    /// True when the sequencer is actively stepping (either user ran it in
    /// standalone mode, or host transport is playing in a DAW). UI reads
    /// this to render the PLAY/STOP button state and the playhead.
    pub running_effective: AtomicBool,
    /// Set by the audio thread on its first `process()` call — signals that
    /// `host_synced` now reflects reality. The editor uses this to defer
    /// standalone-only actions (e.g. restoring last session) until after
    /// the DAW/standalone decision has been made.
    pub transport_probed: AtomicBool,
    /// UI-thread mirror of the step bitmask for DAW/standalone state
    /// persistence. The audio thread never touches this — only the UI
    /// thread (via `toggle_step` / `set_step`) and `initialize()`.
    persist_mirror: Arc<Mutex<u16>>,
}

impl Sequencer {
    /// Build a new sequencer. The `persist_mirror` is the same
    /// `Arc<Mutex<u16>>` stored on `SlammerParams` as a `#[persist]` field;
    /// passing it in here lets the UI thread keep the serialized pattern
    /// in sync with the live atomics on every edit. The initial atomic
    /// state is seeded from the mirror's current value, so DAW-restored
    /// state wins over the 4/4 default, and fresh instances (which carry
    /// the `DEFAULT_STEP_BITS` default on the mirror) come up with a
    /// four-on-the-floor kick pattern.
    pub fn new(persist_mirror: Arc<Mutex<u16>>) -> Self {
        let initial_bits = *persist_mirror.lock();
        Self {
            steps: std::array::from_fn(|i| {
                AtomicBool::new((initial_bits >> i) & 1 != 0)
            }),
            flam_state: std::array::from_fn(|_| AtomicU8::new(0)),
            running: AtomicBool::new(false),
            bpm_milli: AtomicU32::new((DEFAULT_BPM * 1000.0) as u32),
            current_step: AtomicUsize::new(0),
            host_synced: AtomicBool::new(false),
            display_bpm_milli: AtomicU32::new((DEFAULT_BPM * 1000.0) as u32),
            running_effective: AtomicBool::new(false),
            transport_probed: AtomicBool::new(false),
            persist_mirror,
        }
    }

    /// Copy the persist-mirror bitmask into the step atomics. Called once
    /// from `Plugin::initialize()` after nih-plug has deserialized the
    /// `#[persist]` field, so DAW-restored patterns reach the audio
    /// thread before the first `process()` call.
    pub fn restore_from_persist(&self) {
        let bits = *self.persist_mirror.lock();
        for i in 0..STEPS {
            self.steps[i].store((bits >> i) & 1 != 0, Ordering::Relaxed);
        }
    }

    pub fn display_bpm(&self) -> f32 {
        self.display_bpm_milli.load(Ordering::Relaxed) as f32 / 1000.0
    }

    pub fn set_display_bpm(&self, bpm: f32) {
        self.display_bpm_milli
            .store((bpm * 1000.0) as u32, Ordering::Relaxed);
    }

    pub fn is_host_synced(&self) -> bool {
        self.host_synced.load(Ordering::Relaxed)
    }

    pub fn is_running_effective(&self) -> bool {
        self.running_effective.load(Ordering::Relaxed)
    }

    pub fn bpm(&self) -> f32 {
        self.bpm_milli.load(Ordering::Relaxed) as f32 / 1000.0
    }

    pub fn set_bpm(&self, bpm: f32) {
        let clamped = bpm.clamp(MIN_BPM, MAX_BPM);
        self.bpm_milli
            .store((clamped * 1000.0) as u32, Ordering::Relaxed);
    }

    pub fn is_step_on(&self, idx: usize) -> bool {
        self.steps[idx].load(Ordering::Relaxed)
    }

    /// Audio-thread read: current per-step flam state (0..=3). 0 = single
    /// hit (no flam), 1 = Flam (2 hits), 2 = Ruff (3), 3 = Roll (4).
    pub fn flam_state(&self, idx: usize) -> u8 {
        self.flam_state[idx].load(Ordering::Relaxed)
    }

    /// UI-thread only: advance the per-step flam state through
    /// Off → Flam → Ruff → Roll → Off. No-op when the step is currently
    /// inactive, since a flam on a rest is meaningless.
    pub fn cycle_flam_state(&self, idx: usize) {
        if !self.is_step_on(idx) {
            return;
        }
        let next = (self.flam_state[idx].load(Ordering::Relaxed) + 1) & 0b11;
        self.flam_state[idx].store(next, Ordering::Relaxed);
    }

    /// UI-thread only: flip a step on/off and mirror the change into the
    /// persist bitmask.
    pub fn toggle_step(&self, idx: usize) {
        let prev = self.steps[idx].fetch_xor(true, Ordering::Relaxed);
        let mut bits = self.persist_mirror.lock();
        *bits ^= 1u16 << idx;
        let _ = prev;
    }

    /// UI-thread only: set a step to an explicit state. Used by the
    /// click-drag paint path so repeated writes as the pointer moves are
    /// idempotent (unlike `toggle_step`, which would oscillate).
    pub fn set_step(&self, idx: usize, on: bool) {
        self.steps[idx].store(on, Ordering::Relaxed);
        if !on {
            self.flam_state[idx].store(0, Ordering::Relaxed);
        }
        let mut bits = self.persist_mirror.lock();
        if on {
            *bits |= 1u16 << idx;
        } else {
            *bits &= !(1u16 << idx);
        }
    }

    pub fn current(&self) -> usize {
        self.current_step.load(Ordering::Relaxed)
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    pub fn toggle_running(&self) {
        let prev = self.running.load(Ordering::Relaxed);
        self.running.store(!prev, Ordering::Relaxed);
    }
}

impl Default for Sequencer {
    fn default() -> Self {
        Self::new(Arc::new(Mutex::new(DEFAULT_STEP_BITS)))
    }
}

#[cfg(test)]
mod flam_state_tests {
    use super::*;

    #[test]
    fn flam_state_default_off() {
        let seq = Sequencer::default();
        for i in 0..STEPS {
            assert_eq!(seq.flam_state(i), 0);
        }
    }

    #[test]
    fn cycle_flam_state_walks_four_states() {
        let seq = Sequencer::default();
        seq.set_step(0, true);
        assert_eq!(seq.flam_state(0), 0);
        seq.cycle_flam_state(0);
        assert_eq!(seq.flam_state(0), 1);
        seq.cycle_flam_state(0);
        assert_eq!(seq.flam_state(0), 2);
        seq.cycle_flam_state(0);
        assert_eq!(seq.flam_state(0), 3);
        seq.cycle_flam_state(0);
        assert_eq!(seq.flam_state(0), 0);
    }

    #[test]
    fn turning_step_off_clears_flam_state() {
        let seq = Sequencer::default();
        seq.set_step(5, true);
        seq.cycle_flam_state(5);
        seq.cycle_flam_state(5);
        assert_eq!(seq.flam_state(5), 2);
        seq.set_step(5, false);
        assert_eq!(seq.flam_state(5), 0);
    }

    #[test]
    fn cycle_on_inactive_step_is_noop() {
        let seq = Sequencer::default();
        seq.set_step(7, false);
        seq.cycle_flam_state(7);
        assert_eq!(seq.flam_state(7), 0);
    }
}
