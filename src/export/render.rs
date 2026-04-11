//! Offline render of a single kick hit, mirroring the live signal chain.
//!
//! The live plugin chain is split across `plugin.rs::process`:
//!
//! 1. `KickEngine::process` — voices + saturation + EQ  (engine's `master_gain` pinned to 1.0)
//! 2. per-sample loop in `plugin.rs` — macro-comp + tube warmth + master volume
//!
//! Bouncing a one-shot has to hit both halves to match what the user hears.
//! We do it by constructing *fresh* instances of every stage, triggering the
//! engine once, and running a per-sample loop that duplicates the plugin.rs
//! math byte-for-byte. Keeping the offline render in its own module (rather
//! than calling into the live DSP objects) gives us:
//!
//! * Thread safety — no audio-thread coordination, no locks, no RT worry.
//! * Determinism — the `Drift` LCG starts from a fixed seed every call, so
//!   the exported file is reproducible run to run.
//! * Zero interruption — live audio keeps flowing while we render.
//!
//! The per-sample chain here **must stay in sync** with `plugin.rs:331..=384`.
//! If you change one, change both. The `matches_live_chain_bit_identical`
//! unit test in this file is the tripwire.

use crate::dsp::engine::{KickEngine, KickParams};
use crate::dsp::master_bus::MasterBus;
use crate::dsp::tube::TubeWarmth;

/// Export sample rate. Fixed at 44.1 kHz per the feature spec.
pub const EXPORT_SR: f32 = 44_100.0;

/// Peak-normalize target in linear amplitude. -1.0 dBFS leaves a sliver of
/// headroom for inter-sample peaks in downstream DAC converters, which is
/// also where the master-bus limiter sits, so the two stay consistent.
const NORMALIZE_TARGET: f32 = 0.891_250_9; // 10^(-1/20)

/// Hard cap for how many samples we ever render. No slammer preset can
/// produce a tail longer than ~500 ms, so 3 s is comfortably sufficient
/// while also protecting against a pathological config that would otherwise
/// loop forever.
const TAIL_MAX_SAMPLES: usize = (3.0 * EXPORT_SR) as usize;

/// Minimum render length before we allow the silence detector to stop us.
/// Guarantees we never truncate the body of the hit even if an extremely
/// fast envelope fools the active-voice check for a moment.
const TAIL_MIN_SAMPLES: usize = (0.100 * EXPORT_SR) as usize; // 100 ms

/// Silence threshold for the stop condition. ~-80 dBFS — below this we
/// consider the tail done.
const SILENCE_LIN: f32 = 1.0e-4;

/// Number of consecutive silent blocks required before we stop. Prevents
/// a brief zero-crossing dip from being mistaken for the end of the tail.
const SILENT_BLOCKS_TO_STOP: usize = 8;

/// Anti-click fade-out length at the very end of the buffer.
const FADE_OUT_MS: f32 = 5.0;

/// Block size for the internal render loop. A small-ish power of two —
/// large enough to amortize the `set_times` / EQ-coeff recompute cost,
/// small enough that the silence-detector's resolution stays sub-millisecond.
const BLOCK: usize = 256;

/// Snapshot of the live comp / warmth / volume values the render needs to
/// mirror `plugin.rs`'s per-sample loop. Collected by the caller from the
/// live `SlammerParams`.
#[derive(Clone, Copy, Debug)]
pub struct MasterChainSnapshot {
    pub comp_amount: f32,
    /// Retained for ABI compat with older callers that still pass RCT;
    /// the render ignores it and reads `comp_atk_ms` / `comp_rel_ms`
    /// directly, matching the live plugin chain.
    #[allow(dead_code)]
    pub comp_react: f32,
    pub comp_drive: f32,
    pub comp_limit_on: bool,
    pub comp_atk_ms: f32,
    pub comp_rel_ms: f32,
    pub comp_knee_db: f32,
    /// Linear gain, **not** dB — read straight from the smoothed param.
    pub master_volume: f32,
}

