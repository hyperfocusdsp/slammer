use crate::dsp::click::ClickGen;
use crate::dsp::drift::Drift;
use crate::dsp::envelope::{AmpEnvelope, PitchEnvelope};
use crate::dsp::filter::{EqParams, MasterEq};
use crate::dsp::noise::NoiseGen;
use crate::dsp::oscillator::SineOsc;
use crate::dsp::saturation::{SatMode, Saturation};

/// Voice-steal fadeout time. When a new trigger arrives while another voice
/// is still audible, that voice is linearly ramped to silence over this many
/// milliseconds while the new voice starts fresh in the other slot. The two
/// voices sum into the shared saturation + EQ chain so the crossfade is
/// seamless and there's no step discontinuity.
const VOICE_FADEOUT_MS: f32 = 5.0;

/// Number of voice slots. Four gives every hit in a flam/ruff/roll group its
/// own slot so overlapping triggers (up to 4 within ~90 ms) don't steal each
/// other mid-tail; the outgoing voice still fades out over ~5 ms during
/// normal retriggers.
const NUM_VOICES: usize = 4;

/// Fixed size of the pending-hit ring on `KickEngine`. 12 slots cover the
/// pathological case of 4 hits × 3 overlapping step-boundaries.
const PENDING_RING_SIZE: usize = 12;

/// A single scheduled future engine trigger. Countdown is sample-based;
/// when `samples_until` reaches 0 on a `tick_pending` call, the slot fires
/// (returning its velocity) and is marked dead.
#[derive(Copy, Clone, Debug, Default)]
struct PendingHit {
    samples_until: u32,
    velocity: f32,
    live: bool,
}

/// A single kick voice: all per-trigger state (oscillators, envelopes,
/// click, noise) plus a linear fadeout multiplier used for voice stealing.
struct KickVoice {
    // SUB
    sub_osc: SineOsc,
    sub_pitch_env: PitchEnvelope,
    sub_amp_env: AmpEnvelope,
    // MID
    mid_osc: SineOsc,
    mid_pitch_env: PitchEnvelope,
    mid_amp_env: AmpEnvelope,
    mid_noise: NoiseGen,
    // TOP
    top_click: ClickGen,
    top_amp_env: AmpEnvelope,
    /// Per-voice velocity captured at trigger time (used for `velocity_sens`).
    velocity: f32,
    /// Voice-level output multiplier. Normally 1.0. When this voice is
    /// stolen, `fadeout_step` is set and `fadeout_gain` decreases linearly
    /// each sample until it reaches 0, at which point the voice is dead.
    fadeout_gain: f32,
    fadeout_step: f32,
    /// True once any generator in this voice has been triggered; gates the
    /// early-exit in `KickEngine::process()`.
    triggered: bool,
}

impl KickVoice {
    fn new(sample_rate: f32) -> Self {
        Self {
            sub_osc: SineOsc::new(sample_rate),
            sub_pitch_env: PitchEnvelope::new(sample_rate),
            sub_amp_env: AmpEnvelope::new(sample_rate),
            mid_osc: SineOsc::new(sample_rate),
            mid_pitch_env: PitchEnvelope::new(sample_rate),
            mid_amp_env: AmpEnvelope::new(sample_rate),
            mid_noise: NoiseGen::new(sample_rate),
            top_click: ClickGen::new(sample_rate),
            top_amp_env: AmpEnvelope::new(sample_rate),
            velocity: 0.0,
            fadeout_gain: 1.0,
            fadeout_step: 0.0,
            triggered: false,
        }
    }

    fn set_sample_rate(&mut self, sample_rate: f32) {
        *self = Self::new(sample_rate);
    }

    /// Voice is still producing audio: one of its generators has state AND
    /// its fadeout hasn't fully killed it.
    fn is_active(&self) -> bool {
        if !self.triggered {
            return false;
        }
        if self.fadeout_gain <= 0.0 {
            return false;
        }
        self.sub_amp_env.is_active()
            || self.mid_amp_env.is_active()
            || self.top_click.is_active()
    }

