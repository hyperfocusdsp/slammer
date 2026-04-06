//! Master-bus dynamics chain: RMS compressor → transformer drive → brickwall limiter.
//!
//! All state is pre-allocated and updated in-place; every call to
//! [`MasterBus::process_sample`] is pure scalar arithmetic plus one `exp()`
//! only when attack/release times change (dirty-checked in [`set_times`]).
//! Safe to run under nih-plug's `assert_process_allocs`.
//!
//! The compressor uses a rolling RMS detector + a one-pole envelope follower
//! in the dB domain — the classic Autokit master-bus recipe. Macro mapping
//! (single "amount" → threshold+ratio, single "react" → attack+release) lives
//! in `plugin.rs`; this module just takes raw DSP values.
//!
//! **Stage 2 is a transformer model, not a tanh shaper.** Previous versions
//! used `tanh(x·pre) · post`, which is the same odd-harmonic curve as the
//! kick-stage `Sat Clip` mode — there was no meaningful character difference
//! between them. The transformer injects a controlled mix of 2nd and 3rd
//! harmonics via a polynomial waveshaper and adds a small drive-dependent
//! LF bloom (one-pole LP feed-forward) to emulate how an iron-core output
//! transformer's saturation pulls on sustained low frequencies. That gives
//! it a distinct harmonic + frequency fingerprint vs. every other stage in
//! the plugin.

use nih_plug::util;

/// Brickwall ceiling for the optional limiter stage, in dBFS. Chosen to leave
/// a hair of headroom for inter-sample peaks in downstream DAC converters.
const LIMITER_CEILING_DB: f32 = -0.3;

/// Transformer LF-bloom LP cutoff (Hz). The one-pole LP on the input feeds
/// forward as a drive-scaled boost after the shaper — a loose analogue of
/// how an iron core's saturation response interacts with low frequencies.
const XFMR_BLOOM_LP_HZ: f32 = 60.0;

/// Transformer input clamp before the cubic term. `x³` grows fast; clamping
/// to a few times unity keeps the polynomial well-behaved under pathological
/// signal levels (inter-sample peaks, feedback, etc.).
const XFMR_INPUT_CLAMP: f32 = 3.0;

/// Rolling RMS detector window length (seconds). 5 ms is short enough to react
/// to kick transients but long enough to ignore cycle-by-cycle fluctuation.
const RMS_WINDOW_SECS: f32 = 0.005;

/// Fast limiter time constants (fixed — these aren't user-exposed).
const LIM_ATTACK_MS: f32 = 0.1;
const LIM_RELEASE_MS: f32 = 50.0;

const MAX_RMS_BUF: usize = 8192;

/// Master bus chain: RMS compressor → transformer saturator → brickwall limiter.
/// Zero allocations on the audio thread.
pub struct MasterBus {
    // Detector
    rms_buf: Box<[f32; MAX_RMS_BUF]>,
    rms_write: usize,
    rms_sum: f32,
    rms_buf_len: usize,
    env_db: f32,

    // Limiter
    lim_env: f32,
    limiter_ceiling_lin: f32,

    // Transformer LF-bloom LP state (per channel).
    xfmr_bloom_l: f32,
    xfmr_bloom_r: f32,
    /// Cached LP smoothing coefficient for the LF bloom. Recomputed only in
    /// `prepare()` when sample rate changes.
    xfmr_bloom_alpha: f32,

    // Coefficients (recomputed only when (atk,rel) change)
    comp_attack_coeff: f32,
    comp_release_coeff: f32,
    lim_attack_coeff: f32,
    lim_release_coeff: f32,

    // Dirty-check for set_times
    last_atk_ms: f32,
    last_rel_ms: f32,
    last_sr: f32,

    // Reported to GUI meter. Positive = dB of reduction currently applied.
    last_gr_db: f32,
}

impl MasterBus {
    pub fn new() -> Self {
        Self {
            rms_buf: Box::new([0.0f32; MAX_RMS_BUF]),
            rms_write: 0,
            rms_sum: 0.0,
            rms_buf_len: 1,
            env_db: -60.0,
            lim_env: 0.0,
            limiter_ceiling_lin: util::db_to_gain(LIMITER_CEILING_DB),
            xfmr_bloom_l: 0.0,
            xfmr_bloom_r: 0.0,
            xfmr_bloom_alpha: 0.0,
            comp_attack_coeff: 0.0,
            comp_release_coeff: 0.0,
            lim_attack_coeff: 0.0,
            lim_release_coeff: 0.0,
            last_atk_ms: -1.0,
            last_rel_ms: -1.0,
            last_sr: 0.0,
            last_gr_db: 0.0,
        }
    }