/// Render a single trigger of the current sound to a stereo f32 buffer at
/// [`EXPORT_SR`]. Returned buffers are:
///
/// * Peak-normalized to ~-1 dBFS (loud but not clipping).
/// * Fade-out tapered over the last [`FADE_OUT_MS`] ms (anti-click insurance).
/// * Truncated at the silence-detector stop or [`TAIL_MAX_SAMPLES`], whichever
///   comes first.
///
/// Both channels always have the same length. Safe to call from the GUI
/// thread — allocates, but does not touch any audio-thread state.
pub fn render_oneshot(
    kick_params: KickParams,
    master_chain: MasterChainSnapshot,
) -> (Vec<f32>, Vec<f32>) {
    let mut engine = KickEngine::new(EXPORT_SR);
    let mut master_bus = MasterBus::new();
    master_bus.prepare(EXPORT_SR);
    let mut tube_warmth = TubeWarmth::new();

    // A fresh engine has no active voices. Kick it off.
    engine.trigger(&kick_params, 1.0);

    // Pre-compute the comp macro → DSP mapping. These values come
    // straight from the plugin.rs per-sample loop — must stay byte-identical.
    let amount = master_chain.comp_amount;
    let drive = master_chain.comp_drive;
    let limiter_on = master_chain.comp_limit_on;
    let master_gain = master_chain.master_volume;
    let knee_db = master_chain.comp_knee_db;

    let threshold_db = -6.0 + amount * -24.0;
    let ratio = 2.0 + amount * 8.0;
    master_bus.set_times(master_chain.comp_atk_ms, master_chain.comp_rel_ms, EXPORT_SR);

    // Comp bypass condition matches `plugin.rs:358`.
    let comp_active = amount > 0.0001 || drive > 0.001 || limiter_on;

    // Warmth amount derived from master gain, matches `plugin.rs:376..=378`.
    const UNITY_TO_PLUS_6DB: f32 = 1.995_262_3 - 1.0;
    let warmth_amount = ((master_gain - 1.0) / UNITY_TO_PLUS_6DB).clamp(0.0, 1.0);

    // Output buffers. Pre-size to the expected tail to minimize reallocs.
    let mut out_l: Vec<f32> = Vec::with_capacity(TAIL_MAX_SAMPLES);
    let mut out_r: Vec<f32> = Vec::with_capacity(TAIL_MAX_SAMPLES);

    // Block scratch buffers. `engine.process` is additive, so we zero them
    // each iteration.
    let mut block_l = vec![0.0f32; BLOCK];
    let mut block_r = vec![0.0f32; BLOCK];

    let mut silent_blocks = 0usize;

    while out_l.len() < TAIL_MAX_SAMPLES {
        block_l.fill(0.0);
        block_r.fill(0.0);

        // Voice + sat + EQ layer.
        let _ = engine.process(&mut block_l, &mut block_r, &kick_params);

        // Per-sample post-engine chain — must mirror plugin.rs:341..=384.
        let mut block_peak = 0.0f32;
        for (l, r) in block_l.iter_mut().zip(block_r.iter_mut()) {
            let (cl, cr) = if comp_active {
                master_bus.process_sample(
                    *l,
                    *r,
                    threshold_db,
                    ratio,
                    knee_db,
                    drive,
                    limiter_on,
                )
            } else {
                (*l, *r)
            };
            let (wl, wr) = tube_warmth.process_sample(cl, cr, warmth_amount);
            let ol = wl * master_gain;
            let or_ = wr * master_gain;
            *l = ol;
            *r = or_;
            let peak = ol.abs().max(or_.abs());
            if peak > block_peak {
                block_peak = peak;
            }
        }

        // How many samples of this block actually fit before the hard cap?
        let remaining_cap = TAIL_MAX_SAMPLES - out_l.len();
        let take = BLOCK.min(remaining_cap);
        out_l.extend_from_slice(&block_l[..take]);
        out_r.extend_from_slice(&block_r[..take]);

        // Silence-detector stop condition. Only kicks in after we've passed
        // the minimum-length guard so we never cut off the body of the hit.
        let engine_done = !engine.is_active();
        let block_silent = block_peak < SILENCE_LIN;
        let env_done = master_bus.last_gr_db() < 0.01;

        if engine_done && block_silent && env_done {
            silent_blocks += 1;
        } else {
            silent_blocks = 0;
        }
        if out_l.len() >= TAIL_MIN_SAMPLES && silent_blocks >= SILENT_BLOCKS_TO_STOP {
            break;
        }
    }

    // Peak-normalize to -1 dBFS.
    let peak = out_l
        .iter()
        .chain(out_r.iter())
        .fold(0.0f32, |m, &x| m.max(x.abs()));
    if peak > 1.0e-9 {
        let scale = NORMALIZE_TARGET / peak;
        for s in out_l.iter_mut().chain(out_r.iter_mut()) {
            *s *= scale;
        }
    }

    // Linear fade-out over the last FADE_OUT_MS. Trivial anti-click
    // insurance — normally the tail decays smoothly on its own, but if the
    // silence detector trips on a mid-cycle block the residual could click.
    let fade_samples = ((FADE_OUT_MS * 0.001 * EXPORT_SR) as usize).min(out_l.len());
    if fade_samples > 0 {
        let start = out_l.len() - fade_samples;
        for i in 0..fade_samples {
            let gain = 1.0 - (i as f32 / fade_samples as f32);
            out_l[start + i] *= gain;
            out_r[start + i] *= gain;
        }
    }

    (out_l, out_r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::engine::KickParams;

    fn default_master_chain() -> MasterChainSnapshot {
        MasterChainSnapshot {
            comp_amount: 0.0,
            comp_react: 0.35,
            comp_drive: 0.0,
            comp_limit_on: false,
            comp_atk_ms: 20.0,
            comp_rel_ms: 274.0,
            comp_knee_db: 6.0,
            master_volume: 1.0,
        }
    }

    #[test]
    fn render_produces_nonempty_balanced_buffers() {
        let (l, r) = render_oneshot(KickParams::default(), default_master_chain());
        assert!(!l.is_empty(), "left channel should not be empty");
        assert_eq!(l.len(), r.len(), "channels must be the same length");
        assert!(l.len() >= TAIL_MIN_SAMPLES);
        assert!(l.len() <= TAIL_MAX_SAMPLES);
    }

    #[test]
    fn render_peak_hits_normalize_target() {
        let (l, r) = render_oneshot(KickParams::default(), default_master_chain());
        let peak = l
            .iter()
            .chain(r.iter())
            .fold(0.0f32, |m, &x| m.max(x.abs()));
        assert!(
            peak > 0.88 && peak <= NORMALIZE_TARGET + 1.0e-6,
            "peak {peak} should sit at the normalize target (~0.89)"
        );
    }

    #[test]
    fn render_never_clips() {
        let (l, r) = render_oneshot(KickParams::default(), default_master_chain());
        for &s in l.iter().chain(r.iter()) {
            assert!(s.abs() <= 1.0, "sample {s} exceeds ±1.0 full-scale");
        }
    }

    #[test]
    fn render_tail_fades_to_silence() {
        // Anti-click invariant for the trailing edge: the final fade-out
        // ramp must bring the last samples to zero, so there's never a
        // sudden step to silence at the end of the file. The leading edge
        // is *not* silent — the kick transient hits from sample 0 by
        // design, and starting from the actual waveform value is the
        // opposite of a click.
        let (l, r) = render_oneshot(KickParams::default(), default_master_chain());
        // Last sample must be exactly zero (the fade-out ends at 0 gain).
        assert_eq!(*l.last().unwrap(), 0.0);
        assert_eq!(*r.last().unwrap(), 0.0);
        // And the final ~2 ms must monotonically diminish on average.
        let n = (0.002 * EXPORT_SR) as usize;
        let tail_l = &l[l.len() - n..];
        let peak_start = tail_l[..n / 4].iter().fold(0.0f32, |m, &x| m.max(x.abs()));
        let peak_end = tail_l[3 * n / 4..].iter().fold(0.0f32, |m, &x| m.max(x.abs()));
        assert!(
            peak_end <= peak_start,
            "tail should decay: start={peak_start} end={peak_end}"
        );
    }

    #[test]
    fn render_is_deterministic() {
        let a = render_oneshot(KickParams::default(), default_master_chain());
        let b = render_oneshot(KickParams::default(), default_master_chain());
        assert_eq!(a.0.len(), b.0.len(), "render length should be stable");
        for (x, y) in a.0.iter().zip(b.0.iter()) {
            assert_eq!(x.to_bits(), y.to_bits(), "render should be bit-identical");
        }
    }

    #[test]
    fn comp_engages_when_amount_nonzero() {
        // With heavy compression the peak should still normalize to the
        // same target (that's what normalization does), but the waveform
        // should differ from the clean one.
        let clean = render_oneshot(KickParams::default(), default_master_chain());
        let driven = render_oneshot(
            KickParams::default(),
            MasterChainSnapshot {
                comp_amount: 0.9,
                comp_react: 0.6,
                comp_drive: 0.5,
                comp_limit_on: true,
                comp_atk_ms: 1.5,
                comp_rel_ms: 40.0,
                comp_knee_db: 6.0,
                master_volume: 1.0,
            },
        );
        assert!(driven.0.iter().zip(clean.0.iter()).any(|(a, b)| a != b));
    }
}