    fn trigger(
        &mut self,
        params: &KickParams,
        velocity: f32,
        drift: &mut Drift,
        sample_rate: f32,
    ) {
        self.velocity = velocity;
        self.fadeout_gain = 1.0;
        self.fadeout_step = 0.0;
        self.triggered = true;

        // Analog drift: per-trigger pitch + phase jitter
        let (sub_pf, sub_pd) = drift.jitter(params.drift_amount);
        let (mid_pf, mid_pd) = drift.jitter(params.drift_amount);

        // SUB
        self.sub_pitch_env.trigger(
            params.sub_fstart * sub_pf,
            params.sub_fend * sub_pf,
            params.sub_sweep_ms / 1000.0,
            params.sub_sweep_curve,
        );
        self.sub_amp_env.trigger(params.decay_ms);
        self.sub_osc.trigger(params.sub_phase_offset + sub_pd);

        // MID
        self.mid_pitch_env.trigger(
            params.mid_fstart * mid_pf,
            params.mid_fend * mid_pf,
            params.mid_sweep_ms / 1000.0,
            params.mid_sweep_curve,
        );
        self.mid_amp_env.trigger(params.mid_decay_ms);
        self.mid_osc.trigger(params.mid_phase_offset + mid_pd);
        self.mid_noise.trigger();

        // TOP
        self.top_click.regenerate(
            sample_rate,
            params.top_decay_ms,
            params.top_freq,
            params.top_bw,
        );
        self.top_click.trigger();
        self.top_amp_env.trigger(params.top_decay_ms);
    }

    /// Start a linear fadeout over `VOICE_FADEOUT_MS`. Called when this
    /// voice is being stolen by a new trigger.
    fn start_fadeout(&mut self, sample_rate: f32) {
        let samples = (VOICE_FADEOUT_MS * 0.001 * sample_rate).max(1.0);
        // If we're already fading, don't extend the ramp — keep whichever
        // step is steeper so an already-dying voice doesn't linger.
        let new_step = 1.0 / samples;
        if self.fadeout_step == 0.0 || new_step > self.fadeout_step {
            self.fadeout_step = new_step;
        }
    }

    /// Generate one sample of this voice's contribution (pre-saturation,
    /// pre-EQ). Returns 0.0 if the voice isn't producing audio.
    fn tick(&mut self, params: &KickParams) -> f32 {
        if !self.is_active() {
            return 0.0;
        }

        // SUB: sine with pitch sweep + amp envelope
        let sub_freq = self.sub_pitch_env.tick();
        let sub_amp = self.sub_amp_env.tick();
        let sub = self.sub_osc.tick(sub_freq) * params.sub_gain * sub_amp;

        // MID: sine + noise, own pitch envelope + amp envelope
        let mid_freq = self.mid_pitch_env.tick();
        let mid_amp = self.mid_amp_env.tick();
        let mid_tone = self.mid_osc.tick(mid_freq) * params.mid_tone_gain;
        let mid_noise = self.mid_noise.tick(params.mid_noise_color) * params.mid_noise_gain;
        let mid = (mid_tone + mid_noise) * params.mid_gain * mid_amp;

        // TOP: click transient with its own amp envelope for anti-click
        let top_raw = self.top_click.tick();
        let top_amp = self.top_amp_env.tick();
        let top = top_raw * params.top_gain * top_amp;

        let vel_gain = params.velocity_sens * self.velocity + (1.0 - params.velocity_sens);
        let sample = (sub + mid + top) * vel_gain * self.fadeout_gain;

        // Advance fadeout ramp (if any)
        if self.fadeout_step > 0.0 {
            self.fadeout_gain -= self.fadeout_step;
            if self.fadeout_gain <= 0.0 {
                self.fadeout_gain = 0.0;
                self.triggered = false;
            }
        }

        sample
    }
}

/// Three-layer kick synthesis engine: SUB / MID / TOP, with 2-slot voice
/// stealing so retriggers crossfade instead of snapping.
///
/// Signal flow:
///
/// ```text
///   Voice 0 ─┐
///            ├─► Sum ─► Saturation ─► Master EQ ─► Output
///   Voice 1 ─┘
/// ```
///
/// On `trigger()`, if the currently-active voice is still producing audio,
/// a ~5 ms linear fadeout is started on it and the next trigger lands in
/// the other slot. The two voices sum into the shared post-mix chain, so
/// the old voice's tail smoothly ramps to zero while the new voice ramps
/// in via its own 1 ms attack envelope — no step discontinuity anywhere.
pub struct KickEngine {
    voices: [KickVoice; NUM_VOICES],
    /// Index of the slot most recently triggered. A new trigger advances to
    /// the next slot (mod `NUM_VOICES`) whenever the current one is still
    /// audible.
    active_voice: usize,
    saturation: Saturation,
    master_eq: MasterEq,
    drift: Drift,
    sample_rate: f32,
    pending: [PendingHit; PENDING_RING_SIZE],
    rng_state: u32,
}

