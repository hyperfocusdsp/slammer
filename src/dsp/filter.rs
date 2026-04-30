//! Simple biquad filter for EQ stages.
//! Implements tilt EQ, low shelf, and parametric notch using the Audio EQ Cookbook.

#[inline]
fn flush_denormal(x: f32) -> f32 {
    // Flush subnormal floats to zero to prevent CPU spikes and noise artifacts
    if x.is_subnormal() {
        0.0
    } else {
        x
    }
}

pub struct BiquadFilter {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Default for BiquadFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl BiquadFilter {
    pub fn new() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    #[allow(dead_code)]
    pub fn reset(&mut self) {
        self.x1 = 0.0;
        self.x2 = 0.0;
        self.y1 = 0.0;
        self.y2 = 0.0;
    }

    /// Low shelf: boost/cut below `freq` by `gain_db`.
    pub fn set_low_shelf(&mut self, sample_rate: f32, freq: f32, gain_db: f32) {
        if gain_db.abs() < 0.01 {
            self.set_passthrough();
            return;
        }
        let a = 10.0f32.powf(gain_db / 40.0);
        let w0 = std::f32::consts::TAU * freq / sample_rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / 2.0 * ((a + 1.0 / a) * (1.0 / 0.707 - 1.0) + 2.0).sqrt();
        let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;

        let a0 = (a + 1.0) + (a - 1.0) * cos_w0 + two_sqrt_a_alpha;
        self.b0 = (a * ((a + 1.0) - (a - 1.0) * cos_w0 + two_sqrt_a_alpha)) / a0;
        self.b1 = (2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w0)) / a0;
        self.b2 = (a * ((a + 1.0) - (a - 1.0) * cos_w0 - two_sqrt_a_alpha)) / a0;
        self.a1 = (-2.0 * ((a - 1.0) + (a + 1.0) * cos_w0)) / a0;
        self.a2 = ((a + 1.0) + (a - 1.0) * cos_w0 - two_sqrt_a_alpha) / a0;
    }

    /// Parametric notch/bell at `freq` with `q` and `gain_db`.
    pub fn set_peaking(&mut self, sample_rate: f32, freq: f32, q: f32, gain_db: f32) {
        if gain_db.abs() < 0.01 || q < 0.01 {
            self.set_passthrough();
            return;
        }
        let a = 10.0f32.powf(gain_db / 40.0);
        let w0 = std::f32::consts::TAU * freq / sample_rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / (2.0 * q);

        let a0 = 1.0 + alpha / a;
        self.b0 = (1.0 + alpha * a) / a0;
        self.b1 = (-2.0 * cos_w0) / a0;
        self.b2 = (1.0 - alpha * a) / a0;
        self.a1 = (-2.0 * cos_w0) / a0;
        self.a2 = (1.0 - alpha / a) / a0;
    }

    /// High shelf: boost/cut above `freq` by `gain_db`.
    pub fn set_high_shelf(&mut self, sample_rate: f32, freq: f32, gain_db: f32) {
        if gain_db.abs() < 0.01 {
            self.set_passthrough();
            return;
        }
        let a = 10.0f32.powf(gain_db / 40.0);
        let w0 = std::f32::consts::TAU * freq / sample_rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / 2.0 * ((a + 1.0 / a) * (1.0 / 0.707 - 1.0) + 2.0).sqrt();
        let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;

        let a0 = (a + 1.0) - (a - 1.0) * cos_w0 + two_sqrt_a_alpha;
        self.b0 = (a * ((a + 1.0) + (a - 1.0) * cos_w0 + two_sqrt_a_alpha)) / a0;
        self.b1 = (-2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w0)) / a0;
        self.b2 = (a * ((a + 1.0) + (a - 1.0) * cos_w0 - two_sqrt_a_alpha)) / a0;
        self.a1 = (2.0 * ((a - 1.0) - (a + 1.0) * cos_w0)) / a0;
        self.a2 = ((a + 1.0) - (a - 1.0) * cos_w0 - two_sqrt_a_alpha) / a0;
    }

    pub fn set_passthrough(&mut self) {
        self.b0 = 1.0;
        self.b1 = 0.0;
        self.b2 = 0.0;
        self.a1 = 0.0;
        self.a2 = 0.0;
    }

    pub fn process(&mut self, input: f32) -> f32 {
        let output = self.b0 * input + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = input;
        self.y2 = self.y1;
        // Flush subnormal state to prevent denormal CPU spikes and noise artifacts
        self.y1 = flush_denormal(output);
        output
    }
}

