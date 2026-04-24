//! Real-time spectrum analyzer for the OUTPUT display.
//!
//! Fills a 1024-sample ring at `sr`; when full, applies a Hann window and
//! runs a real FFT (`realfft`), then aggregates the 513-bin complex result
//! into 64 log-spaced dB bands covering 20 Hz → 20 kHz.
//!
//! All buffers are allocated once in [`SpectrumAnalyzer::new`]; `feed_sample`
//! performs only in-place arithmetic and a single FFT call per 1024 samples,
//! so the analyzer is safe to call from the audio thread under
//! `assert_process_allocs`.

use realfft::{num_complex::Complex, RealFftPlanner, RealToComplex};
use std::sync::Arc;

/// FFT size. 1024 @ 48 kHz ⇒ 21 ms window, 47 Hz bin width — enough low-end
/// resolution to see the kick fundamental while still repainting ~every 21 ms.
pub const FFT_SIZE: usize = 1024;

/// Number of log-spaced output bands shown on screen.
pub const BINS: usize = 64;

/// Display magnitude floor (dB). Below this the bar draws at zero height.
pub const DB_FLOOR: f32 = -60.0;

/// Display magnitude ceiling (dB). A full-scale sine hits 0 dB.
pub const DB_CEIL: f32 = 0.0;

pub struct SpectrumAnalyzer {
    fft: Arc<dyn RealToComplex<f32>>,
    /// Hann window, pre-computed once.
    window: Vec<f32>,
    /// Normalization factor to map a full-scale sine (peak = 1.0) to 0 dB
    /// after the Hann correction. Derived from `sum(window)`.
    norm: f32,
    /// Sample ring. Fills linearly; on wrap we compute an FFT.
    ring: Vec<f32>,
    ring_pos: usize,
    /// FFT input scratch (windowed copy of the ring). Reused every FFT.
    fft_in: Vec<f32>,
    /// FFT output scratch. `FFT_SIZE/2 + 1` complex values.
    fft_out: Vec<Complex<f32>>,
    /// realfft-requested scratch buffer. Pre-allocated so `process_with_scratch`
    /// is zero-alloc (unlike plain `process`, which grows its own scratch).
    fft_scratch: Vec<Complex<f32>>,
    /// Inclusive/exclusive bin-index edges into `fft_out` for each of the
    /// `BINS` display bands. Length `BINS + 1`. Precomputed from `sample_rate`.
    band_edges: Vec<usize>,
    /// Most recently computed dB magnitudes per band.
    bins_db: [f32; BINS],
}

impl SpectrumAnalyzer {
    pub fn new(sample_rate: f32) -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);

        // Hann window: 0.5 * (1 - cos(2π n / (N-1)))
        let mut window = vec![0.0f32; FFT_SIZE];
        let n_minus_one = (FFT_SIZE - 1) as f32;
        let mut win_sum = 0.0f32;
        for (i, w) in window.iter_mut().enumerate() {
            let v =
                0.5 - 0.5 * (std::f32::consts::TAU * (i as f32) / n_minus_one).cos();
            *w = v;
            win_sum += v;
        }
        // For a real sine of amplitude 1.0, the FFT peak-bin magnitude after
        // Hann windowing is approximately `win_sum / 2`. Divide by that to
        // map full-scale to 1.0, then log-convert to 0 dB.
        let norm = if win_sum > 0.0 { 2.0 / win_sum } else { 1.0 };

        let band_edges = compute_band_edges(sample_rate);
        let fft_scratch = fft.make_scratch_vec();

        Self {
            fft,
            window,
            norm,
            ring: vec![0.0f32; FFT_SIZE],
            ring_pos: 0,
            fft_in: vec![0.0f32; FFT_SIZE],
            fft_out: vec![Complex::new(0.0f32, 0.0f32); FFT_SIZE / 2 + 1],
            fft_scratch,
            band_edges,
            bins_db: [DB_FLOOR; BINS],
        }
    }

    /// Update band edges when the sample rate changes (e.g. DAW switched rate).
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.band_edges = compute_band_edges(sample_rate);
        self.ring.iter_mut().for_each(|s| *s = 0.0);
        self.ring_pos = 0;
        self.bins_db = [DB_FLOOR; BINS];
    }

    /// Push one mono sample. Returns `true` when the ring just wrapped and
    /// `bins_db` was updated with a fresh FFT result.
    #[inline]
    pub fn feed_sample(&mut self, s: f32) -> bool {
        self.ring[self.ring_pos] = s;
        self.ring_pos += 1;
        if self.ring_pos >= FFT_SIZE {
            self.ring_pos = 0;
            self.compute();
            true
        } else {
            false
        }
    }

    /// Latest dB-per-band snapshot.
    #[inline]
    pub fn bins_db(&self) -> &[f32; BINS] {
        &self.bins_db
    }

    fn compute(&mut self) {
        // Window the ring into fft_in (in-place copy × Hann).
        for i in 0..FFT_SIZE {
            self.fft_in[i] = self.ring[i] * self.window[i];
        }

        // Use `process_with_scratch` — plain `process()` grows an internal
        // scratch vec on the hot path, which would trip `assert_process_allocs`
        // when called from the audio thread. Error path: only returns Err on
        // a length mismatch, which is a static invariant here; ignore.
        let _ = self.fft.process_with_scratch(
            &mut self.fft_in,
            &mut self.fft_out,
            &mut self.fft_scratch,
        );

        // Aggregate into log-spaced bands by taking the max magnitude in each.
        // Max-pool rather than averaging so a lone tonal peak stays visible at
        // wide bands (the right feel for a kick spectrum).
        for (i, w) in self.bins_db.iter_mut().enumerate() {
            let start = self.band_edges[i];
            // Ensure each band has at least one FFT bin, else a narrow low-end
            // band (e.g. 20 → 22 Hz on a fine FFT) could be empty.
            let end = self.band_edges[i + 1].max(start + 1);
            let mut peak_mag = 0.0f32;
            for k in start..end {
                // SAFETY-adjacent: `end` is clamped to fft_out.len() in
                // compute_band_edges via `nyquist_bin`, so no bounds issue.
                let m = self.fft_out[k].norm();
                if m > peak_mag {
                    peak_mag = m;
                }
            }
            let scaled = peak_mag * self.norm;
            let db = 20.0 * (scaled + 1e-9).log10();
            *w = db.clamp(DB_FLOOR, DB_CEIL);
        }
    }
}