    /// Reset all state and recompute coefficients for a new sample rate.
    /// Call from `Plugin::initialize()` and `Plugin::reset()`.
    pub fn prepare(&mut self, sample_rate: f32) {
        self.last_sr = sample_rate;
        self.lim_attack_coeff = coeff(LIM_ATTACK_MS, sample_rate);
        self.lim_release_coeff = coeff(LIM_RELEASE_MS, sample_rate);

        // Transformer LF-bloom LP coefficient.
        let rc = 1.0 / (std::f32::consts::TAU * XFMR_BLOOM_LP_HZ);
        let dt = 1.0 / sample_rate;
        self.xfmr_bloom_alpha = dt / (rc + dt);
        self.xfmr_bloom_l = 0.0;
        self.xfmr_bloom_r = 0.0;

        self.rms_buf_len = ((RMS_WINDOW_SECS * sample_rate).ceil() as usize).clamp(1, MAX_RMS_BUF);
        self.rms_write = 0;
        self.rms_sum = 0.0;
        self.rms_buf.fill(0.0);
        self.env_db = -60.0;
        self.lim_env = 0.0;
        self.last_gr_db = 0.0;

        // Force the next set_times() call to recompute.
        self.last_atk_ms = -1.0;
        self.last_rel_ms = -1.0;
    }

    /// Update comp attack/release coefficients iff the times changed since the
    /// last call. Keeps the hot path free of `exp()` at the audio rate.
    #[inline]
    pub fn set_times(&mut self, atk_ms: f32, rel_ms: f32, sample_rate: f32) {
        if atk_ms != self.last_atk_ms || sample_rate != self.last_sr {
            self.comp_attack_coeff = coeff(atk_ms, sample_rate);
            self.last_atk_ms = atk_ms;
        }
        if rel_ms != self.last_rel_ms || sample_rate != self.last_sr {
            self.comp_release_coeff = coeff(rel_ms, sample_rate);
            self.last_rel_ms = rel_ms;
        }
        if sample_rate != self.last_sr {
            self.lim_attack_coeff = coeff(LIM_ATTACK_MS, sample_rate);
            self.lim_release_coeff = coeff(LIM_RELEASE_MS, sample_rate);
            self.last_sr = sample_rate;
        }
    }

    /// Last-sample gain reduction in dB (positive value). Read by the GUI
    /// once per buffer for the meter.
    #[inline]
    pub fn last_gr_db(&self) -> f32 {
        self.last_gr_db
    }

