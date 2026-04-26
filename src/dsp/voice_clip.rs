//! Per-voice soft-clip stage. Sits inside `KickVoice::tick` between the
//! layer mix (SUB + MID) and the final amp-envelope multiplication, so the
//! waveshaper sees the raw oscillator/noise sum at full level — independent
//! of envelope decay — and adds harmonics that survive into the tail.
//!
//! Why per-voice and not at the master bus? A real TR-909 kick takes its
//! sawtooth source through a soft-clipper (the "near-sine" shaping that
//! gives the 909 its body). That non-linearity sits BEFORE the VCA decay,
//! so the harmonic structure is dense at attack and thins as the envelope
//! closes. Master-bus saturation runs AFTER the envelope, so by the time
//! the tail reaches it there's nothing to shape. The two are complementary,
//! not redundant — keep both available.
//!
//! Modes (matched to `params::SatMode` numbering where it makes sense):
//!   0 = Off     — pass-through, zero CPU cost
//!   1 = Tanh    — symmetric soft-clip, odd harmonics, smooth roll-off
//!   2 = Diode   — asymmetric exponential, even-rich (matches the 909's
//!                 single-ended VCA character; `bd01.wav` analysis returns
//!                 `mode=Diode r2≈14×r3` in `tools/909-fit.py`)
//!   3 = Cubic   — fast `x - x³/3` soft-clip; cheaper than tanh, slightly
//!                 different harmonic content. Useful as a hard-vs-soft
//!                 reference when designing presets.
//!
//! All three modes are unity-pass at `drive = 0.0` (input returns
//! unchanged), so leaving the existing v0.5.x default 0.0/0.0 keeps every
//! preset's harmonic content bit-identical to v0.5.5.

pub const VC_OFF: u8 = 0;
pub const VC_TANH: u8 = 1;
pub const VC_DIODE: u8 = 2;
pub const VC_CUBIC: u8 = 3;

/// Apply the selected voice-clip mode at the given drive amount.
///
/// `drive` is in [0.0, 1.0]; values outside the range are clamped to keep
/// the shaper bounded under automation overshoot. At `drive = 0.0` the
/// function is the identity for every mode (no harmonic addition, no
/// behaviour change vs. earlier Slammer versions).
#[inline]
pub fn apply(mode: u8, drive: f32, x: f32) -> f32 {
    if drive <= 0.0 || mode == VC_OFF {
        return x;
    }
    let drive = drive.min(1.0);
    match mode {
        VC_TANH => tanh_clip(x, drive),
        VC_DIODE => diode_clip(x, drive),
        VC_CUBIC => cubic_clip(x, drive),
        _ => x,
    }
}

/// Symmetric `tanh` saturation. Drive scales pre-shaper gain; the output
/// is `tanh(g·x)` without post-normalisation, so for any finite input
/// the result is strictly bounded in (-1, 1). Even-symmetric waveforms
/// keep their symmetry (no DC offset). Loudness drops slightly as drive
/// rises — same as analog overdrive — which is what we want here.
#[inline]
fn tanh_clip(x: f32, drive: f32) -> f32 {
    // 1.0 → 4× gain (~12 dB) — strong but not solid-wall clipping. Beyond
    // that the harmonic series saturates and the timbre stops changing.
    let g = 1.0 + drive * 3.0;
    (x * g).tanh()
}

/// Asymmetric "diode" soft-clip. Positive half compresses faster than
/// negative half — produces strong even harmonics (notably 2nd) which
/// matches the harmonic signature `tools/909-fit.py` extracts from a real
/// 909 kick (`r2 ≈ 14× r3` on `bd01.wav`). Implemented as `tanh` on each
/// half-wave with different drive multipliers so positive and negative
/// excursions saturate at different rates — avoids the unbounded
/// `1 - exp(-x)` form which can drift outside (-1, 1) under transients.
#[inline]
fn diode_clip(x: f32, drive: f32) -> f32 {
    let g = 1.0 + drive * 3.0;
    if x >= 0.0 {
        (x * g).tanh()
    } else {
        // Negative half compresses LESS aggressively (factor 0.6 on the
        // effective gain) so the waveform is biased asymmetric → 2nd
        // harmonic dominates.
        (x * g * 0.6).tanh()
    }
}

