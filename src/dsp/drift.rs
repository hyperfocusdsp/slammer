//! Per-trigger analog pitch drift: small random tuning variation on each
//! note-on, simulating analog VCO tolerance / temperature drift.
//!
//! Phase is deliberately NOT randomized — an analog VCO kick is gated by the
//! trigger edge, so sub / mid oscillators reset to a deterministic starting
//! phase every hit. Randomizing phase here caused audibly inconsistent low
//! end between identical MIDI hits.

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
}
