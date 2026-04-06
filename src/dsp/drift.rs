//! Per-trigger analog drift: adds subtle random variations to pitch and phase
//! on each note-on, simulating analog component tolerances.
//!
//! Usage: call `jitter()` at trigger time to get (pitch_factor, phase_offset_delta).
//! - `pitch_factor`: multiply the pitch envelope start/end frequencies (e.g. 0.995–1.005)
//! - `phase_delta`: add to the oscillator phase offset (small random radians)

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

    /// Returns (pitch_factor, phase_delta) for this trigger.
    ///
    /// `amount`: 0.0 = no drift, 1.0 = maximum drift
    /// - pitch_factor: 1.0 ± up to 0.8% (±14 cents at max)
    /// - phase_delta: ± up to 0.15 radians (~8.6°) at max
    pub fn jitter(&mut self, amount: f32) -> (f32, f32) {
        let pitch_factor = 1.0 + self.rand_bipolar() * amount * 0.008;
        let phase_delta = self.rand_bipolar() * amount * 0.15;
        (pitch_factor, phase_delta)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_drift_is_unity() {
        let mut d = Drift::new();
        let (pf, pd) = d.jitter(0.0);
        assert!((pf - 1.0).abs() < f32::EPSILON);
        assert!(pd.abs() < f32::EPSILON);
    }

    #[test]
    fn max_drift_bounded() {
        let mut d = Drift::new();
        for _ in 0..1000 {
            let (pf, pd) = d.jitter(1.0);
            assert!(pf > 0.99 && pf < 1.01, "pitch factor out of range: {}", pf);
            assert!(pd.abs() < 0.2, "phase delta out of range: {}", pd);
        }
    }

    #[test]
    fn drift_varies_between_triggers() {
        let mut d = Drift::new();
        let (pf1, _) = d.jitter(1.0);
        let (pf2, _) = d.jitter(1.0);
        assert!(
            (pf1 - pf2).abs() > 1e-6,
            "consecutive triggers should differ"
        );
    }
}