/// Cubic soft-clip: `x - x³/3` with input pre-gain, hard-clipped beyond
/// the inflection where the cubic starts to fold. Cheap (one multiply
/// + one cube + one subtract) and a good harmonic reference vs. tanh —
/// 3rd harmonic only, no even content. The clamp gates the input to the
/// region where the cubic is monotonic (`|xg| ≤ 1`); past that the curve
/// would fold back and produce aliased garbage.
#[inline]
fn cubic_clip(x: f32, drive: f32) -> f32 {
    let g = 1.0 + drive * 3.0;
    let xg = (x * g).clamp(-1.0, 1.0);
    xg - (xg * xg * xg) / 3.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_mode_is_identity() {
        for &x in &[-1.0, -0.5, 0.0, 0.3, 0.9] {
            assert_eq!(apply(VC_OFF, 0.5, x), x);
        }
    }

    #[test]
    fn zero_drive_is_identity_for_every_mode() {
        for &mode in &[VC_OFF, VC_TANH, VC_DIODE, VC_CUBIC] {
            for &x in &[-1.0, -0.5, 0.0, 0.3, 0.9] {
                assert_eq!(
                    apply(mode, 0.0, x),
                    x,
                    "mode {} not identity at drive=0 for x={}",
                    mode,
                    x
                );
            }
        }
    }

    #[test]
    fn tanh_clip_is_bounded() {
        // tanh stays inside (-1, 1); after the 1/tanh(g) normalization the
        // output reaches ±1 only at infinity, so it must stay strictly
        // inside that range for any finite input.
        for &drive in &[0.2, 0.5, 1.0] {
            for &x in &[-10.0, -2.0, 2.0, 10.0] {
                let y = tanh_clip(x, drive);
                assert!(y.abs() < 1.001, "tanh out of bounds: drive={drive} x={x} y={y}");
            }
        }
    }

    #[test]
    fn diode_clip_is_asymmetric() {
        // Same |x|, opposite sign should NOT produce symmetric outputs —
        // that's what makes it even-rich. Compare at moderate drive.
        let pos = diode_clip(0.5, 0.6);
        let neg = diode_clip(-0.5, 0.6);
        assert!(
            (pos.abs() - neg.abs()).abs() > 1e-3,
            "diode clip should be asymmetric: pos={pos} neg={neg}"
        );
    }

    #[test]
    fn cubic_clip_is_monotonic_in_input() {
        // Across the audio range, increasing input must give a non-decreasing
        // output — guards against a sign error in the cubic that would fold
        // the waveform at the wrong point.
        let drive = 0.5;
        let mut prev = cubic_clip(-1.0, drive);
        let n = 200;
        for i in 1..=n {
            let x = -1.0 + 2.0 * (i as f32) / (n as f32);
            let y = cubic_clip(x, drive);
            assert!(
                y >= prev - 1e-5,
                "non-monotonic at x={x}: {y} < prev {prev}"
            );
            prev = y;
        }
    }

    #[test]
    fn apply_clamps_drive_above_one() {
        // Automation overshoot (drive > 1.0) shouldn't blow up the shaper —
        // it should saturate at the drive=1.0 result. Apply a representative
        // value at the boundary and at a clearly-overshot value; the two
        // must match.
        let at_one = apply(VC_TANH, 1.0, 0.5);
        let overshoot = apply(VC_TANH, 5.0, 0.5);
        assert!(
            (at_one - overshoot).abs() < 1e-6,
            "drive>1.0 was not clamped: at_one={at_one} overshoot={overshoot}"
        );
    }
}
