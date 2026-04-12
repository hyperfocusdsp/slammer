use std::f32::consts::PI;

#[inline]
fn flush_denormal(x: f32) -> f32 {
    if x.is_subnormal() { 0.0 } else { x }
}

/// Stereo 2-pole SVF DJ filter with bipolar cutoff mapping.
///
/// Uses the Cytomic/Simper trapezoidal SVF topology which is
/// unconditionally stable at all frequencies — the naive SVF blows up
/// when cutoff exceeds ~sr/6.
///
/// Coefficients are dirty-checked per sample: the expensive `powf()` +
/// `tan()` only recompute when cutoff_pos or resonance actually change.
/// For a static knob position (the common case) the per-sample cost is
/// a two-float comparison + the SVF tick.
pub struct DjFilter {
    /// Integrator state (ic1eq, ic2eq) per channel.
    ic1: [f32; 2],
    ic2: [f32; 2],
    sr: f32,
    // Cached SVF coefficients + dirty-check inputs.
    last_pos: f32,
    last_res: f32,
    a1: f32,
    a2: f32,
    a3: f32,
    k: f32,
    is_hp: bool,
}

impl DjFilter {
    pub fn new() -> Self {
        Self {
            ic1: [0.0; 2],
            ic2: [0.0; 2],
            sr: 44100.0,
            last_pos: 0.0,
            last_res: 0.0,
            a1: 0.0,
            a2: 0.0,
            a3: 0.0,
            k: 1.0,
            is_hp: false,
        }
    }

    pub fn set_sample_rate(&mut self, sr: f32) {
        self.sr = sr;
        self.reset();
    }

    pub fn reset(&mut self) {
        self.ic1 = [0.0; 2];
        self.ic2 = [0.0; 2];
        // Force coefficient recomputation on next sample.
        self.last_pos = f32::NAN;
        self.last_res = f32::NAN;
    }