    /// Process a single stereo sample through the chain.
    ///
    /// * `threshold_db` — compressor threshold (dBFS)
    /// * `ratio` — compressor ratio (≥ 1.0). 1.0 = no compression.
    /// * `drive` — 0..1 tanh post-compressor drive amount.
    /// * `limiter_on` — enables the brickwall limiter stage.
    #[inline]
    pub fn process_sample(
        &mut self,
        l: f32,
        r: f32,
        threshold_db: f32,
        ratio: f32,
        drive: f32,
        limiter_on: bool,
    ) -> (f32, f32) {
        // ── Stage 1: RMS compressor ──

        // Mono sum for level detection.
        let mid = (l + r) * 0.5;
        let sq = mid * mid;

        // Rolling RMS window.
        let old = self.rms_buf[self.rms_write];
        self.rms_sum = (self.rms_sum - old + sq).max(0.0);
        self.rms_buf[self.rms_write] = sq;
        self.rms_write += 1;
        if self.rms_write >= self.rms_buf_len {
            self.rms_write = 0;
        }

        let rms_lin = (self.rms_sum / self.rms_buf_len as f32).sqrt();
        let rms_db = util::gain_to_db(rms_lin.max(1e-9));

        // One-pole envelope follower in dB domain.
        let env_coeff = if rms_db > self.env_db {
            self.comp_attack_coeff
        } else {
            self.comp_release_coeff
        };
        self.env_db = env_coeff * self.env_db + (1.0 - env_coeff) * rms_db;

        // Gain computation.
        let overshoot_db = self.env_db - threshold_db;
        let inv_ratio = 1.0 / ratio;
        let gain_reduction_db = if overshoot_db > 0.0 && ratio > 1.0 {
            -overshoot_db * (1.0 - inv_ratio)
        } else {
            0.0
        };

        // Report positive GR to the meter.
        self.last_gr_db = -gain_reduction_db;

        // Auto-makeup: compensate by half the max theoretical GR.
        let makeup_db = -threshold_db * (1.0 - inv_ratio) * 0.5;
        let comp_gain = util::db_to_gain(gain_reduction_db + makeup_db);

        let mut l = l * comp_gain;
        let mut r = r * comp_gain;

        // ── Stage 2: Transformer drive ──
        //
        // Polynomial waveshaper with explicit 2nd- and 3rd-harmonic
        // injection, plus a drive-scaled LF bloom (one-pole LP of the
        // input fed forward after the shape). This gives the bus drive a
        // mixed-harmonic + frequency-dependent character that doesn't
        // overlap with the `tanh`/rational shapers used elsewhere.

        if drive > 0.001 {
            let pre_gain = 1.0 + drive * 3.0;
            // Harmonic coefficients grow with drive.
            //   h2 weights `x * |x|` — pure even-harmonic term (no
            //       fundamental, no DC on a symmetric signal).
            //   h3 weights `x³ − 0.75·x` — pure 3rd-harmonic term (the
            //       `0.75·x` subtraction cancels the fundamental gain that
            //       `x³` would otherwise contribute on a unit sine).
            let h2 = drive * 0.35;
            let h3 = drive * 0.25;
            let post_gain = 1.0 / (1.0 + drive * 0.6);
            let bloom_gain = drive * 0.25;

            let xl = (l * pre_gain).clamp(-XFMR_INPUT_CLAMP, XFMR_INPUT_CLAMP);
            let xr = (r * pre_gain).clamp(-XFMR_INPUT_CLAMP, XFMR_INPUT_CLAMP);

            let shaped_l = xl + h2 * xl * xl.abs() + h3 * (xl * xl * xl - 0.75 * xl);
            let shaped_r = xr + h2 * xr * xr.abs() + h3 * (xr * xr * xr - 0.75 * xr);

            // LF bloom: one-pole LP of the pre-shape input, fed forward
            // after the shaper. On a kick this re-emphasizes the sub
            // fundamental the harder you drive, giving a sense of the
            // transformer core "blooming" on transients rather than
            // collapsing like a plain tanh would.
            self.xfmr_bloom_l += self.xfmr_bloom_alpha * (l - self.xfmr_bloom_l);
            self.xfmr_bloom_r += self.xfmr_bloom_alpha * (r - self.xfmr_bloom_r);
            self.xfmr_bloom_l = flush_denormal(self.xfmr_bloom_l);
            self.xfmr_bloom_r = flush_denormal(self.xfmr_bloom_r);

            l = shaped_l * post_gain + self.xfmr_bloom_l * bloom_gain;
            r = shaped_r * post_gain + self.xfmr_bloom_r * bloom_gain;
        }

        // ── Stage 3: Brickwall limiter ──

        if limiter_on {
            let peak = l.abs().max(r.abs());
            self.lim_env = if peak > self.lim_env {
                self.lim_attack_coeff * self.lim_env + (1.0 - self.lim_attack_coeff) * peak
            } else {
                self.lim_release_coeff * self.lim_env + (1.0 - self.lim_release_coeff) * peak
            };
            if self.lim_env > self.limiter_ceiling_lin {
                let lim_gain = self.limiter_ceiling_lin / self.lim_env;
                l *= lim_gain;
                r *= lim_gain;
            }
        }

        (l, r)
    }
}

#[inline]
fn coeff(ms: f32, sr: f32) -> f32 {
    (-1.0 / (ms * 0.001 * sr)).exp()
}