impl KickEngine {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            voices: std::array::from_fn(|_| KickVoice::new(sample_rate)),
            active_voice: 0,
            saturation: Saturation::new(sample_rate),
            master_eq: MasterEq::new(),
            drift: Drift::new(),
            sample_rate,
            pending: [PendingHit::default(); PENDING_RING_SIZE],
            rng_state: 0xA5A5_A5A5,
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        for v in &mut self.voices {
            v.set_sample_rate(sample_rate);
        }
        self.saturation = Saturation::new(sample_rate);
        self.master_eq = MasterEq::new();
    }

    /// Push a hit into the first free ring slot. Silently drops if the ring
    /// is full (pathological — 12 slots covers 4 hits × 3 overlapping steps,
    /// which is beyond musically reasonable).
    fn push_pending_internal(&mut self, samples_until: u32, velocity: f32) {
        for slot in &mut self.pending {
            if !slot.live {
                slot.samples_until = samples_until;
                slot.velocity = velocity;
                slot.live = true;
                return;
            }
        }
    }

    /// Advance the ring by one sample. Writes the velocities of any hits
    /// that fire this sample into `out`, returning how many were written.
    fn tick_pending_internal(&mut self, out: &mut [f32; PENDING_RING_SIZE]) -> usize {
        let mut count = 0;
        for slot in &mut self.pending {
            if !slot.live {
                continue;
            }
            if slot.samples_until == 0 {
                out[count] = slot.velocity;
                count += 1;
                slot.live = false;
            } else {
                slot.samples_until -= 1;
            }
        }
        count
    }

    /// xorshift32 — Marsaglia's 2003 variant. Produces a non-zero u32 as long
    /// as the seed is non-zero. Fast, branchless, zero-alloc.
    fn rng_xorshift32(&mut self) -> u32 {
        let mut x = self.rng_state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng_state = x;
        x
    }

    /// Centered, symmetric f32 in [-1.0, 1.0) derived from the PRNG. Used to
    /// scale humanize timing and velocity jitter.
    fn rng_centered(&mut self) -> f32 {
        let u = self.rng_xorshift32();
        (u as f32 / 2_147_483_648.0) - 1.0
    }

    /// Schedule a group of 1–4 hits into the pending ring with sample-accurate
    /// offsets relative to "now" (offset 0 fires on the next `tick_pending`).
    ///
    /// * `n_hits` — 1, 2, 3, or 4. Anything else is clamped to 1..=4.
    /// * `gap_samples` — base inter-stroke gap in samples. Ignored when `n_hits == 1`.
    /// * `humanize` — 0.0 (exact) to 1.0 (full jitter). Scales both timing
    ///   jitter (±20%×humanize) and velocity jitter (±10%×humanize).
    /// * `step_velocity` — per-step velocity multiplier applied on top of the
    ///   per-hit base amplitude.
    pub fn schedule_group(
        &mut self,
        n_hits: usize,
        gap_samples: u32,
        humanize: f32,
        step_velocity: f32,
    ) {
        let n = n_hits.clamp(1, 4);
        let base_vels: &[f32] = match n {
            1 => &[1.0],
            2 => &[0.7, 1.0],
            3 => &[0.7, 0.85, 1.0],
            4 => &[0.6, 0.75, 0.85, 1.0],
            _ => unreachable!(),
        };

        let humanize = humanize.clamp(0.0, 1.0);
        for (i, &bv) in base_vels.iter().enumerate() {
            let base_offset = (i as u32).saturating_mul(gap_samples);

            let offset = if humanize > 0.0 && gap_samples > 0 && i > 0 {
                let j = self.rng_centered() * 0.2 * humanize * gap_samples as f32;
                let signed = base_offset as i64 + j as i64;
                signed.max(0) as u32
            } else {
                base_offset
            };

            let vel = if humanize > 0.0 {
                let j = self.rng_centered() * 0.1 * humanize;
                (bv * step_velocity * (1.0 + j)).clamp(0.0, 2.0)
            } else {
                bv * step_velocity
            };

            self.push_pending_internal(offset, vel);
        }
    }

    #[cfg(test)]
    pub fn push_pending(&mut self, samples_until: u32, velocity: f32) {
        self.push_pending_internal(samples_until, velocity);
    }

    #[cfg(test)]
    pub fn tick_pending_for_test(&mut self) -> Vec<f32> {
        let mut out = [0.0f32; PENDING_RING_SIZE];
        let n = self.tick_pending_internal(&mut out);
        out[..n].to_vec()
    }

    #[cfg(test)]
    pub fn seed_rng_for_test(&mut self, seed: u32) {
        self.rng_state = if seed == 0 { 0xA5A5_A5A5 } else { seed };
    }

    #[cfg(test)]
    pub fn rng_u32_for_test(&mut self) -> u32 {
        self.rng_xorshift32()
    }

    pub fn trigger(&mut self, params: &KickParams, velocity: f32) {
        // Voice stealing: if the currently-active slot is still audible,
        // start its fadeout and flip to the other slot so the new hit lands
        // on clean state. The old voice keeps ticking in parallel during
        // the ~5 ms crossfade, summed through the shared saturation/EQ.
        if self.voices[self.active_voice].is_active() {
            self.voices[self.active_voice].start_fadeout(self.sample_rate);
            self.active_voice = (self.active_voice + 1) % NUM_VOICES;

            // Pathological case: the "other" slot is also still audible
            // (a 3rd trigger arrives within the fadeout window of the
            // previous one). Fade it too, so restarting its state below
            // happens from a near-silent baseline.
            if self.voices[self.active_voice].is_active() {
                self.voices[self.active_voice].start_fadeout(self.sample_rate);
            }
        }
        self.voices[self.active_voice].trigger(
            params,
            velocity,
            &mut self.drift,
            self.sample_rate,
        );
    }

    pub fn process(
        &mut self,
        output_left: &mut [f32],
        output_right: &mut [f32],
        params: &KickParams,
    ) -> f32 {
        if !self.is_active() {
            return 0.0;
        }

        // Update EQ coefficients once per buffer
        self.master_eq.update(
            self.sample_rate,
            &EqParams {
                tilt_db: params.eq_tilt_db,
                low_boost_db: params.eq_low_boost_db,
                notch_freq: params.eq_notch_freq,
                notch_q: params.eq_notch_q,
                notch_depth_db: params.eq_notch_depth_db,
            },
        );

        let sat_mode = SatMode::from_u8(params.sat_mode);
        let mut peak = 0.0f32;
        let mut fired_buf = [0.0f32; PENDING_RING_SIZE];

        for (l, r) in output_left.iter_mut().zip(output_right.iter_mut()) {
            // Fire any scheduled hits whose countdown reached zero this sample.
            let n_fired = self.tick_pending_internal(&mut fired_buf);
            for i in 0..n_fired {
                let vel = fired_buf[i];
                if self.voices[self.active_voice].is_active() {
                    self.voices[self.active_voice].start_fadeout(self.sample_rate);
                    self.active_voice = (self.active_voice + 1) % NUM_VOICES;
                    if self.voices[self.active_voice].is_active() {
                        self.voices[self.active_voice].start_fadeout(self.sample_rate);
                    }
                }
                self.voices[self.active_voice].trigger(
                    params,
                    vel,
                    &mut self.drift,
                    self.sample_rate,
                );
            }

            // Sum all voices (some may be fading out).
            let mut mixed = 0.0f32;
            for v in &mut self.voices {
                mixed += v.tick(params);
            }

            // Saturation
            mixed = self
                .saturation
                .process(mixed, sat_mode, params.sat_drive, params.sat_mix);

            // Master EQ
            mixed = self.master_eq.process(mixed);

            // Master gain
            mixed *= params.master_gain;

            peak = peak.max(mixed.abs());
            *l += mixed;
            *r += mixed;
        }

        peak
    }

    /// Whether any voice is still producing audio, or a scheduled hit is
    /// waiting to fire.
    pub fn is_active(&self) -> bool {
        self.voices.iter().any(|v| v.is_active())
            || self.pending.iter().any(|p| p.live)
    }
}

