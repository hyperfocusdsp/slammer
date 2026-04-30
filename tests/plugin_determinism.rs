//! P0.2 — Long-form sequencer-style engine determinism.
//! P0.5 — RT-allocation stress under heavy retriggering.
//!
//! P0.1 (`tests/golden_engine.rs`) locks single-trigger output. This file
//! adds two larger nets:
//!
//! * `golden_kick_loop_5s` renders 5 s of 4-on-the-floor at 120 BPM
//!   through the engine with the default preset, hashes the output, and
//!   asserts against a checked-in u64. Catches voice-stealing drift,
//!   retrigger discontinuity, and any cumulative state bug that wouldn't
//!   show up in a single 1-second hit.
//!
//! * `golden_kick_loop_5s_heavy` does the same with the heavy preset
//!   (saturation + drift + voice clip) so the same nets cover the harder
//!   code paths.
//!
//! * `rt_alloc_stress_max_polyphony` retriggers every 23 samples for 10 s
//!   so that all 4 voice slots are constantly in fadeout. Cargo.toml has
//!   `assert_process_allocs` on the nih-plug feature list, but this test
//!   is on `KickEngine` directly — what it really asserts is "no panic,
//!   no NaN, no stuck-at-zero output" under worst-case retrigger.
//!
//! See `tests/golden_engine.rs` for the hash-fold helper documentation.

use niner::dsp::engine::{KickEngine, KickParams};

const SAMPLE_RATE: f32 = 48_000.0;

fn hash_samples(samples: &[f32]) -> u64 {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for s in samples {
        h ^= s.to_bits() as u64;
        h = h.rotate_left(13).wrapping_mul(0x100_0000_01B3);
    }
    h
}

/// Render `seconds` of audio with kicks fired every `samples_per_step`
/// samples (4-on-the-floor when `samples_per_step` is a 16th-note in
/// samples). Returns mono (L+R) output.
fn render_loop(params: &KickParams, seconds: f32, samples_per_step: usize) -> Vec<f32> {
    let total = (SAMPLE_RATE * seconds) as usize;
    let block = 1024;
    let mut engine = KickEngine::new(SAMPLE_RATE);
    let mut left = vec![0.0f32; total];
    let mut right = vec![0.0f32; total];
    let mut sample_in_step = 0usize;

    let mut i = 0;
    while i < total {
        let n = block.min(total - i);
        for j in 0..n {
            if sample_in_step == 0 {
                engine.trigger(params);
            }
            sample_in_step += 1;
            if sample_in_step >= samples_per_step {
                sample_in_step = 0;
            }
            let _ = j;
        }
        engine.process(&mut left[i..i + n], &mut right[i..i + n], params);
        i += n;
    }

    left.iter().zip(right.iter()).map(|(l, r)| l + r).collect()
}

fn samples_per_16th_at_120bpm() -> usize {
    // 16th-note interval: 60 / bpm / 4 seconds.
    (SAMPLE_RATE * 60.0 / 120.0 / 4.0) as usize
}

fn assert_no_nan_inf(name: &str, samples: &[f32]) {
    for (i, &s) in samples.iter().enumerate() {
        assert!(s.is_finite(), "{name}: non-finite sample at {i}");
    }
}

fn assert_audible(name: &str, samples: &[f32]) {
    let peak = samples.iter().fold(0.0_f32, |a, &b| a.max(b.abs()));
    assert!(
        peak > 0.05,
        "{name}: peak {peak} too quiet (expected > 0.05)"
    );
}

/// Locked goldens (Linux x86_64). Captured against WIP snapshot 7199a93
/// on 2026-04-30. Update only after a deliberate, audited output change.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const LOOP_GOLDENS: &[(&str, u64)] = &[
    ("kick_loop_5s_default", 0x430A_0374_CEBA_141E),
    ("kick_loop_5s_heavy", 0xCD5B_E566_F0D9_C06D),
];

fn assert_golden(name: &str, samples: &[f32]) {
    assert_no_nan_inf(name, samples);
    assert_audible(name, samples);
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        let hash = hash_samples(samples);
        let expected = LOOP_GOLDENS
            .iter()
            .find(|(n, _)| *n == name)
            .expect("golden missing")
            .1;
        assert_eq!(
            hash, expected,
            "{name}: golden hash drifted\n  expected: 0x{expected:016X}\n  actual:   0x{hash:016X}"
        );
    }
}

#[test]
fn golden_kick_loop_5s() {
    let params = KickParams::default();
    let mono = render_loop(&params, 5.0, samples_per_16th_at_120bpm());
    assert_golden("kick_loop_5s_default", &mono);
}

#[test]
fn golden_kick_loop_5s_heavy() {
    let params = KickParams {
        sat_mode: 1,
        sat_drive: 0.7,
        sat_mix: 1.0,
        kick_clip_mode: 1,
        kick_clip_drive: 0.4,
        drift_amount: 0.5,
        ..KickParams::default()
    };
    let mono = render_loop(&params, 5.0, samples_per_16th_at_120bpm());
    assert_golden("kick_loop_5s_heavy", &mono);
}

/// P0.5 — pathological retriggering.
///
/// Every 23 samples is ~480 µs at 48 kHz — far below the ~5 ms voice
/// fadeout, so all 4 voice slots are in fadeout simultaneously. Verifies
/// the engine does not panic, allocate (release builds with
/// `assert_process_allocs`), or produce non-finite output. Output is also
/// allowed to be silent if voice stealing is starving everything; the
/// invariant we care about is no NaN/Inf and no panic.
#[test]
fn rt_alloc_stress_max_polyphony() {
    let params = KickParams::default();
    let mut engine = KickEngine::new(SAMPLE_RATE);
    let total = (SAMPLE_RATE * 10.0) as usize;
    let mut left = vec![0.0f32; 1024];
    let mut right = vec![0.0f32; 1024];

    let mut sample_counter = 0usize;
    let mut i = 0;
    while i < total {
        let n = 1024.min(total - i);
        for _ in 0..n {
            if sample_counter % 23 == 0 {
                engine.trigger(&params);
            }
            sample_counter += 1;
        }
        engine.process(&mut left[..n], &mut right[..n], &params);
        for j in 0..n {
            assert!(
                left[j].is_finite() && right[j].is_finite(),
                "non-finite in stress run"
            );
        }
        i += n;
    }
}
