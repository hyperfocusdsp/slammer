/// Exponential pitch envelope: sweeps from f_start DOWN to f_end.
///
/// f(t) = f_end + (f_start - f_end) * (1 - (t / sweep_dur)^curve)
///
/// If f_start < f_end, they are swapped (kick always sweeps down).
pub struct PitchEnvelope {
    f_start: f32,
    f_end: f32,
    sweep_dur: f32,
    curve: f32,
    t: f32,
    dt: f32,
}

impl PitchEnvelope {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            f_start: 150.0,
            f_end: 45.0,
            sweep_dur: 0.06,
            curve: 3.0,
            t: 0.0,
            dt: 1.0 / sample_rate,
        }
    }

    pub fn trigger(&mut self, f_start: f32, f_end: f32, sweep_dur_s: f32, curve: f32) {
        // Always sweep downward
        if f_start >= f_end {
            self.f_start = f_start;
            self.f_end = f_end;
        } else {
            self.f_start = f_end;
            self.f_end = f_start;
        }
        self.sweep_dur = sweep_dur_s.max(0.001);
        self.curve = curve.max(0.1);
        self.t = 0.0;
    }

    pub fn tick(&mut self) -> f32 {
        if self.t >= self.sweep_dur {
            return self.f_end;
        }
        let x = self.t / self.sweep_dur;
        let shape = x.powf(self.curve);
        let freq = self.f_end + (self.f_start - self.f_end) * (1.0 - shape);
        self.t += self.dt;
        freq
    }
}

/// Amplitude envelope with 1ms attack ramp (anti-click) + exponential decay.
///
/// gain(t) = attack_ramp(t) * exp(-(t - attack_time) / tau)
pub struct AmpEnvelope {
    tau: f32,
    t: f32,
    dt: f32,
    attack_samples: usize,
    attack_counter: usize,
    active: bool,
}

impl AmpEnvelope {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            tau: 0.06,
            t: 0.0,
            dt: 1.0 / sample_rate,
            attack_samples: (0.001 * sample_rate) as usize, // 1ms
            attack_counter: 0,
            active: false,
        }
    }

    pub fn trigger(&mut self, decay_ms: f32) {
        let decay_s = decay_ms / 1000.0;
        self.tau = (decay_s / 6.9078).max(0.0001);
        self.t = 0.0;
        self.attack_counter = 0;
        self.active = true;
    }

    pub fn tick(&mut self) -> f32 {
        if !self.active {
            return 0.0;
        }

        // Attack ramp (linear, 1ms)
        let attack_gain = if self.attack_counter < self.attack_samples {
            let g = (self.attack_counter as f32 + 1.0) / self.attack_samples as f32;
            self.attack_counter += 1;
            g
        } else {
            1.0
        };

        let decay_gain = (-self.t / self.tau).exp();
        self.t += self.dt;

        let gain = attack_gain * decay_gain;
        if gain < 0.0001 {
            self.active = false;
            return 0.0;
        }
        gain
    }

    pub fn is_active(&self) -> bool {
        self.active
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pitch_env_at_t0_returns_fstart() {
        let mut env = PitchEnvelope::new(44100.0);
        env.trigger(200.0, 40.0, 0.05, 3.0);
        let freq = env.tick();
        assert!((freq - 200.0).abs() < 0.1, "expected ~200, got {}", freq);
    }

    #[test]
    fn pitch_env_beyond_sweep_clamps_to_fend() {
        let mut env = PitchEnvelope::new(44100.0);
        env.trigger(200.0, 40.0, 0.01, 3.0);
        for _ in 0..1000 {
            env.tick();
        }
        let freq = env.tick();
        assert!((freq - 40.0).abs() < 0.01, "expected 40, got {}", freq);
    }

    #[test]
    fn pitch_env_swaps_if_start_below_end() {
        let mut env = PitchEnvelope::new(44100.0);
        env.trigger(40.0, 200.0, 0.05, 3.0); // backwards!
        let freq = env.tick();
        // Should have swapped: starts at 200 (the higher one)
        assert!(
            (freq - 200.0).abs() < 0.1,
            "expected ~200 (swapped), got {}",
            freq
        );
    }

    #[test]
    fn pitch_env_curve_1_is_linear() {
        let mut env = PitchEnvelope::new(44100.0);
        env.trigger(200.0, 40.0, 0.1, 1.0);
        let half_samples = (0.05 * 44100.0) as usize;
        for _ in 0..half_samples {
            env.tick();
        }
        let freq = env.tick();
        let expected = 40.0 + (200.0 - 40.0) * 0.5;
        assert!(
            (freq - expected).abs() < 2.0,
            "expected ~{}, got {}",
            expected,
            freq
        );
    }

    #[test]
    fn amp_env_has_attack_ramp() {
        let mut env = AmpEnvelope::new(44100.0);
        env.trigger(200.0);
        let first = env.tick();
        // First sample should be near zero (start of 1ms ramp), not 1.0
        assert!(first < 0.1, "expected ramp start near 0, got {}", first);
    }

    #[test]
    fn amp_env_60db_at_decay_ms() {
        let mut env = AmpEnvelope::new(44100.0);
        env.trigger(100.0);
        let samples = (0.1 * 44100.0) as usize;
        let mut gain = 1.0;
        for _ in 0..samples {
            gain = env.tick();
        }
        assert!(gain < 0.002, "expected <-54dB at decay_ms, got {}", gain);
    }

    #[test]
    fn amp_env_monotonic_after_attack() {
        let mut env = AmpEnvelope::new(44100.0);
        env.trigger(200.0);
        // Skip attack phase
        for _ in 0..50 {
            env.tick();
        }
        let mut prev = env.tick();
        for _ in 0..10000 {
            let g = env.tick();
            assert!(g <= prev + f32::EPSILON, "non-monotonic: {} > {}", g, prev);
            prev = g;
        }
    }

    #[test]
    fn amp_env_inactive_returns_zero() {
        let env = AmpEnvelope::new(44100.0);
        assert!(!env.is_active());
    }
}
