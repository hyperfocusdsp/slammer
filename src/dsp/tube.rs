//! Master-bus tube warmth: asymmetric cubic waveshaper with even-harmonic bias.
//!
//! Engages automatically when the master volume knob is pushed past 0 dB.
//! `amount` ramps 0 → 1 as the knob moves from 0 dB to +6 dB of gain, giving
//! a gentle-to-blooming analog character without a separate user control.
//!
//! Character is asymmetric cubic with a DC bias term:
//!
//! ```text
//!   x' = (x + bias·amount).clamp(-3, 3)      // input bias for 2nd harm
//!   y  = x' - (x'^3)/3                        // cubic soft shaper
//!   y  = y - (bias·amount - (bias·amount)^3/3) // remove DC
//!   y  = y * makeup(amount)                   // gentle auto-makeup
//! ```
//!
//! The bias creates a 2nd-harmonic-dominant signature that's sonically
//! distinct from every other distortion stage in the plugin (tanh/clip/
//! transformer/tape), so stacking master warmth onto a saturated kick adds
//! *new* harmonic content rather than piling more of the same.
//!
//! Memoryless and branch-light — safe under `assert_process_allocs`.

/// Maximum input level the cubic is allowed to see. Beyond this the curve
/// bends the wrong way (the derivative of `x - x³/3` becomes negative at
/// |x| > 1 and the polynomial diverges at |x| > √3), so we clamp before
/// shaping. Chosen well above the expected post-master-bus signal range.
const CUBIC_CLAMP: f32 = 1.6;

/// Positive DC bias applied pre-shaper. Small — we want an even-harmonic
/// tilt, not an audible DC offset or asymmetric clipping. 0.1 yields a
/// musically noticeable 2nd-harmonic component without pulling the zero
/// crossing visibly.
const TUBE_BIAS: f32 = 0.10;

pub struct TubeWarmth;

impl TubeWarmth {
    pub fn new() -> Self {
        Self
    }

    /// Process one stereo sample. `amount` is 0..=1, typically derived from
    /// how far past 0 dB the master knob has been pushed. Below ~0.0005 the
    /// stage is bypassed bit-identically so the default operating point
    /// (knob at 0 dB) is transparent.
    #[inline]
    pub fn process_sample(&mut self, l: f32, r: f32, amount: f32) -> (f32, f32) {
        if amount <= 0.0005 {
            return (l, r);
        }
        let a = amount.clamp(0.0, 1.0);
        let bias = TUBE_BIAS * a;

        // DC compensation: the same constant the shaper would emit for a
        // DC input of `bias`. Subtracting it keeps the output centered so
        // the low end doesn't get pushed out of the DAC.
        let dc = bias - (bias * bias * bias) / 3.0;

        // Gentle auto-makeup: the cubic loses ~3 dB at full drive on a unity
        // sine; compensate linearly with `a`.
        let makeup = 1.0 + 0.35 * a;

        let lo = tube_shape(l, bias, dc) * makeup;
        let ro = tube_shape(r, bias, dc) * makeup;
        (lo, ro)
    }
}

#[inline]
fn tube_shape(x: f32, bias: f32, dc: f32) -> f32 {
    let xb = (x + bias).clamp(-CUBIC_CLAMP, CUBIC_CLAMP);
    let shaped = xb - (xb * xb * xb) / 3.0;
    shaped - dc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_amount_is_passthrough() {
        let mut tube = TubeWarmth::new();
        let (l, r) = tube.process_sample(0.42, -0.17, 0.0);
        assert_eq!(l, 0.42);
        assert_eq!(r, -0.17);
    }

    #[test]
    fn dc_is_preserved() {
        // A constant DC input should not produce a constant DC output —
        // i.e. the bias compensation should null out the offset.
        let mut tube = TubeWarmth::new();
        let (l, _) = tube.process_sample(0.0, 0.0, 1.0);
        assert!(
            l.abs() < 1e-5,
            "tube should not leak DC at zero-input, got {l}"
        );
    }

    #[test]
    fn full_amount_injects_even_harmonics() {
        // Feed a pure unit sine; an asymmetric shaper should produce output
        // whose positive-excursion peak differs from the negative one (the
        // fingerprint of even-harmonic content). A symmetric shaper would
        // give |pos| == |neg|.
        let mut tube = TubeWarmth::new();
        let mut max_pos = 0.0f32;
        let mut max_neg = 0.0f32;
        for i in 0..256 {
            let x = (i as f32 / 256.0 * std::f32::consts::TAU).sin() * 0.8;
            let (y, _) = tube.process_sample(x, x, 1.0);
            if y > max_pos {
                max_pos = y;
            }
            if y < max_neg {
                max_neg = y;
            }
        }
        let asymmetry = (max_pos + max_neg).abs();
        assert!(
            asymmetry > 0.01,
            "expected asymmetric response (even harmonics), got asym={asymmetry} \
             (pos={max_pos}, neg={max_neg})"
        );
    }

    #[test]
    fn output_bounded_on_extreme_input() {
        let mut tube = TubeWarmth::new();
        for &x in &[-10.0f32, -2.0, 2.0, 10.0, 100.0, -100.0] {
            let (y, _) = tube.process_sample(x, x, 1.0);
            assert!(y.is_finite(), "non-finite output for {x}: {y}");
            assert!(y.abs() < 4.0, "output too large for {x}: {y}");
        }
    }

    #[test]
    fn small_amount_is_near_passthrough() {
        let mut tube = TubeWarmth::new();
        let (l, r) = tube.process_sample(0.3, -0.3, 0.0001);
        assert!((l - 0.3).abs() < 1e-6);
        assert!((r + 0.3).abs() < 1e-6);
    }
}
