//! P0.1 — Golden engine determinism.
//!
//! Captures bit-stable hashes of the `KickEngine` output for a handful of
//! known parameter sets. Any P2 refactor that touches `engine.rs`,
//! `voice_clip.rs`, `saturation.rs`, `master_bus.rs`, etc. must leave these
//! hashes unchanged.
//!
//! ## How the hash works
//!
//! For each rendered f32 sample we fold its IEEE-754 bit pattern into a
//! running u64 via xor + 64-bit rotate. No external deps; deterministic on
//! any IEEE-754 platform. The fold preserves order (sample N matters), so
//! it catches any phase / amplitude drift introduced by a refactor.
//!
//! ## Updating goldens after an INTENTIONAL change
//!
//! If a refactor genuinely alters output (e.g. fixing a math bug), the test
//! prints the new hash on failure. Verify the change is intentional, then
//! paste the new u64 into the constant table below.
//!
//! ## Cross-platform note
//!
//! Hashes are captured on Linux x86_64. Other targets (macOS arm64, Windows
//! x86_64) may differ by a single ULP in some bins due to differing libm
//! implementations of `sin` / `exp`. The test gates strict-equality on
//! Linux x86_64; on other targets it asserts only that the output is
//! finite and non-zero (a smoke check, not a regression net).

use niner::dsp::engine::{KickEngine, KickParams};

const SAMPLE_RATE: f32 = 48_000.0;
const RENDER_SAMPLES: usize = 48_000; // 1 s mono

/// Fold a slice of f32 samples into a deterministic u64. XOR each sample's
/// IEEE-754 bit pattern into the accumulator, then rotate to mix position
/// into the hash so reordering changes the result.
fn hash_samples(samples: &[f32]) -> u64 {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325; // FNV-1a offset basis
    for s in samples {
        h ^= s.to_bits() as u64;
        h = h.rotate_left(13).wrapping_mul(0x100_0000_01B3); // FNV-1a-ish prime
    }
    h
}

/// Render one trigger of the engine for `RENDER_SAMPLES` and return the
/// summed mono output (L+R) plus its hash.
fn render(params: &KickParams) -> (Vec<f32>, u64) {
    let mut engine = KickEngine::new(SAMPLE_RATE);
    engine.trigger(params);

    let mut left = vec![0.0f32; RENDER_SAMPLES];
    let mut right = vec![0.0f32; RENDER_SAMPLES];
    let block = 256;
    let mut i = 0;
    while i < RENDER_SAMPLES {
        let n = block.min(RENDER_SAMPLES - i);
        let _peak = engine.process(&mut left[i..i + n], &mut right[i..i + n], params);
        i += n;
    }

    let mono: Vec<f32> = left.iter().zip(right.iter()).map(|(l, r)| l + r).collect();
    let h = hash_samples(&mono);
    (mono, h)
}

/// Default kit — vanilla 909-ish kick, no clap, no drift, no saturation.
fn preset_default() -> KickParams {
    KickParams::default()
}

/// Heavy preset — saturation engaged, drift on, voice clip on.
fn preset_heavy() -> KickParams {
    KickParams {
        sat_mode: 1, // tube
        sat_drive: 0.7,
        sat_mix: 1.0,
        kick_clip_mode: 1,
        kick_clip_drive: 0.4,
        drift_amount: 0.5,
        ..KickParams::default()
    }
}

/// Long-tail sub-heavy preset.
fn preset_sub_long() -> KickParams {
    KickParams {
        decay_ms: 800.0,
        sub_gain: 1.0,
        sub_fstart: 120.0,
        sub_fend: 35.0,
        sub_sweep_ms: 100.0,
        mid_gain: 0.2,
        top_gain: 0.1,
        ..KickParams::default()
    }
}

/// Clap-on preset.
fn preset_clap() -> KickParams {
    KickParams {
        clap_on: true,
        clap_level: 0.9,
        clap_freq: 1200.0,
        clap_tail_ms: 180.0,
        ..KickParams::default()
    }
}

/// Accented preset — sequencer accent flag flips on.
fn preset_accented() -> KickParams {
    KickParams {
        accent: true,
        accent_amount: 0.6,
        ..KickParams::default()
    }
}

/// Locked goldens (Linux x86_64). Update only after deliberate output change.
/// Captured against WIP snapshot 7199a93 on 2026-04-30.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const GOLDENS: &[(&str, u64)] = &[
    ("default", 0xD51A_D35C_8AA5_F0FC),
    ("heavy", 0xBAF1_3CF3_B940_B9E6),
    ("sub_long", 0x1BBC_7CC7_E31C_EBDB),
    ("clap", 0x3B90_A9DE_3906_5EDE),
    ("accented", 0x71A9_647A_1AE6_2752),
];

fn assert_golden(name: &str, mono: &[f32], hash: u64) {
    // Always-on sanity: output is finite and non-zero somewhere.
    assert!(
        mono.iter().all(|s| s.is_finite()),
        "{name}: non-finite sample in render"
    );
    let peak = mono.iter().fold(0.0_f32, |a, &b| a.max(b.abs()));
    assert!(
        peak > 1e-4,
        "{name}: render peaked at {peak}, expected > 1e-4"
    );

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        let expected = GOLDENS
            .iter()
            .find(|(n, _)| *n == name)
            .expect("golden missing")
            .1;
        assert_eq!(
            hash, expected,
            "{name}: golden hash drifted\n  expected: 0x{expected:016X}\n  actual:   0x{hash:016X}\n\
             If the change is intentional, paste the new value into GOLDENS."
        );
    }
    #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
    {
        let _ = hash; // silence unused on other platforms
    }
}

#[test]
fn golden_default() {
    let (mono, h) = render(&preset_default());
    assert_golden("default", &mono, h);
}

#[test]
fn golden_heavy() {
    let (mono, h) = render(&preset_heavy());
    assert_golden("heavy", &mono, h);
}

#[test]
fn golden_sub_long() {
    let (mono, h) = render(&preset_sub_long());
    assert_golden("sub_long", &mono, h);
}

#[test]
fn golden_clap() {
    let (mono, h) = render(&preset_clap());
    assert_golden("clap", &mono, h);
}

#[test]
fn golden_accented() {
    let (mono, h) = render(&preset_accented());
    assert_golden("accented", &mono, h);
}
