//! Simple 16-step pattern sequencer.
//!
//! State is shared between the UI (click to toggle steps, read playhead for
//! highlight) and the audio thread (advance counter, read steps, fire
//! triggers) via plain atomics — no locks, RT-safe.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use parking_lot::Mutex;

pub const STEPS: usize = 16;

/// Default pattern: four-on-the-floor (steps 0, 4, 8, 12).
pub const DEFAULT_STEP_BITS: u16 = 0x1111;
const DEFAULT_BPM: f32 = 120.0;
const MIN_BPM: f32 = 40.0;
const MAX_BPM: f32 = 240.0;

pub struct Sequencer {
    pub steps: [AtomicBool; STEPS],
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
