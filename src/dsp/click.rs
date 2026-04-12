/// Click transient generator.
///
/// A short bandpass-filtered noise burst (~6ms) that provides the attack
/// transient the ear uses to locate the kick in a mix. Pre-generated at
/// init time into a fixed-size buffer — playback is zero-cost.
///
/// The buffer is sized for the worst-case sample rate / duration we
/// support so `regenerate` never has to allocate on the audio thread.
pub struct ClickGen {
    buffer: [f32; Self::MAX_SAMPLES],
    len: usize,
    pos: usize,
}

impl ClickGen {
    /// Worst-case click length: 50 ms at 96 kHz = 4800 samples.
    /// Clamping at this cap means any host running faster than 96 kHz will
    /// silently shorten the click, which is acceptable for a transient.
    const MAX_SAMPLES: usize = 4800;

    pub fn new(sample_rate: f32) -> Self {
        let mut gen = Self {
            buffer: [0.0; Self::MAX_SAMPLES],
            len: 0,
            pos: 0,
        };
        gen.regenerate(sample_rate, 6.0, 3500.0, 1.5);
        gen
    }

    /// Regenerate the click buffer with new parameters.
    /// - `decay_ms`: click duration in milliseconds
    /// - `center_freq`: bandpass center frequency in Hz
    /// - `bw_oct`: bandwidth in octaves
    pub fn regenerate(&mut self, sample_rate: f32, decay_ms: f32, center_freq: f32, bw_oct: f32) {
        let n = ((decay_ms / 1000.0) * sample_rate) as usize;
        self.len = n.min(Self::MAX_SAMPLES);

        // Generate white noise
        let mut rng: u32 = 0xDEADBEEF;
        for i in 0..self.len {
            rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
            self.buffer[i] = (rng as f32 / u32::MAX as f32) * 2.0 - 1.0;
        }

        // Apply 2-pole bandpass (trapezoidal SVF — stable at all freqs)
        let f0 = center_freq / sample_rate;
        let q = (1.0
            / (2.0
                * (std::f32::consts::LN_2 / 2.0 * bw_oct * (std::f32::consts::TAU * f0)
                    / (std::f32::consts::TAU * f0).sin())
                .sinh()))
        .max(0.5);
        let k = 1.0 / q;
        let g = (std::f32::consts::PI * f0).tan();
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;

        let mut ic1 = 0.0f32;
        let mut ic2 = 0.0f32;
        for i in 0..self.len {
            let v3 = self.buffer[i] - ic2;
            let v1 = a1 * ic1 + a2 * v3;
            let v2 = ic2 + a2 * ic1 + a3 * v3;
            ic1 = 2.0 * v1 - ic1;
            ic2 = 2.0 * v2 - ic2;
            self.buffer[i] = v1; // bandpass output
        }

        // Apply amplitude envelope (linear fade-out)
        for i in 0..self.len {
            let env = 1.0 - (i as f32 / self.len as f32);
            self.buffer[i] *= env;
        }

        // Normalize peak to 1.0
        let peak = self.buffer[..self.len]
            .iter()
            .fold(0.0f32, |m, &s| m.max(s.abs()));
        if peak > 0.0 {
            for s in &mut self.buffer[..self.len] {
                *s /= peak;
            }
        }

        self.pos = self.len; // not playing until triggered
    }

    pub fn trigger(&mut self) {
        self.pos = 0;
    }

    /// Returns one sample of the click, or 0.0 if done.
    pub fn tick(&mut self) -> f32 {
        if self.pos >= self.len {
            return 0.0;
        }
        let s = self.buffer[self.pos];
        self.pos += 1;
        s
    }

    pub fn is_active(&self) -> bool {
        self.pos < self.len
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_has_transient() {
        let mut click = ClickGen::new(44100.0);
        click.trigger();
        let first = click.tick().abs();
        // After normalization, early samples should have significant energy
        assert!(first > 0.01, "expected audible transient, got {}", first);
    }

    #[test]
    fn click_decays_to_zero() {
        let mut click = ClickGen::new(44100.0);
        click.trigger();
        let mut last = 0.0;
        while click.is_active() {
            last = click.tick();
        }
        // Last sample should be near zero due to envelope
        assert!(last.abs() < 0.1, "expected near-zero at end, got {}", last);
    }

    #[test]
    fn not_active_before_trigger() {
        let click = ClickGen::new(44100.0);
        assert!(!click.is_active());
    }
}
