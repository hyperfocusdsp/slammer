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
}

impl MasterEq {
    pub fn new() -> Self {
        Self {
            tilt_low: BiquadFilter::new(),
            tilt_high: BiquadFilter::new(),
            low_boost: BiquadFilter::new(),
            notch: BiquadFilter::new(),
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
    pub fn update(&mut self, sample_rate: f32, params: &EqParams) {
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
}
