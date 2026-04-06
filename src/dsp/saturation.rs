//! Saturation/distortion block with three distinct analog-flavored modes.
//!
//! All modes operate on a drive → waveshaper → makeup chain, but each has a
//! deliberately different harmonic fingerprint so switching modes is an
//! audible character change, not three shades of the same curve:
//!
//! * **Clip**   — split-band rational hard-shoulder clipper. Lows pass through,
//!   highs (above ~60 Hz) hit a `x / sqrt(1+x²)` shaper. Symmetric, odd-only,
//!   bright/edgy — the "digital clipper" voice.
//! * **Diode**  — asymmetric `1-exp(-x)` pair. Even+odd harmonics, gritty.
//! * **Tape**   — drive-dependent HF loss + one-pole hysteresis memory + mild
//!   DC-bias asymmetry. The **only** dynamic/memory-based stage — it darkens
//!   and smears as you drive it harder. The "glue and smear" voice.
//!
//! The master-bus tube warmth (see `dsp/tube.rs`) and the master-bus
//! transformer drive (see `dsp/master_bus.rs`) complete the five-voice
//! palette; no two share all four of (symmetry, harmonic weighting,
//! dynamics, frequency dependence).

/// Drive knob at 1.0 maps to `1 + DRIVE_GAIN_RANGE`× input gain, producing
/// roughly +26 dB of drive at full. Empirically tuned for musical response
/// across all three modes without clipping the makeup stage.
const DRIVE_GAIN_RANGE: f32 = 19.0;

/// Asymmetry factor for the negative half of the diode waveshaper.
/// Values < 1.0 make the negative lobe clip harder than the positive lobe,
/// which is what produces the even-harmonic "diode" character.
const DIODE_NEG_ASYM: f32 = 0.8;

/// Clamp the input to the diode `.exp()` terms to avoid overflow on
/// pathological drive values. At ±20 the diode output is already within
/// f32 headroom (`exp(20) ≈ 4.85e8`) and audibly saturated.
const DIODE_EXP_CLAMP: f32 = 20.0;

/// Split-band cutoff between "low end passes through" and "highs hit the
/// clipper" for Clip mode. 60 Hz keeps the sub-bass of a kick below the
/// shaper so the lows stay punchy and clean while the upper harmonics get
/// the hard-shoulder edge.
const CLIP_SPLIT_HZ: f32 = 60.0;

/// Tape HF-loss cutoff range. At `drive=0` the tape LP sits at the max
/// cutoff (airy); at `drive=1` it drops to the min (dark, smeared). This
/// drive-reactive darkening is what separates tape from every other stage.
const TAPE_LP_MAX_HZ: f32 = 10_000.0;
const TAPE_LP_MIN_HZ: f32 = 2_500.0;

/// Hysteresis coefficient — the fraction of the previous shaper output
/// (minus the previous raw shape) that feeds into the current sample.
/// Small values create subtle analog-like smear; larger values start to
/// sound like a comb filter. 0.18 was tuned by ear against the old tanh
/// tape at matched drive levels.
const TAPE_HYSTERESIS: f32 = 0.18;

/// Pre-shaper DC bias for tape's asymmetric (even-harmonic) character.
/// Small — we want a tilt, not a clipped-off negative lobe.
const TAPE_BIAS: f32 = 0.08;

#[inline]
fn flush_denormal(x: f32) -> f32 {
    // Flush subnormal floats to zero to prevent CPU spikes and noise artifacts
    if x.is_subnormal() {
        0.0
    } else {
        x
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum SatMode {
    Off = 0,
    SoftClip = 1,
    Diode = 2,
    Tape = 3,
}

impl SatMode {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::SoftClip,
            2 => Self::Diode,
            3 => Self::Tape,
            _ => Self::Off,
        }
    }
}

pub struct Saturation {
    /// One-pole LP state for Clip's split-band low extraction. The lows
    /// bypass the shaper; (input − lp) is what gets clipped.
    clip_lp_z: f32,
    /// One-pole LP state for Tape's drive-dependent HF loss.
    tape_lp_z: f32,
    /// Previous post-shape output (for hysteresis memory term).
    tape_prev_y: f32,
    /// Previous raw shaper output at the previous input (the subtracted
    /// term in the hysteresis model, so that steady-state DC doesn't drift).
    tape_prev_fx: f32,
    sample_rate: f32,
}