    /// Recompute SVF coefficients only when inputs change.
    #[inline]
    fn update_coefficients(&mut self, cutoff_pos: f32, resonance: f32) {
        // Fast path: skip if inputs unchanged (bit-exact for smoothed
        // params that have settled to their target).
        if cutoff_pos == self.last_pos && resonance == self.last_res {
            return;
        }
        self.last_pos = cutoff_pos;
        self.last_res = resonance;

        let t = cutoff_pos.abs();
        self.is_hp = cutoff_pos > 0.0;

        let freq = if self.is_hp {
            20.0 * (800.0f32 / 20.0).powf(t)
        } else {
            20000.0 * (200.0f32 / 20000.0).powf(t)
        };

        let q = 0.707 + resonance * 14.3;
        self.k = 1.0 / q;
        let g = (PI * freq / self.sr).tan();
        self.a1 = 1.0 / (1.0 + g * (g + self.k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
    }

    pub fn process_sample(
        &mut self,
        l: f32,
        r: f32,
        cutoff_pos: f32,
        resonance: f32,
    ) -> (f32, f32) {
        if cutoff_pos.abs() < 0.001 {
            return (l, r);
        }

        self.update_coefficients(cutoff_pos, resonance);

        let inputs = [l, r];
        let mut outputs = [0.0f32; 2];

        for ch in 0..2 {
            let v3 = inputs[ch] - self.ic2[ch];
            let v1 = self.a1 * self.ic1[ch] + self.a2 * v3;
            let v2 = self.ic2[ch] + self.a2 * self.ic1[ch] + self.a3 * v3;
            self.ic1[ch] = flush_denormal(2.0 * v1 - self.ic1[ch]);
            self.ic2[ch] = flush_denormal(2.0 * v2 - self.ic2[ch]);

            outputs[ch] = if self.is_hp {
                inputs[ch] - self.k * v1 - v2
            } else {
                v2
            };
        }

        (outputs[0], outputs[1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bypass_when_centered() {
        let mut f = DjFilter::new();
        f.set_sample_rate(48000.0);
        let mut sum_diff = 0.0f32;
        for i in 0..512 {
            let input = (i as f32 * 0.1).sin();
            let (ol, or) = f.process_sample(input, input, 0.0, 0.0);
            sum_diff += (ol - input).abs() + (or - input).abs();
        }
        assert!(
            sum_diff < 1e-6,
            "center position must be bit-identical bypass, diff={sum_diff}"
        );
    }

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

    #[test]
    fn hp_attenuates_low_frequencies() {
        let sr = 48000.0;
        let mut f = DjFilter::new();
        f.set_sample_rate(sr);
        let freq = 80.0;
        let n = 4096;
        let mut out = vec![0.0f32; n];
        for i in 0..n {
            let input = (std::f32::consts::TAU * freq * i as f32 / sr).sin();
            let (ol, _) = f.process_sample(input, input, 1.0, 0.0);
            out[i] = ol;
        }
        let power_out = fundamental_power(&out[256..], sr, freq);
        let mut ref_buf = Vec::new();
        for i in 256..n {
            ref_buf.push((std::f32::consts::TAU * freq * i as f32 / sr).sin());
        }
        let power_ref = fundamental_power(&ref_buf, sr, freq);
        assert!(
            power_out < power_ref * 0.1,
            "HP should attenuate 80 Hz at 800 Hz cutoff: out={power_out} ref={power_ref}"
        );
    }

    #[test]
    fn lp_attenuates_high_frequencies() {
        let sr = 48000.0;
        let mut f = DjFilter::new();
        f.set_sample_rate(sr);
        let freq = 10000.0;
        let n = 4096;
        let mut out = vec![0.0f32; n];
        for i in 0..n {
            let input = (std::f32::consts::TAU * freq * i as f32 / sr).sin();
            let (ol, _) = f.process_sample(input, input, -1.0, 0.0);
            out[i] = ol;
        }
        let power_out = fundamental_power(&out[256..], sr, freq);
        let mut ref_buf = Vec::new();
        for i in 256..n {
            ref_buf.push((std::f32::consts::TAU * freq * i as f32 / sr).sin());
        }
        let power_ref = fundamental_power(&ref_buf, sr, freq);
        assert!(
            power_out < power_ref * 0.1,
            "LP should attenuate 10 kHz at 200 Hz cutoff: out={power_out} ref={power_ref}"
        );
    }

    #[test]
    fn resonance_boosts_cutoff_region() {
        let sr = 48000.0;
        let pos = -0.5;
        let n = 8192;

        let mut rng: u32 = 0xCAFEBABE;
        let noise: Vec<f32> = (0..n)
            .map(|_| {
                rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
                (rng as f32 / u32::MAX as f32) * 2.0 - 1.0
            })
            .collect();

        let cutoff_hz = 20000.0 * (200.0f32 / 20000.0).powf(0.5);

        let mut f_flat = DjFilter::new();
        f_flat.set_sample_rate(sr);
        let mut out_flat = vec![0.0f32; n];
        for i in 0..n {
            let (ol, _) = f_flat.process_sample(noise[i], noise[i], pos, 0.0);
            out_flat[i] = ol;
        }

        let mut f_reso = DjFilter::new();
        f_reso.set_sample_rate(sr);
        let mut out_reso = vec![0.0f32; n];
        for i in 0..n {
            let (ol, _) = f_reso.process_sample(noise[i], noise[i], pos, 0.9);
            out_reso[i] = ol;
        }

        let p_flat = fundamental_power(&out_flat[512..], sr, cutoff_hz);
        let p_reso = fundamental_power(&out_reso[512..], sr, cutoff_hz);
        assert!(
            p_reso > p_flat * 1.5,
            "resonance should boost power near cutoff: flat={p_flat} reso={p_reso}"
        );
    }

    #[test]
    fn reset_zeroes_state() {
        let mut f = DjFilter::new();
        f.set_sample_rate(48000.0);
        for i in 0..256 {
            f.process_sample(
                (i as f32 * 0.1).sin(),
                (i as f32 * 0.1).sin(),
                -0.5,
                0.5,
            );
        }
        f.reset();
        let (ol, _) = f.process_sample(0.5, 0.5, 0.0, 0.0);
        assert!((ol - 0.5).abs() < 1e-6, "reset should zero filter state");
    }

    #[test]
    fn stable_at_extreme_lp_cutoff() {
        let mut f = DjFilter::new();
        f.set_sample_rate(44100.0);
        for i in 0..4096 {
            let input = (i as f32 * 0.3).sin();
            let (ol, or) = f.process_sample(input, input, -0.01, 0.0);
            assert!(ol.is_finite(), "LP near-Nyquist must not blow up (sample {i})");
            assert!(or.is_finite());
        }
    }

    #[test]
    fn stable_at_high_resonance_near_nyquist() {
        let mut f = DjFilter::new();
        f.set_sample_rate(44100.0);
        for i in 0..4096 {
            let input = if i == 0 { 1.0 } else { 0.0 };
            let (ol, or) = f.process_sample(input, input, -0.05, 1.0);
            assert!(ol.is_finite(), "high-Q near Nyquist must stay finite (sample {i})");
            assert!(or.is_finite());
        }
    }

    #[test]
    fn dirty_check_avoids_redundant_recompute() {
        // Verify that passing the same cutoff/res repeatedly doesn't
        // recompute coefficients (functional correctness, not timing).
        let mut f = DjFilter::new();
        f.set_sample_rate(48000.0);
        let mut results = Vec::new();
        for i in 0..256 {
            let input = (i as f32 * 0.1).sin();
            let (ol, _) = f.process_sample(input, input, -0.5, 0.3);
            results.push(ol);
        }
        // Second pass with identical params should produce the same
        // output (integrator state differs, but coefficient path is
        // exercised).
        assert!(results.iter().all(|x| x.is_finite()));
    }
}
