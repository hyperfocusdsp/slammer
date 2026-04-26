//! Per-trigger analog drift: small random variation on each note-on,
//! simulating analog VCO tolerance / temperature drift and envelope-chip
//! tolerance.
//!
//! Phase is deliberately NOT randomized — an analog VCO kick is gated by the
//! trigger edge, so sub / mid oscillators reset to a deterministic starting
//! phase every hit. Randomizing phase here caused audibly inconsistent low
//! end between identical MIDI hits.
//!
//! Three independent jitter axes, all gated by the same `drift_amount` knob:
//! - `pitch_jitter` ±0.8%  (≈±14 cents) — VCO tuning tolerance, sampled
//!   per layer (sub and mid get independent values for slight detune).
//! - `amp_jitter`   ±2.5%  — VCA / level-trimmer tolerance, sampled once
//!   per trigger and applied uniformly across layers.
//! - `decay_jitter` ±5%    — envelope-chip RC tolerance, sampled once per
//!   trigger and applied to all amp-envelope time constants.

/// One trigger's worth of drift values. Sample once per `trigger()` call.
/// All three fields default to `1.0` (unity, no drift) when `drift_amount`
/// is 0.0, so the deterministic case is preserved exactly.
#[derive(Clone, Copy, Debug)]
pub struct DriftSample {
    pub amp_scale: f32,
    pub decay_scale: f32,
}

pub struct Drift {
    /// LCG state — cheap, deterministic, no allocation
    state: u32,
}

impl Drift {
    pub fn new() -> Self {
        Self { state: 0xCAFEBABE }
    }

    /// Generate a random f32 in [-1.0, 1.0] and advance state.
    fn rand_bipolar(&mut self) -> f32 {
        self.state = self.state.wrapping_mul(1664525).wrapping_add(1013904223);
        (self.state as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    /// Returns a pitch factor to multiply into the pitch-envelope start/end
    /// frequencies for this trigger.
    ///
    /// `amount`: 0.0 = no drift, 1.0 = maximum drift (±0.8%, ~±14 cents)
    pub fn pitch_jitter(&mut self, amount: f32) -> f32 {
        1.0 + self.rand_bipolar() * amount * 0.008
    }

    /// Per-trigger amplitude + decay drift, sampled together so all three
    /// LCG advances happen in a fixed order regardless of which fields the
    /// caller actually uses (keeps the sequence deterministic across
    /// refactors). At `amount=0.0` both fields are exactly 1.0.
    pub fn sample_envelope(&mut self, amount: f32) -> DriftSample {
        let a = self.rand_bipolar();
        let d = self.rand_bipolar();
        DriftSample {
            amp_scale: 1.0 + a * amount * 0.025,
            decay_scale: 1.0 + d * amount * 0.05,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_drift_is_unity() {
        let mut d = Drift::new();
        let pf = d.pitch_jitter(0.0);
        assert!((pf - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn max_drift_bounded() {
        let mut d = Drift::new();
        for _ in 0..1000 {
            let pf = d.pitch_jitter(1.0);
            assert!(pf > 0.99 && pf < 1.01, "pitch factor out of range: {}", pf);
        }
    }

    #[test]
    fn drift_varies_between_triggers() {
        let mut d = Drift::new();
        let pf1 = d.pitch_jitter(1.0);
        let pf2 = d.pitch_jitter(1.0);
        assert!(
            (pf1 - pf2).abs() > 1e-6,
            "consecutive triggers should differ"
        );
    }

    #[test]
    fn envelope_drift_is_unity_at_zero() {
        let mut d = Drift::new();
        let s = d.sample_envelope(0.0);
        assert!((s.amp_scale - 1.0).abs() < f32::EPSILON);
        assert!((s.decay_scale - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn envelope_drift_bounds() {
        let mut d = Drift::new();
        for _ in 0..1000 {
            let s = d.sample_envelope(1.0);
            assert!(
                s.amp_scale > 0.974 && s.amp_scale < 1.026,
                "amp_scale {} outside ±2.5%",
                s.amp_scale
            );
            assert!(
                s.decay_scale > 0.949 && s.decay_scale < 1.051,
                "decay_scale {} outside ±5%",
                s.decay_scale
            );
        }
    }

    #[test]
    fn envelope_drift_varies_between_triggers() {
        let mut d = Drift::new();
        let s1 = d.sample_envelope(1.0);
        let s2 = d.sample_envelope(1.0);
        assert!(
            (s1.amp_scale - s2.amp_scale).abs() > 1e-6
                || (s1.decay_scale - s2.decay_scale).abs() > 1e-6,
            "consecutive triggers should differ"
        );
    }
}