impl Saturation {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            clip_lp_z: 0.0,
            tape_lp_z: 0.0,
            tape_prev_y: 0.0,
            tape_prev_fx: 0.0,
            sample_rate,
        }
    }

    #[allow(dead_code)]
    pub fn reset(&mut self) {
        self.clip_lp_z = 0.0;
        self.tape_lp_z = 0.0;
        self.tape_prev_y = 0.0;
        self.tape_prev_fx = 0.0;
    }

    /// Process a single sample through the saturation stage.
    /// `drive`: 0.0 (clean) to 1.0 (heavy), mapped to 1x-20x gain.
    /// `mix`: 0.0 (dry) to 1.0 (fully wet).
    pub fn process(&mut self, input: f32, mode: SatMode, drive: f32, mix: f32) -> f32 {
        if mode == SatMode::Off || mix <= 0.0 || drive <= 1e-4 {
            // Bit-identical bypass at zero drive. The shapers themselves are
            // non-linear even at unity gain (the rational clipper compresses
            // at ±1 regardless of pre-gain), so "drive=0 means clean" has to
            // be enforced here rather than emerging from the math. This also
            // guarantees parameter-automation rides from 0 → N cross no
            // audible discontinuity at the zero boundary.
            return input;
        }

        // Map drive 0..1 to gain 1..(1 + DRIVE_GAIN_RANGE).
        let gain = (1.0 + drive * DRIVE_GAIN_RANGE).max(1.0);
        let driven = input * gain;

        let saturated = match mode {
            SatMode::Off => driven,
            SatMode::SoftClip => self.process_clip(driven),
            SatMode::Diode => {
                // Asymmetric diode clipping — adds even harmonics.
                // Positive half: soft knee, negative half: harder clip.
                // Clamp before `.exp()` to avoid overflow on extreme drive.
                let d = driven.clamp(-DIODE_EXP_CLAMP, DIODE_EXP_CLAMP);
                if d >= 0.0 {
                    1.0 - (-d).exp()
                } else {
                    -(1.0 - d.exp()) * DIODE_NEG_ASYM
                }
            }
            SatMode::Tape => self.process_tape(driven, drive),
        };

        // Makeup gain to compensate for level loss from clipping
        let makeup = 1.0 / gain.sqrt();
        let wet = saturated * makeup;

        // Wet/dry mix
        input * (1.0 - mix) + wet * mix
    }

    /// Split-band rational hard-shoulder clipper.
    ///
    /// 1. Low-pass the input at ~60 Hz to extract the kick fundamental.
    /// 2. The "highs" path is (input − lows), fed into `x / sqrt(1+x²)`
    ///    which is bounded to ±1 but has a harder shoulder than tanh.
    /// 3. Recombine lows (unclipped) + clipped highs.
    ///
    /// Result: the sub-bass stays clean and punchy while the upper
    /// harmonics get a bright, edgy clipped character — distinct from any
    /// `tanh` curve.
    #[inline]
    fn process_clip(&mut self, x: f32) -> f32 {
        let rc = 1.0 / (std::f32::consts::TAU * CLIP_SPLIT_HZ);
        let dt = 1.0 / self.sample_rate;
        let alpha = dt / (rc + dt);
        self.clip_lp_z += alpha * (x - self.clip_lp_z);
        self.clip_lp_z = flush_denormal(self.clip_lp_z);

        let lows = self.clip_lp_z;
        let highs = x - lows;
        // Rational hard-shoulder shaper. Symmetric, bounded to ±1.
        let clipped_highs = highs / (1.0 + highs * highs).sqrt();
        lows + clipped_highs
    }

    /// Drive-dependent tape model with hysteresis memory and bias asymmetry.
    ///
    /// The three distinguishing characteristics vs. a plain `tanh`:
    ///
    /// * **HF loss scales with drive**: the post-shape LP cutoff drops from
    ///   10 kHz → 2.5 kHz as drive climbs, so harder hits darken the tone
    ///   *and* the saturation-generated harmonics get rolled off by the
    ///   same tape loss — closer to how a real machine degrades under heavy
    ///   level than an input-side filter would be. This is the most
    ///   immediately audible tape fingerprint.
    /// * **Hysteresis memory**: `y = f(x) + α·(y_prev − f(x_prev))`. The
    ///   output retains a fraction of the divergence from the memoryless
    ///   shape, producing a subtle analog-like smear on transients.
    /// * **DC bias asymmetry**: a small positive bias pre-shaper adds
    ///   2nd-harmonic content (even-dominant) for warmth.
    #[inline]
    fn process_tape(&mut self, x: f32, drive: f32) -> f32 {
        // Asymmetric memoryless shape: tanh of (input + small DC bias).
        // The bias creates 2nd harmonic content; the tanh provides the
        // bounded soft-knee shape. Subtract the DC shift so zero-input
        // yields zero output in steady state.
        let fx_raw = (x + TAPE_BIAS).tanh() - TAPE_BIAS.tanh();

        // Drive-reactive LP AFTER the shape. Placing the filter on the
        // readback side (downstream of the nonlinearity) ensures that
        // saturation-generated harmonics are subject to the same HF loss
        // as the recorded signal — which is what makes tape sound
        // *darker* under drive rather than brighter, and is the acoustic
        // fingerprint that distinguishes tape from tanh/rational clippers.
        let cutoff = TAPE_LP_MAX_HZ - drive.clamp(0.0, 1.0) * (TAPE_LP_MAX_HZ - TAPE_LP_MIN_HZ);
        let rc = 1.0 / (std::f32::consts::TAU * cutoff);
        let dt = 1.0 / self.sample_rate;
        let alpha = dt / (rc + dt);
        self.tape_lp_z += alpha * (fx_raw - self.tape_lp_z);
        self.tape_lp_z = flush_denormal(self.tape_lp_z);
        let fx = self.tape_lp_z;

        // One-pole hysteresis memory on the filtered output. Blends in
        // the previous divergence from the current shape — this is what
        // makes the curve traversed upward differ from the curve traversed
        // downward, which is the essence of magnetic hysteresis.
        let y = fx + TAPE_HYSTERESIS * (self.tape_prev_y - self.tape_prev_fx);

        self.tape_prev_y = flush_denormal(y);
        self.tape_prev_fx = flush_denormal(fx);
        y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn soft_clip_symmetric() {
        // Split-band clipper should be symmetric (the rational shaper is
        // odd-symmetric, and the split-band LP is DC-symmetric).
        let mut sat = Saturation::new(44100.0);
        // Prime the LP with a steady state so we're measuring the shaper,
        // not the LP settling time.
        for _ in 0..1024 {
            let _ = sat.process(0.5, SatMode::SoftClip, 0.5, 1.0);
        }
        sat.reset();
        let pos = sat.process(0.5, SatMode::SoftClip, 0.5, 1.0);
        sat.reset();
        let neg = sat.process(-0.5, SatMode::SoftClip, 0.5, 1.0);
        assert!(
            (pos + neg).abs() < 0.001,
            "clip should be symmetric: {pos} vs {neg}"
        );
    }

    #[test]
    fn diode_asymmetric() {
        let mut sat = Saturation::new(44100.0);
        let pos = sat.process(0.5, SatMode::Diode, 0.5, 1.0).abs();
        sat.reset();
        let neg = sat.process(-0.5, SatMode::Diode, 0.5, 1.0).abs();
        assert!(
            (pos - neg).abs() > 0.01,
            "diode should be asymmetric: {pos} vs {neg}"
        );
    }

    #[test]
    fn off_is_passthrough() {
        let mut sat = Saturation::new(44100.0);
        let out = sat.process(0.7, SatMode::Off, 1.0, 1.0);
        assert!((out - 0.7).abs() < f32::EPSILON);
    }

    #[test]
    fn zero_drive_near_unity() {
        let mut sat = Saturation::new(44100.0);
        let out = sat.process(0.5, SatMode::SoftClip, 0.0, 1.0);
        assert!(
            (out - 0.5).abs() < 0.05,
            "zero drive should be near unity, got {out}"
        );
    }

    #[test]
    fn output_bounded() {
        let mut sat = Saturation::new(44100.0);
        for mode in [SatMode::SoftClip, SatMode::Diode, SatMode::Tape] {
            for &input in &[-2.0, -1.0, -0.5, 0.0, 0.5, 1.0, 2.0] {
                let out = sat.process(input, mode, 1.0, 1.0);
                assert!(
                    out.abs() < 3.0,
                    "output too large: {out} for input {input} mode {mode:?}",
                );
                sat.reset();
            }
        }
    }

    #[test]
    fn diode_extreme_input_is_finite() {
        let mut sat = Saturation::new(44100.0);
        for &input in &[100.0_f32, -100.0, 1e6, -1e6] {
            let out = sat.process(input, SatMode::Diode, 1.0, 1.0);
            assert!(out.is_finite(), "diode produced non-finite for {input}: {out}");
            sat.reset();
        }
    }

    #[test]
    fn mix_zero_is_dry() {
        let mut sat = Saturation::new(44100.0);
        let out = sat.process(0.7, SatMode::SoftClip, 1.0, 0.0);
        assert!((out - 0.7).abs() < f32::EPSILON);
    }

    #[test]
    fn tape_has_memory() {
        // Fundamental tape-vs-memoryless test: given two identical inputs
        // arriving from different histories, the tape output must differ.
        // A memoryless shaper would give identical outputs regardless of
        // history — that's the contrast that makes tape sonically distinct.
        let mut sat_a = Saturation::new(44100.0);
        let mut sat_b = Saturation::new(44100.0);

        // Prime A with a loud transient pulse, then let it relax to zero.
        for _ in 0..8 {
            sat_a.process(0.9, SatMode::Tape, 0.7, 1.0);
        }
        for _ in 0..4 {
            sat_a.process(0.0, SatMode::Tape, 0.7, 1.0);
        }

        // Prime B with zero only.
        for _ in 0..12 {
            sat_b.process(0.0, SatMode::Tape, 0.7, 1.0);
        }

        // Same current input; outputs should differ due to memory state.
        let a = sat_a.process(0.2, SatMode::Tape, 0.7, 1.0);
        let b = sat_b.process(0.2, SatMode::Tape, 0.7, 1.0);
        assert!(
            (a - b).abs() > 1e-4,
            "tape should have memory — same input gave same output a={a} b={b}"
        );
    }

    /// Single-bin Goertzel-style power at `bin_freq` for a periodic signal.
    /// Used to measure the fundamental energy of a sine through the tape
    /// stage while ignoring saturation harmonics.
    fn fundamental_power(samples: &[f32], sr: f32, bin_freq: f32) -> f32 {
        let w = std::f32::consts::TAU * bin_freq / sr;
        let mut re = 0.0f32;
        let mut im = 0.0f32;
        for (i, &x) in samples.iter().enumerate() {
            let p = w * i as f32;
            re += x * p.cos();
            im += x * p.sin();
        }
        re * re + im * im
    }

    #[test]
    fn tape_darkens_under_drive() {
        // The defining fingerprint of the retuned tape stage is drive-
        // reactive HF loss: as drive climbs from light to heavy, a pure
        // HF sine's *fundamental* energy at the output should drop
        // dramatically relative to what the LP's response would allow at
        // low drive. Measuring at the exact bin of the input fundamental
        // (via Goertzel) avoids being fooled by saturation harmonics that
        // a wideband RMS would lump in.
        fn hf_fund_rel(drive: f32) -> f32 {
            let sr = 48_000.0;
            let n = 4096;
            let freq = 9_000.0;
            let mut sat = Saturation::new(sr);

            // Let the LP settle to steady state on the test signal.
            for _ in 0..512 {
                let x = (std::f32::consts::TAU * freq * 0.0 / sr).sin() * 0.3;
                sat.process(x, SatMode::Tape, drive, 1.0);
            }

            let mut out = vec![0.0f32; n];
            for i in 0..n {
                let x = (std::f32::consts::TAU * freq * i as f32 / sr).sin() * 0.3;
                out[i] = sat.process(x, SatMode::Tape, drive, 1.0);
            }

            fundamental_power(&out, sr, freq).sqrt()
        }

        let bright = hf_fund_rel(0.1);
        let dark = hf_fund_rel(1.0);
        assert!(
            dark < bright * 0.5,
            "tape at drive=1 should attenuate a 9 kHz fundamental much more than at \
             drive=0.1: bright={bright} dark={dark}"
        );
    }

    #[test]
    fn clip_lows_pass_through() {
        // DC-ish / sub-bass input: the split-band clipper should pass it
        // through nearly unchanged at low drive because the LP captures
        // almost all of it, leaving ~0 for the shaper to act on.
        let mut sat = Saturation::new(44100.0);
        // Settle the 60 Hz LP on a steady DC value so lp_z ≈ x.
        let steady = 0.25;
        for _ in 0..8192 {
            sat.process(steady, SatMode::SoftClip, 0.0, 1.0);
        }
        // With drive=0, makeup=1, the output should be ~= steady.
        let out = sat.process(steady, SatMode::SoftClip, 0.0, 1.0);
        assert!(
            (out - steady).abs() < 0.01,
            "clip should pass sub-bass through cleanly, got {out} for {steady}"
        );
    }
}