/// Three-band master EQ: tilt (low shelf + high shelf), low boost, boxiness notch.
pub struct MasterEq {
    tilt_low: BiquadFilter,
    tilt_high: BiquadFilter,
    low_boost: BiquadFilter,
    notch: BiquadFilter,
    /// Last-applied params snapshot. `update` early-exits when nothing
    /// changed, skipping 4 biquad-coefficient recomputes (each ~6 trig
    /// ops). Initialized to NaN-poisoned values so the first call always
    /// recomputes.
    last_sample_rate: f32,
    last_tilt_db: f32,
    last_low_boost_db: f32,
    last_notch_freq: f32,
    last_notch_q: f32,
    last_notch_depth_db: f32,
}

impl Default for MasterEq {
    fn default() -> Self {
        Self::new()
    }
}

impl MasterEq {
    pub fn new() -> Self {
        Self {
            tilt_low: BiquadFilter::new(),
            tilt_high: BiquadFilter::new(),
            low_boost: BiquadFilter::new(),
            notch: BiquadFilter::new(),
            last_sample_rate: f32::NAN,
            last_tilt_db: f32::NAN,
            last_low_boost_db: f32::NAN,
            last_notch_freq: f32::NAN,
            last_notch_q: f32::NAN,
            last_notch_depth_db: f32::NAN,
        }
    }

    #[allow(dead_code)]
    pub fn reset(&mut self) {
        self.tilt_low.reset();
        self.tilt_high.reset();
        self.low_boost.reset();
        self.notch.reset();
    }

    /// Update EQ coefficients. Call once per buffer (not per-sample).
    /// Skips the 4 biquad recomputes when no input has changed since the
    /// previous call — common case when the user isn't automating EQ.
    pub fn update(&mut self, sample_rate: f32, params: &EqParams) {
        let unchanged = sample_rate == self.last_sample_rate
            && params.tilt_db == self.last_tilt_db
            && params.low_boost_db == self.last_low_boost_db
            && params.notch_freq == self.last_notch_freq
            && params.notch_q == self.last_notch_q
            && params.notch_depth_db == self.last_notch_depth_db;
        if unchanged {
            return;
        }

        // Tilt: pivot at 1kHz — low shelf up = high shelf down, and vice versa
        self.tilt_low
            .set_low_shelf(sample_rate, 1000.0, params.tilt_db);
        self.tilt_high
            .set_high_shelf(sample_rate, 1000.0, -params.tilt_db);

        // Low boost shelf at 80Hz
        self.low_boost
            .set_low_shelf(sample_rate, 80.0, params.low_boost_db);

        // Boxiness notch (parametric cut) — Q=0 means bypassed
        if params.notch_q > 0.01 {
            self.notch.set_peaking(
                sample_rate,
                params.notch_freq,
                params.notch_q,
                -params.notch_depth_db.abs(),
            );
        } else {
            self.notch.set_passthrough();
        }

        self.last_sample_rate = sample_rate;
        self.last_tilt_db = params.tilt_db;
        self.last_low_boost_db = params.low_boost_db;
        self.last_notch_freq = params.notch_freq;
        self.last_notch_q = params.notch_q;
        self.last_notch_depth_db = params.notch_depth_db;
    }