/// All parameters needed by the engine for one process() call.
#[derive(Clone, Copy)]
pub struct KickParams {
    pub master_gain: f32,
    pub decay_ms: f32,
    pub velocity_sens: f32,

    // SUB
    pub sub_gain: f32,
    pub sub_fstart: f32,
    pub sub_fend: f32,
    pub sub_sweep_ms: f32,
    pub sub_sweep_curve: f32,
    pub sub_phase_offset: f32,

    // MID
    pub mid_gain: f32,
    pub mid_fstart: f32,
    pub mid_fend: f32,
    pub mid_sweep_ms: f32,
    pub mid_sweep_curve: f32,
    pub mid_phase_offset: f32,
    pub mid_decay_ms: f32,
    pub mid_tone_gain: f32,
    pub mid_noise_gain: f32,
    pub mid_noise_color: f32,

    // TOP
    pub top_gain: f32,
    pub top_decay_ms: f32,
    pub top_freq: f32,
    pub top_bw: f32,

    // Saturation
    pub sat_mode: u8,
    pub sat_drive: f32,
    pub sat_mix: f32,

    // Drift
    pub drift_amount: f32,

    // EQ
    pub eq_tilt_db: f32,
    pub eq_low_boost_db: f32,
    pub eq_notch_freq: f32,
    pub eq_notch_q: f32,
    pub eq_notch_depth_db: f32,
}

