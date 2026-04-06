use std::f32::consts::TAU;

/// Phase-accumulating sine oscillator.
///
/// At trigger time, phase is set to `phase_offset` (default π/2 = cosine start
/// for maximum punch — first sample at peak amplitude).
pub struct SineOsc {
    phase: f32,
    sample_rate: f32,
}

impl SineOsc {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            phase: 0.0,
            sample_rate,
        }
    }

    /// Reset phase to the given offset. π/2 = cosine start = max amplitude.
    pub fn trigger(&mut self, phase_offset: f32) {
        self.phase = phase_offset;
    }

    /// Generate one sample at the given frequency (Hz).
    pub fn tick(&mut self, freq: f32) -> f32 {
        let out = self.phase.sin();
        self.phase += freq / self.sample_rate * TAU;
        // Keep phase in [0, TAU) to avoid precision loss over time
        if self.phase >= TAU {
            self.phase -= TAU;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_start_gives_peak() {
        let mut osc = SineOsc::new(44100.0);
        osc.trigger(std::f32::consts::FRAC_PI_2);
        let sample = osc.tick(100.0);
        assert!(
            (sample - 1.0).abs() < 0.001,
            "expected ~1.0, got {}",
            sample
        );
    }

    #[test]
    fn phase_continuous_across_buffers() {
        let mut osc = SineOsc::new(44100.0);
        osc.trigger(0.0);
        let mut prev = osc.tick(440.0);
        for _ in 0..1000 {
            let curr = osc.tick(440.0);
            // At 440Hz / 44100Hz, phase increment is small, so consecutive
            // samples should differ by a bounded amount
            let diff = (curr - prev).abs();
            assert!(
                diff < 0.1,
                "discontinuity: {} -> {} (diff {})",
                prev,
                curr,
                diff
            );
            prev = curr;
        }
    }

    #[test]
    fn output_bounded() {
        let mut osc = SineOsc::new(44100.0);
        osc.trigger(0.0);
        for _ in 0..10000 {
            let s = osc.tick(440.0);
            assert!((-1.0..=1.0).contains(&s), "out of range: {}", s);
        }
    }
}
