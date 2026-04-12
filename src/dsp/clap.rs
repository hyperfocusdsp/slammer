//! 909-style clap voice.
//!
//! A parallel DSP layer that fires alongside the kick. White noise runs
//! through a 2-pole SVF bandpass, then a baked amplitude envelope shaped
//! like a real analog clap: three fast bursts ~10ms apart, followed by a
//! longer exponential tail.
//!
//! The envelope is pre-computed into `env` on `regenerate` (dirty-checked
//! per buffer against the last freq/tail/sr seen, so normal calls are free).
//! Only one `Vec` is ever allocated, in `new()`, sized for the worst case of
//! 400 ms × 96 kHz. Inside `tick()` nothing allocates.

#[inline]
fn flush_denormal(x: f32) -> f32 {
    if x.is_subnormal() { 0.0 } else { x }
}

pub struct ClapVoice {
    // Trapezoidal SVF bandpass state
    ic1: f32,
    ic2: f32,
    // Cached SVF coefficients (trapezoidal)
    svf_a1: f32,
    svf_a2: f32,
    svf_a3: f32,
    // Pre-computed amplitude envelope, indexed by `pos`.
    env: Vec<f32>,
    env_len: usize,
    pos: usize,
    // Per-trigger white-noise RNG state (xorshift32).
    rng: u32,
    // Cached params for dirty-check
    last_freq: f32,
    last_tail_ms: f32,
    last_sr: f32,
}

impl ClapVoice {
    /// Worst case: 400 ms tail at 96 kHz + 30 ms burst region.
    const MAX_ENV_SAMPLES: usize = (0.43 * 96_000.0) as usize;

    pub fn new(sample_rate: f32) -> Self {
        let mut v = Self {
            ic1: 0.0,
            ic2: 0.0,
            svf_a1: 0.0,
            svf_a2: 0.0,
            svf_a3: 0.0,
            env: vec![0.0; Self::MAX_ENV_SAMPLES],
            env_len: 0,
            pos: Self::MAX_ENV_SAMPLES, // inactive until triggered
            rng: 0xC1AB_C1AB,
            last_freq: 0.0,
            last_tail_ms: 0.0,
            last_sr: 0.0,
        };
        v.regenerate(sample_rate, 1200.0, 180.0);
        v.pos = v.env_len; // still inactive after init
        v
    }

    /// Dirty-checked param update. Safe to call every buffer — only does
    /// real work when freq, tail, or sample rate actually changed.
    pub fn set_params(&mut self, sample_rate: f32, freq: f32, tail_ms: f32) {
        if (freq - self.last_freq).abs() < 0.01
            && (tail_ms - self.last_tail_ms).abs() < 0.01
            && (sample_rate - self.last_sr).abs() < 0.01
        {
            return;
        }
        self.regenerate(sample_rate, freq, tail_ms);
    }

    fn regenerate(&mut self, sample_rate: f32, freq: f32, tail_ms: f32) {
        // Trapezoidal SVF bandpass coeffs (Cytomic/Simper — stable at all
        // frequencies). Q ~= 1.8 for a narrow-ish tonal center.
        let f0 = (freq / sample_rate).clamp(1e-4, 0.49);
        let q: f32 = 1.8;
        let k = 1.0 / q;
        let g = (std::f32::consts::PI * f0).tan();
        self.svf_a1 = 1.0 / (1.0 + g * (g + k));
        self.svf_a2 = g * self.svf_a1;
        self.svf_a3 = g * self.svf_a2;

        // Envelope timeline (in samples)
        let burst_gap_ms = 10.0;
        let burst_count = 3;
        let burst_window_ms = 5.0;
        let pre_tail_ms = burst_gap_ms * (burst_count as f32); // 30ms
        let total_ms = pre_tail_ms + tail_ms.max(1.0);

        let env_len = ((total_ms * 0.001) * sample_rate) as usize;
        let env_len = env_len.min(Self::MAX_ENV_SAMPLES);
        self.env_len = env_len;

        let burst_win_samples = ((burst_window_ms * 0.001) * sample_rate).max(1.0) as usize;
        let burst_half = burst_win_samples / 2;
        let gap_samples = ((burst_gap_ms * 0.001) * sample_rate) as usize;
        let tail_start_sample = pre_tail_ms * 0.001 * sample_rate;
        let tail_samples = (tail_ms * 0.001 * sample_rate).max(1.0);

        for i in 0..env_len {
            // --- three burst tents at t = 0, gap, 2*gap ---
            let mut burst_env = 0.0f32;
            for b in 0..burst_count {
                let center = b * gap_samples;
                let d = (i as i32 - center as i32).unsigned_abs() as usize;
                if d < burst_half {
                    let e = 1.0 - (d as f32 / burst_half as f32);
                    if e > burst_env {
                        burst_env = e;
                    }
                }
            }

            // --- exponential tail, starts at tail_start_sample ---
            let tail_env = if (i as f32) >= tail_start_sample {
                let t = (i as f32 - tail_start_sample) / tail_samples;
                // exp decay: -6 time-constants over the tail length
                0.7 * (-6.0 * t).exp()
            } else {
                0.0
            };

            // Bursts ride above the tail.
            let v = if burst_env > tail_env { burst_env } else { tail_env };
            self.env[i] = v;
        }

        self.last_freq = freq;
        self.last_tail_ms = tail_ms;
        self.last_sr = sample_rate;
    }