impl Default for KickParams {
    fn default() -> Self {
        Self {
            master_gain: 1.0,
            decay_ms: 400.0,
            velocity_sens: 0.8,

            sub_gain: 0.85,
            sub_fstart: 150.0,
            sub_fend: 45.0,
            sub_sweep_ms: 60.0,
            sub_sweep_curve: 3.0,
            sub_phase_offset: std::f32::consts::FRAC_PI_2,

            mid_gain: 0.5,
            mid_fstart: 400.0,
            mid_fend: 120.0,
            mid_sweep_ms: 30.0,
            mid_sweep_curve: 4.0,
            mid_phase_offset: std::f32::consts::FRAC_PI_2,
            mid_decay_ms: 150.0,
            mid_tone_gain: 0.7,
            mid_noise_gain: 0.3,
            mid_noise_color: 0.4,

            top_gain: 0.25,
            top_decay_ms: 6.0,
            top_freq: 3500.0,
            top_bw: 1.5,

            sat_mode: 0,
            sat_drive: 0.0,
            sat_mix: 1.0,

            drift_amount: 0.0,

            eq_tilt_db: 0.0,
            eq_low_boost_db: 0.0,
            eq_notch_freq: 250.0,
            eq_notch_q: 0.0,
            eq_notch_depth_db: 12.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigger_produces_nonzero_output() {
        let mut engine = KickEngine::new(44100.0);
        let params = KickParams::default();
        engine.trigger(&params, 1.0);
        let mut left = vec![0.0f32; 512];
        let mut right = vec![0.0f32; 512];
        let peak = engine.process(&mut left, &mut right, &params);
        assert!(peak > 0.01, "expected audible output, got peak {}", peak);
    }

    #[test]
    fn decays_to_silence() {
        let mut engine = KickEngine::new(44100.0);
        let params = KickParams {
            decay_ms: 100.0,
            mid_decay_ms: 50.0,
            ..KickParams::default()
        };
        engine.trigger(&params, 1.0);
        let mut left = vec![0.0f32; 22050];
        let mut right = vec![0.0f32; 22050];
        engine.process(&mut left, &mut right, &params);
        assert!(!engine.is_active(), "engine should be inactive after decay");
    }

    #[test]
    fn velocity_zero_is_quiet() {
        let mut engine = KickEngine::new(44100.0);
        let params = KickParams {
            velocity_sens: 1.0,
            ..KickParams::default()
        };
        engine.trigger(&params, 0.0);
        let mut left = vec![0.0f32; 512];
        let mut right = vec![0.0f32; 512];
        engine.process(&mut left, &mut right, &params);
        let max = left.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!(max < 0.001, "expected silence with velocity 0, got {}", max);
    }

    #[test]
    fn retrigger_no_panic() {
        let mut engine = KickEngine::new(44100.0);
        let params = KickParams::default();
        engine.trigger(&params, 1.0);
        let mut left = vec![0.0f32; 64];
        let mut right = vec![0.0f32; 64];
        engine.process(&mut left, &mut right, &params);
        engine.trigger(&params, 0.8);
        left.fill(0.0);
        right.fill(0.0);
        engine.process(&mut left, &mut right, &params);
        assert!(engine.is_active());
    }

    #[test]
    fn saturation_adds_harmonics() {
        let mut engine = KickEngine::new(44100.0);
        let mut params = KickParams {
            mid_gain: 0.0,
            top_gain: 0.0,
            sat_mode: 1, // SoftClip
            sat_drive: 0.8,
            ..KickParams::default()
        };
        engine.trigger(&params, 1.0);
        let mut left_sat = vec![0.0f32; 512];
        let mut right_sat = vec![0.0f32; 512];
        engine.process(&mut left_sat, &mut right_sat, &params);

        let mut engine2 = KickEngine::new(44100.0);
        params.sat_mode = 0; // Off
        engine2.trigger(&params, 1.0);
        let mut left_dry = vec![0.0f32; 512];
        let mut right_dry = vec![0.0f32; 512];
        engine2.process(&mut left_dry, &mut right_dry, &params);

        // Saturated signal should differ from dry
        let diff: f32 = left_sat
            .iter()
            .zip(left_dry.iter())
            .map(|(a, b)| (a - b).abs())
            .sum();
        assert!(
            diff > 0.1,
            "saturation should change the signal, diff={}",
            diff
        );
    }

    /// Scan `samples` for the maximum absolute sample-to-sample delta.
    /// A true click appears as a single-sample jump much larger than the
    /// surrounding slope. A 150 Hz sine at full amplitude has a max
    /// per-sample delta of ~0.021 at 44.1 kHz, so anything above ~0.15 is
    /// solidly a discontinuity.
    fn max_abs_delta(samples: &[f32]) -> (usize, f32) {
        let mut max = 0.0f32;
        let mut idx = 0;
        for i in 1..samples.len() {
            let d = (samples[i] - samples[i - 1]).abs();
            if d > max {
                max = d;
                idx = i;
            }
        }
        (idx, max)
    }

    #[test]
    fn fully_decayed_retrigger_is_identical_to_first() {
        // User-reported repro: first hit is clean, every subsequent hit has
        // a "ghost attack" on top — even when decay is short enough that
        // the previous voice is fully silent before the next trigger.
        // This test compares the first N samples of a fresh trigger against
        // the first N samples of a second trigger fired after complete
        // decay. They should be bit-identical (or near it) because the
        // engine state should be fully reset to the same starting point.
        let params = KickParams {
            decay_ms: 30.0,
            mid_decay_ms: 20.0,
            top_decay_ms: 6.0,
            ..KickParams::default()
        };
        let mut engine = KickEngine::new(48000.0);

        // First hit: render 128 samples.
        let mut l1 = vec![0.0f32; 128];
        let mut r1 = vec![0.0f32; 128];
        engine.trigger(&params, 1.0);
        engine.process(&mut l1, &mut r1, &params);

        // Render enough samples for ALL envelopes to fully decay.
        let mut l_decay = vec![0.0f32; 48000];
        let mut r_decay = vec![0.0f32; 48000];
        engine.process(&mut l_decay, &mut r_decay, &params);
        assert!(
            !engine.is_active(),
            "engine should be fully silent before the retrigger test"
        );

        // Second hit: render 128 samples from the same engine.
        let mut l2 = vec![0.0f32; 128];
        let mut r2 = vec![0.0f32; 128];
        engine.trigger(&params, 1.0);
        engine.process(&mut l2, &mut r2, &params);

        // The first 128 samples of both hits should match closely. Any
        // divergence indicates stale state leaking between triggers —
        // which is the "ghost attack" the user is hearing.
        let mut max_diff = 0.0f32;
        let mut max_idx = 0;
        for i in 0..128 {
            let d = (l1[i] - l2[i]).abs();
            if d > max_diff {
                max_diff = d;
                max_idx = i;
            }
        }
        assert!(
            max_diff < 0.005,
            "fresh triggers should be nearly identical; \
             max diff {} at sample {} (first={}, second={})",
            max_diff,
            max_idx,
            l1[max_idx],
            l2[max_idx],
        );
    }

    #[test]
    fn retrigger_scan_no_discontinuity() {
        // Tight scan: render a fresh hit, splice in a retrigger mid-decay,
        // and look for any sample-to-sample jump across the full retrigger
        // transition. This catches clicks that a single-sample continuity
        // check can miss (multi-sample transient, filter ringing, etc.).
        let mut engine = KickEngine::new(44100.0);
        let params = KickParams::default();

        // Render 2048 samples of a clean single hit as the baseline slope
        // reference — we only care about the max per-sample delta here.
        let mut l_ref = vec![0.0f32; 2048];
        let mut r_ref = vec![0.0f32; 2048];
        engine.trigger(&params, 1.0);
        engine.process(&mut l_ref, &mut r_ref, &params);
        let (_, ref_max) = max_abs_delta(&l_ref);

        // Now render a retrigger scenario in a fresh engine: trigger,
        // process some samples, trigger again, process a long window that
        // covers the full 5 ms crossfade and then some.
        let mut engine = KickEngine::new(44100.0);
        let mut l = vec![0.0f32; 2048];
        let mut r = vec![0.0f32; 2048];
        engine.trigger(&params, 1.0);
        // Run the first hit for 128 samples (well inside decay, past
        // attack ramp).
        engine.process(&mut l[..128], &mut r[..128], &params);
        // Retrigger and render the transition window.
        engine.trigger(&params, 1.0);
        engine.process(&mut l[128..], &mut r[128..], &params);

        let (idx, jump) = max_abs_delta(&l);
        // The retrigger transition must not introduce a delta meaningfully
        // larger than what a clean hit produces by itself.
        assert!(
            jump <= ref_max * 1.5 + 0.05,
            "retrigger discontinuity at idx {} (jump={}, ref_max={})",
            idx,
            jump,
            ref_max,
        );
    }

    #[test]
    fn retrigger_while_active_has_no_click() {
        // Reproduces the sequencer retrigger bug: when a new hit lands while
        // the previous one is still decaying through the signal chain, the
        // transition across the retrigger sample must not be a step
        // discontinuity.
        let mut engine = KickEngine::new(44100.0);
        let params = KickParams::default();
        engine.trigger(&params, 1.0);

        // Process well into the decay (~0.7 ms) but before the attack ramp
        // has fully finished on a fresh hit, so we're sampling the steady
        // decay tail.
        let mut left = vec![0.0f32; 128];
        let mut right = vec![0.0f32; 128];
        engine.process(&mut left, &mut right, &params);
        let prev = left[127];
        assert!(prev.abs() > 0.01, "expected nonzero decay tail, got {}", prev);

        // Retrigger, then process a single sample.
        engine.trigger(&params, 1.0);
        let mut l1 = vec![0.0f32; 1];
        let mut r1 = vec![0.0f32; 1];
        engine.process(&mut l1, &mut r1, &params);

        // The jump from the last decay sample to the first post-retrigger
        // sample must stay within the magnitude that a clean hit's first
        // sample can reach — prior to the fix this jump was large enough
        // (filter-state-reset + envelope drop + phase snap) to sound like
        // a click layered on top of the kick.
        let jump = (l1[0] - prev).abs();
        assert!(
            jump < 0.15,
            "retrigger discontinuity: prev={} new={} jump={}",
            prev,
            l1[0],
            jump
        );
    }

    #[test]
    fn no_click_on_trigger() {
        let mut engine = KickEngine::new(44100.0);
        let params = KickParams::default();
        engine.trigger(&params, 1.0);
        let mut left = vec![0.0f32; 8];
        let mut right = vec![0.0f32; 8];
        engine.process(&mut left, &mut right, &params);
        // First sample should be small (attack ramp), not a full-amplitude pop
        assert!(
            left[0].abs() < 0.1,
            "first sample too loud (click): {}",
            left[0]
        );
    }

    // ---- Flam: pending ring + RNG ----

    #[test]
    fn pending_ring_fires_hits_in_order() {
        let mut eng = KickEngine::new(48000.0);
        eng.push_pending(0, 0.7);
        eng.push_pending(720, 0.85);
        eng.push_pending(1440, 1.0);

        let mut fired: Vec<(usize, f32)> = Vec::new();
        for sample in 0..1500 {
            for v in eng.tick_pending_for_test() {
                fired.push((sample, v));
            }
        }
        assert_eq!(fired.len(), 3, "should fire exactly 3 hits, got {fired:?}");
        assert_eq!(fired[0].0, 0);
        assert!((fired[0].1 - 0.7).abs() < 1e-6);
        assert_eq!(fired[1].0, 720);
        assert!((fired[1].1 - 0.85).abs() < 1e-6);
        assert_eq!(fired[2].0, 1440);
        assert!((fired[2].1 - 1.0).abs() < 1e-6);
    }

    #[test]
    fn pending_ring_ignores_dead_slots() {
        let mut eng = KickEngine::new(48000.0);
        for _ in 0..2000 {
            let _ = eng.tick_pending_for_test();
        }
        let mut count = 0;
        for _ in 0..10 {
            count += eng.tick_pending_for_test().len();
        }
        assert_eq!(count, 0, "idle ring should fire nothing");
    }

    #[test]
    fn xorshift32_never_zero_and_deterministic() {
        let mut eng = KickEngine::new(48000.0);
        eng.seed_rng_for_test(0xDEAD_BEEF);
        let a: Vec<u32> = (0..8).map(|_| eng.rng_u32_for_test()).collect();
        eng.seed_rng_for_test(0xDEAD_BEEF);
        let b: Vec<u32> = (0..8).map(|_| eng.rng_u32_for_test()).collect();
        assert_eq!(a, b, "same seed must produce same sequence");
        assert!(a.iter().all(|&x| x != 0));
    }

    // ---- Flam: schedule_group ----

    #[test]
    fn schedule_group_flam_two_hits() {
        let mut eng = KickEngine::new(48000.0);
        let gap = 720;
        eng.schedule_group(2, gap, 0.0, 1.0);
        let mut fired: Vec<(usize, f32)> = Vec::new();
        for sample in 0..2000 {
            for v in eng.tick_pending_for_test() {
                fired.push((sample, v));
            }
        }
        assert_eq!(fired.len(), 2);
        assert_eq!(fired[0].0, 0);
        assert!((fired[0].1 - 0.7).abs() < 1e-6);
        assert_eq!(fired[1].0, gap as usize);
        assert!((fired[1].1 - 1.0).abs() < 1e-6);
    }

    #[test]
    fn schedule_group_ruff_three_hits() {
        let mut eng = KickEngine::new(48000.0);
        let gap = 480;
        eng.schedule_group(3, gap, 0.0, 1.0);
        let mut fired: Vec<(usize, f32)> = Vec::new();
        for sample in 0..2000 {
            for v in eng.tick_pending_for_test() {
                fired.push((sample, v));
            }
        }
        assert_eq!(fired.len(), 3);
        assert_eq!(fired[0].0, 0);
        assert_eq!(fired[1].0, gap as usize);
        assert_eq!(fired[2].0, (2 * gap) as usize);
        let expected = [0.7, 0.85, 1.0];
        for (a, e) in fired.iter().zip(expected.iter()) {
            assert!((a.1 - e).abs() < 1e-6);
        }
    }

    #[test]
    fn schedule_group_roll_four_hits() {
        let mut eng = KickEngine::new(48000.0);
        let gap = 480;
        eng.schedule_group(4, gap, 0.0, 1.0);
        let mut fired: Vec<(usize, f32)> = Vec::new();
        for sample in 0..3000 {
            for v in eng.tick_pending_for_test() {
                fired.push((sample, v));
            }
        }
        assert_eq!(fired.len(), 4);
        for (i, f) in fired.iter().enumerate() {
            assert_eq!(f.0, i * gap as usize);
        }
        let expected = [0.6, 0.75, 0.85, 1.0];
        for (a, e) in fired.iter().zip(expected.iter()) {
            assert!((a.1 - e).abs() < 1e-6);
        }
    }

    #[test]
    fn schedule_group_single_hit_is_immediate() {
        let mut eng = KickEngine::new(48000.0);
        eng.schedule_group(1, 480, 0.0, 0.9);
        let fired = eng.tick_pending_for_test();
        assert_eq!(fired, vec![0.9]);
    }

    #[test]
    fn humanize_one_bounded_timing_and_velocity_jitter() {
        let mut eng = KickEngine::new(48000.0);
        eng.seed_rng_for_test(42);
        let gap: u32 = 720;
        let mut max_t_dev = 0i64;
        let mut max_v_dev = 0f32;
        for _ in 0..200 {
            eng.schedule_group(4, gap, 1.0, 1.0);
            let mut fired: Vec<(usize, f32)> = Vec::new();
            for sample in 0..4000 {
                for v in eng.tick_pending_for_test() {
                    fired.push((sample, v));
                }
            }
            assert_eq!(fired.len(), 4);
            // First hit is anchored at offset 0 (no timing jitter). Check hit 1.
            let t_dev = fired[1].0 as i64 - gap as i64;
            let v_dev = (fired[0].1 - 0.6).abs();
            max_t_dev = max_t_dev.max(t_dev.abs());
            max_v_dev = max_v_dev.max(v_dev);
        }
        assert!(max_t_dev <= 150, "t dev {max_t_dev} exceeds 150 samples");
        assert!(max_v_dev <= 0.07, "v dev {max_v_dev} exceeds 0.07");
    }

    #[test]
    fn humanize_zero_is_exact_regardless_of_seed() {
        let mut eng = KickEngine::new(48000.0);
        eng.seed_rng_for_test(12345);
        for _ in 0..50 {
            let _ = eng.rng_u32_for_test();
        }
        eng.schedule_group(3, 480, 0.0, 1.0);
        let mut fired: Vec<(usize, f32)> = Vec::new();
        for sample in 0..2000 {
            for v in eng.tick_pending_for_test() {
                fired.push((sample, v));
            }
        }
        assert_eq!(fired.len(), 3);
        assert_eq!(fired[0].0, 0);
        assert_eq!(fired[1].0, 480);
        assert_eq!(fired[2].0, 960);
        let expected = [0.7, 0.85, 1.0];
        for (a, e) in fired.iter().zip(expected.iter()) {
            assert!((a.1 - e).abs() < 1e-6);
        }
    }

    #[test]
    fn schedule_group_fires_hits_via_process() {
        let mut eng = KickEngine::new(48000.0);
        let params = KickParams::default();
        eng.schedule_group(2, 480, 0.0, 1.0);
        let mut left = vec![0.0f32; 1200];
        let mut right = vec![0.0f32; 1200];
        eng.process(&mut left, &mut right, &params);

        let rms = |slice: &[f32]| -> f32 {
            (slice.iter().map(|x| x * x).sum::<f32>() / slice.len() as f32).sqrt()
        };
        let rms_hit0 = rms(&left[0..64]);
        let rms_hit1 = rms(&left[480..544]);
        assert!(rms_hit0 > 1e-4, "hit 0 window silent: {rms_hit0}");
        assert!(rms_hit1 > 1e-4, "hit 1 window silent: {rms_hit1}");
    }

    #[test]
    fn four_voice_pool_handles_roll() {
        let mut eng = KickEngine::new(48000.0);
        let params = KickParams::default();
        eng.schedule_group(4, 480, 0.0, 1.0);
        let mut left = vec![0.0f32; 4800];
        let mut right = vec![0.0f32; 4800];
        eng.process(&mut left, &mut right, &params);
        let total_energy: f32 = left.iter().map(|x| x * x).sum();
        assert!(total_energy > 0.01, "roll produced near-zero signal: {total_energy}");
    }
}