#[inline]
fn flush_denormal(x: f32) -> f32 {
    if x.is_subnormal() {
        0.0
    } else {
        x
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bypass_is_unity_at_low_level() {
        let mut mb = MasterBus::new();
        mb.prepare(48_000.0);
        mb.set_times(10.0, 100.0, 48_000.0);
        // Well below any reasonable threshold; no GR should occur, so the
        // output should be the input times the auto-makeup gain (fixed for
        // a given threshold/ratio), which for threshold = -6 / ratio = 2
        // is db_to_gain(1.5) ≈ 1.189.
        let (l, r) = mb.process_sample(0.01, 0.01, -6.0, 2.0, 0.0, false);
        let expected = 0.01 * util::db_to_gain(1.5);
        assert!((l - expected).abs() < 1e-4, "l={} expected={}", l, expected);
        assert!((r - expected).abs() < 1e-4);
        assert_eq!(mb.last_gr_db(), 0.0);
    }

    #[test]
    fn heavy_signal_triggers_gain_reduction() {
        let mut mb = MasterBus::new();
        mb.prepare(48_000.0);
        mb.set_times(1.0, 50.0, 48_000.0);
        // Hammer a loud signal to fill the RMS window and the envelope.
        for _ in 0..2048 {
            mb.process_sample(0.9, 0.9, -12.0, 4.0, 0.0, false);
        }
        assert!(mb.last_gr_db() > 0.5, "expected GR, got {}", mb.last_gr_db());
    }

    #[test]
    fn limiter_clamps_peaks() {
        let mut mb = MasterBus::new();
        mb.prepare(48_000.0);
        mb.set_times(10.0, 100.0, 48_000.0);
        let ceiling = util::db_to_gain(LIMITER_CEILING_DB);
        // Settle the limiter envelope on a loud DC signal.
        for _ in 0..512 {
            let (l, r) = mb.process_sample(2.0, 2.0, 0.0, 1.0, 0.0, true);
            // l may briefly overshoot during attack; after settling it must
            // not exceed the ceiling.
            let _ = (l, r);
        }
        let (l, r) = mb.process_sample(2.0, 2.0, 0.0, 1.0, 0.0, true);
        assert!(l.abs() <= ceiling * 1.02, "l={} ceiling={}", l, ceiling);
        assert!(r.abs() <= ceiling * 1.02);
    }

    /// Naive Goertzel-style single-bin power at `bin_freq` for a signal
    /// assumed periodic with fundamental `fund`. Returns the squared
    /// magnitude of the complex correlation, which is all we need for an
    /// ordering comparison (tanh vs transformer relative 3rd harmonic).
    fn harmonic_power(samples: &[f32], sr: f32, bin_freq: f32) -> f32 {
        let w = std::f32::consts::TAU * bin_freq / sr;
        let mut re = 0.0f32;
        let mut im = 0.0f32;
        for (i, &x) in samples.iter().enumerate() {
            let p = w * i as f32;
            re += x * p.cos();
            im += x * p.sin();
        }
        re * re + im * im
    }

    #[test]
    fn transformer_injects_third_harmonic() {
        // Feed a pure 220 Hz sine at a level that clearly exercises the
        // shaper (~0.6) and compare the 3rd-harmonic (660 Hz) energy of
        // the transformer stage against what a plain tanh drive would
        // produce at matched pre-gain. Both shapes generate odd harmonics,
        // but the transformer's explicit `x³ − 0.75·x` term should produce
        // a distinct (non-zero, significantly different) 3rd-harmonic
        // signature — if we accidentally collapse back to pure tanh, this
        // test will notice.
        let sr = 48_000.0;
        let fund = 220.0;
        let n = 4096;

        // Transformer output (real bus at drive=0.7, comp bypassed).
        let mut mb = MasterBus::new();
        mb.prepare(sr);
        mb.set_times(10.0, 100.0, sr);
        let mut xf = vec![0.0f32; n];
        for i in 0..n {
            let x = (std::f32::consts::TAU * fund * i as f32 / sr).sin() * 0.6;
            // amount=0, ratio=1 → compressor bypass; drive=0.7 triggers the
            // transformer stage; limiter off.
            let (y, _) = mb.process_sample(x, x, 0.0, 1.0, 0.7, false);
            xf[i] = y;
        }

        // Reference tanh output at a comparable pre-gain.
        let mut th = vec![0.0f32; n];
        let pre = 1.0 + 0.7 * 3.0;
        let post = 1.0 / (1.0 + 0.7 * 0.6);
        for i in 0..n {
            let x = (std::f32::consts::TAU * fund * i as f32 / sr).sin() * 0.6;
            th[i] = (x * pre).tanh() * post;
        }

        // Drop the first 256 samples to let the LF-bloom LP settle.
        let start = 256;
        let xf_power = harmonic_power(&xf[start..], sr, fund * 3.0);
        let th_power = harmonic_power(&th[start..], sr, fund * 3.0);
        let fund_power = harmonic_power(&xf[start..], sr, fund);

        // Sanity: the fundamental should dominate the 3rd harmonic.
        assert!(
            fund_power > xf_power,
            "fundamental should be louder than H3: fund={fund_power} h3={xf_power}"
        );
        // The transformer must produce non-trivial H3 content.
        assert!(
            xf_power > fund_power * 1e-4,
            "transformer should inject audible H3 energy: h3={xf_power} fund={fund_power}"
        );
        // And the transformer's H3 must differ meaningfully from plain
        // tanh's H3 at matched pre-gain — that's the whole point of
        // retuning this stage.
        let ratio = xf_power / th_power.max(1e-12);
        assert!(
            !(0.9..=1.1).contains(&ratio),
            "transformer H3 should NOT match tanh H3: ratio={ratio} xf={xf_power} th={th_power}"
        );
    }
}