    pub fn process(&mut self, input: f32) -> f32 {
        let mut s = input;
        s = self.tilt_low.process(s);
        s = self.tilt_high.process(s);
        s = self.low_boost.process(s);
        s = self.notch.process(s);
        s
    }
}

pub struct EqParams {
    pub tilt_db: f32,        // -6..+6, 0=flat
    pub low_boost_db: f32,   // -3..+9, 0=flat
    pub notch_freq: f32,     // 100..600Hz
    pub notch_q: f32,        // 0..10, 0=bypassed
    pub notch_depth_db: f32, // 0..20dB of cut
}

impl Default for EqParams {
    fn default() -> Self {
        Self {
            tilt_db: 0.0,
            low_boost_db: 0.0,
            notch_freq: 250.0,
            notch_q: 0.0,
            notch_depth_db: 12.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passthrough_is_unity() {
        let mut f = BiquadFilter::new();
        let out = f.process(0.5);
        assert!((out - 0.5).abs() < 0.001);
    }

    #[test]
    fn eq_flat_is_near_unity() {
        let mut eq = MasterEq::new();
        eq.update(44100.0, &EqParams::default());
        // Feed a DC-ish signal
        let mut out = 0.0;
        for _ in 0..1000 {
            out = eq.process(0.5);
        }
        assert!(
            (out - 0.5).abs() < 0.05,
            "flat EQ should be near unity, got {}",
            out
        );
    }

    /// Goertzel single-bin power for spectral assertions on biquad output.
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

    fn render_sine(sr: f32, freq: f32, n: usize) -> Vec<f32> {
        use std::f32::consts::TAU;
        (0..n).map(|i| (TAU * freq * i as f32 / sr).sin()).collect()
    }

    /// Low shelf with positive gain MUST boost low-frequency content vs
    /// high-frequency content. Drive a 60 Hz sine and a 4 kHz sine
    /// through the same shelf and compare on-bin power ratios.
    #[test]
    fn low_shelf_boosts_lows_relative_to_highs() {
        let sr = 48_000.0;
        let mut f_low = BiquadFilter::new();
        let mut f_high = BiquadFilter::new();
        f_low.set_low_shelf(sr, 200.0, 6.0);
        f_high.set_low_shelf(sr, 200.0, 6.0);

        let lows = render_sine(sr, 60.0, 24_000);
        let highs = render_sine(sr, 4000.0, 24_000);
        let lows_out: Vec<f32> = lows.iter().map(|&x| f_low.process(x)).collect();
        let highs_out: Vec<f32> = highs.iter().map(|&x| f_high.process(x)).collect();

        let p_low_in = fundamental_power(&lows, sr, 60.0);
        let p_low_out = fundamental_power(&lows_out, sr, 60.0);
        let p_high_in = fundamental_power(&highs, sr, 4000.0);
        let p_high_out = fundamental_power(&highs_out, sr, 4000.0);

        let low_gain_db = 10.0 * (p_low_out / p_low_in).log10();
        let high_gain_db = 10.0 * (p_high_out / p_high_in).log10();

        // Low frequency should be boosted ~6 dB, highs ~unchanged.
        assert!(
            low_gain_db > 4.0,
            "low_shelf+6dB at 60Hz: got {low_gain_db:.2} dB"
        );
        assert!(
            high_gain_db.abs() < 1.0,
            "low_shelf+6dB at 4kHz: should be ~0 dB, got {high_gain_db:.2}"
        );
    }

    /// Mirror test for the high shelf.
    #[test]
    fn high_shelf_boosts_highs_relative_to_lows() {
        let sr = 48_000.0;
        let mut f_low = BiquadFilter::new();
        let mut f_high = BiquadFilter::new();
        f_low.set_high_shelf(sr, 2000.0, 6.0);
        f_high.set_high_shelf(sr, 2000.0, 6.0);

        let lows = render_sine(sr, 100.0, 24_000);
        let highs = render_sine(sr, 8000.0, 24_000);
        let lows_out: Vec<f32> = lows.iter().map(|&x| f_low.process(x)).collect();
        let highs_out: Vec<f32> = highs.iter().map(|&x| f_high.process(x)).collect();

        let p_low_in = fundamental_power(&lows, sr, 100.0);
        let p_low_out = fundamental_power(&lows_out, sr, 100.0);
        let p_high_in = fundamental_power(&highs, sr, 8000.0);
        let p_high_out = fundamental_power(&highs_out, sr, 8000.0);

        let low_gain_db = 10.0 * (p_low_out / p_low_in).log10();
        let high_gain_db = 10.0 * (p_high_out / p_high_in).log10();

        assert!(
            high_gain_db > 4.0,
            "high_shelf+6dB at 8kHz: got {high_gain_db:.2}"
        );
        assert!(
            low_gain_db.abs() < 1.0,
            "high_shelf+6dB at 100Hz: should be ~0 dB, got {low_gain_db:.2}"
        );
    }

    /// Notch peaking filter at 250 Hz with -12 dB depth must attenuate a
    /// 250 Hz sine far more than a 1 kHz sine.
    #[test]
    fn peaking_notch_attenuates_target_frequency() {
        let sr = 48_000.0;
        let freq = 250.0;
        let mut f_target = BiquadFilter::new();
        let mut f_other = BiquadFilter::new();
        f_target.set_peaking(sr, freq, 4.0, -12.0);
        f_other.set_peaking(sr, freq, 4.0, -12.0);

        let target = render_sine(sr, freq, 24_000);
        let other = render_sine(sr, 1000.0, 24_000);
        let target_out: Vec<f32> = target.iter().map(|&x| f_target.process(x)).collect();
        let other_out: Vec<f32> = other.iter().map(|&x| f_other.process(x)).collect();

        let p_t_in = fundamental_power(&target, sr, freq);
        let p_t_out = fundamental_power(&target_out, sr, freq);
        let p_o_in = fundamental_power(&other, sr, 1000.0);
        let p_o_out = fundamental_power(&other_out, sr, 1000.0);

        let target_db = 10.0 * (p_t_out / p_t_in).log10();
        let other_db = 10.0 * (p_o_out / p_o_in).log10();

        assert!(
            target_db < -6.0,
            "notch should cut 250Hz, got {target_db:.2}"
        );
        assert!(
            other_db.abs() < 1.0,
            "notch off-band 1kHz should be ~0 dB, got {other_db:.2}"
        );
    }

    /// Repeated `update` calls with identical inputs must NOT reset the
    /// biquad state — that would discard the IIR memory and click. The
    /// dirty-check should early-exit and leave state alone.
    #[test]
    fn eq_update_idempotent_no_state_reset() {
        let mut eq_a = MasterEq::new();
        let mut eq_b = MasterEq::new();
        let params = EqParams {
            tilt_db: 1.5,
            low_boost_db: 2.0,
            notch_freq: 250.0,
            notch_q: 1.0,
            notch_depth_db: 6.0,
        };

        // Warm both with some history.
        eq_a.update(48000.0, &params);
        eq_b.update(48000.0, &params);
        for _ in 0..512 {
            let _ = eq_a.process(0.3);
            let _ = eq_b.process(0.3);
        }

        // A: keep updating with identical params (dirty-check should skip).
        // B: leave alone.
        for _ in 0..1000 {
            eq_a.update(48000.0, &params);
        }

        // Both must produce the same next sample; if `update` reset state
        // each call, A's IIR memory would be cleared and the outputs
        // would diverge.
        let a_next = eq_a.process(0.4);
        let b_next = eq_b.process(0.4);
        assert!(
            (a_next - b_next).abs() < 1e-6,
            "idempotent update reset biquad state: a={a_next}, b={b_next}"
        );
    }
}