/// Build the 65-element band-edge table in FFT-bin indices for a log-spaced
/// axis from 20 Hz to min(20 kHz, Nyquist). Clamped so edges stay within
/// `[0, FFT_SIZE / 2]`.
fn compute_band_edges(sample_rate: f32) -> Vec<usize> {
    let nyquist_bin = FFT_SIZE / 2;
    let f_min = 20.0_f32;
    let f_max = 20_000.0_f32.min(sample_rate * 0.5);
    let fft_size_f = FFT_SIZE as f32;
    let mut edges = Vec::with_capacity(BINS + 1);
    for i in 0..=BINS {
        let t = (i as f32) / (BINS as f32);
        let f = f_min * (f_max / f_min).powf(t);
        let bin = (f * fft_size_f / sample_rate).round() as isize;
        let bin = bin.clamp(0, nyquist_bin as isize) as usize;
        edges.push(bin);
    }
    edges
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn band_edges_cover_expected_range() {
        let edges = compute_band_edges(48_000.0);
        assert_eq!(edges.len(), BINS + 1);
        // First edge should map to ≥ bin for 20 Hz @ 48k / 1024 ≈ bin 0 (0 Hz)
        // or 1 (46.9 Hz). Accept either — the exact rounding isn't load-bearing.
        assert!(edges[0] <= 1);
        // Last edge caps at 20 kHz by design (not Nyquist), so at 48 kHz /
        // 1024 that's bin ≈ 427. Allow a small rounding window.
        let expected_top = (20_000.0 * FFT_SIZE as f32 / 48_000.0).round() as usize;
        assert!(
            (edges[BINS] as isize - expected_top as isize).abs() <= 1,
            "last edge = {}, expected near {expected_top}",
            edges[BINS]
        );
        assert!(edges[BINS] <= FFT_SIZE / 2);
        // Monotonic.
        for pair in edges.windows(2) {
            assert!(pair[1] >= pair[0], "edges not monotonic: {pair:?}");
        }
    }

    #[test]
    fn band_edges_cap_at_nyquist_for_low_sample_rate() {
        // At 22.05 kHz the 20 kHz cap disappears — Nyquist (11.025 kHz) is
        // lower, so the last edge must clamp to the Nyquist bin, not overshoot.
        let edges = compute_band_edges(22_050.0);
        assert_eq!(edges[BINS], FFT_SIZE / 2);
    }

    #[test]
    fn full_scale_sine_hits_top_of_range() {
        let sr = 48_000.0f32;
        let mut sp = SpectrumAnalyzer::new(sr);
        // Pump a 1.0-amplitude sine at 1 kHz (well inside a non-edge band).
        let freq = 1000.0f32;
        let phase_step = std::f32::consts::TAU * freq / sr;
        let mut phase = 0.0f32;
        let mut produced = false;
        // Feed enough samples to fill the ring at least once.
        for _ in 0..(FFT_SIZE * 2) {
            let s = phase.sin();
            if sp.feed_sample(s) {
                produced = true;
            }
            phase += phase_step;
            if phase >= std::f32::consts::TAU {
                phase -= std::f32::consts::TAU;
            }
        }
        assert!(produced, "FFT never ran — ring didn't wrap");
        // Expect at least one bin within ~3 dB of 0 dB (full-scale).
        let peak = sp
            .bins_db()
            .iter()
            .copied()
            .fold(DB_FLOOR, f32::max);
        assert!(
            peak >= -3.0,
            "expected a bin near 0 dB for a full-scale sine, got peak = {peak} dB"
        );
        // Sanity: something *below* -30 dB should also exist (not every band
        // is lit up — silence elsewhere).
        let min = sp
            .bins_db()
            .iter()
            .copied()
            .fold(DB_CEIL, f32::min);
        assert!(
            min <= -30.0,
            "expected quiet bands below -30 dB, got min = {min} dB"
        );
    }

    #[test]
    fn silence_floors_all_bands() {
        let mut sp = SpectrumAnalyzer::new(48_000.0);
        for _ in 0..FFT_SIZE {
            sp.feed_sample(0.0);
        }
        for (i, &db) in sp.bins_db().iter().enumerate() {
            assert!(
                db <= DB_FLOOR + 0.01,
                "band {i} wasn't clamped to floor on silence: {db} dB"
            );
        }
    }
}