    pub fn trigger(&mut self) {
        self.pos = 0;
        self.ic1 = 0.0;
        self.ic2 = 0.0;
        // Fresh RNG seed per trigger keeps each clap slightly different
        // without sounding random — xorshift32 is deterministic from seed.
        self.rng = self.rng.wrapping_add(0x9E37_79B9);
        if self.rng == 0 {
            self.rng = 0xC1AB_C1AB;
        }
    }

    #[inline]
    fn noise_sample(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x as f32 / 2_147_483_648.0) - 1.0
    }

    pub fn tick(&mut self) -> f32 {
        if self.pos >= self.env_len {
            return 0.0;
        }
        let n = self.noise_sample();
        // Trapezoidal SVF bandpass: one sample step
        let v3 = n - self.ic2;
        let v1 = self.svf_a1 * self.ic1 + self.svf_a2 * v3;
        let v2 = self.ic2 + self.svf_a2 * self.ic1 + self.svf_a3 * v3;
        self.ic1 = flush_denormal(2.0 * v1 - self.ic1);
        self.ic2 = flush_denormal(2.0 * v2 - self.ic2);
        let out = v1 * self.env[self.pos]; // v1 = bandpass
        self.pos += 1;
        out
    }

    pub fn is_active(&self) -> bool {
        self.pos < self.env_len
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 44100.0;

    fn collect_all(c: &mut ClapVoice) -> Vec<f32> {
        let mut out = Vec::with_capacity(c.env_len + 100);
        c.trigger();
        while c.is_active() {
            out.push(c.tick());
        }
        out
    }

    #[test]
    fn clap_trigger_produces_nonzero() {
        let mut c = ClapVoice::new(SR);
        let samples = collect_all(&mut c);
        let peak = samples.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!(peak > 0.01, "expected audible clap, got peak {}", peak);
    }

    #[test]
    fn clap_decays_to_silence() {
        let mut c = ClapVoice::new(SR);
        c.trigger();
        // Advance past env_len
        let env_len = c.env_len;
        let mut last = 0.0;
        for _ in 0..(env_len + 64) {
            last = c.tick();
        }
        assert!(!c.is_active());
        assert_eq!(last, 0.0);
    }

    #[test]
    fn clap_has_three_burst_peaks() {
        // Envelope should hit ~1.0 at the three burst centers and dip
        // noticeably between them. Burst centers are at t=0, 10ms, 20ms.
        let mut c = ClapVoice::new(SR);
        c.set_params(SR, 1200.0, 180.0);
        let samples_per_ms = (SR / 1000.0) as usize;
        let centers = [0, 10 * samples_per_ms, 20 * samples_per_ms];
        for &ci in &centers {
            assert!(
                c.env[ci] > 0.9,
                "expected peak ~1.0 at center sample {}, got {}",
                ci,
                c.env[ci]
            );
        }
        // Midpoints between bursts should be lower than the peaks.
        let mid1 = 5 * samples_per_ms;
        let mid2 = 15 * samples_per_ms;
        assert!(
            c.env[mid1] < 0.5,
            "expected dip between burst 1 and 2, got {}",
            c.env[mid1]
        );
        assert!(
            c.env[mid2] < 0.5,
            "expected dip between burst 2 and 3, got {}",
            c.env[mid2]
        );
    }

    #[test]
    fn clap_fundamental_at_center_freq() {
        // Goertzel at the clap center should dominate a sideband 2 octaves up.
        let mut c = ClapVoice::new(SR);
        c.set_params(SR, 1200.0, 180.0);
        let samples = collect_all(&mut c);
        let fundamental = fundamental_power(&samples, SR, 1200.0);
        let sideband = fundamental_power(&samples, SR, 4800.0);
        assert!(
            fundamental > 4.0 * sideband,
            "expected bandpass: fund={} sideband={}",
            fundamental,
            sideband
        );
    }

    #[test]
    fn clap_regenerate_dirty_check_noops() {
        let mut c = ClapVoice::new(SR);
        c.set_params(SR, 1200.0, 180.0);
        let baseline_sr = c.last_sr;
        // Calling with identical args must not touch last_sr field path.
        c.set_params(SR, 1200.0, 180.0);
        assert_eq!(c.last_sr, baseline_sr);
        assert!((c.last_freq - 1200.0).abs() < 1e-3);
    }

    #[test]
    fn clap_tail_length_matches_param() {
        let mut c = ClapVoice::new(SR);
        c.set_params(SR, 1200.0, 100.0);
        let short_len = c.env_len;
        c.set_params(SR, 1200.0, 300.0);
        let long_len = c.env_len;
        // long should be strictly longer; roughly (30 + 300) / (30 + 100) ≈ 2.5x
        assert!(long_len > short_len);
        let ratio = long_len as f32 / short_len as f32;
        assert!(
            (2.2..=2.8).contains(&ratio),
            "tail scaling ratio out of range: {}",
            ratio
        );
    }

    /// Single-bin Goertzel correlation. Returns power at `bin_freq`,
    /// ignoring all other frequencies. Used to verify bandpass character.
    fn fundamental_power(samples: &[f32], sr: f32, bin_freq: f32) -> f32 {
        let w = std::f32::consts::TAU * bin_freq / sr;
        let (mut re, mut im) = (0.0f32, 0.0f32);
        for (i, &x) in samples.iter().enumerate() {
            let p = w * i as f32;
            re += x * p.cos();
            im += x * p.sin();
        }
        re * re + im * im
    }
}
