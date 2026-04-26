use crate::dsp::clap::ClapVoice;
use crate::dsp::click::ClickGen;
use crate::dsp::drift::Drift;
use crate::dsp::envelope::{AmpEnvelope, PitchEnvelope};
use crate::dsp::filter::{EqParams, MasterEq};
use crate::dsp::noise::NoiseGen;
use crate::dsp::oscillator::SineOsc;
use crate::dsp::saturation::{SatMode, Saturation};
use crate::dsp::voice_clip;

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
/// and is marked dead.
#[derive(Copy, Clone, Debug, Default)]
struct PendingHit {
    samples_until: u32,
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
    /// Independent amp envelope for the MID noise channel. Real 909 kicks
    /// have a short noise BURST gated to attack (~15-30 ms), distinct from
    /// the tone's longer tail. Legacy slammer ran noise off `mid_amp_env`,
    /// which made noise sustain for as long as the tone — too "hissy" on
    /// long-decay presets. With its own envelope, noise can stay short
    /// while the tone keeps its tail.
    mid_noise_amp_env: AmpEnvelope,
    mid_noise: NoiseGen,
    // TOP
    top_click: ClickGen,
    top_amp_env: AmpEnvelope,
    metal_phase: f32,
    sample_rate: f32,
    /// Voice-level output multiplier. Normally 1.0. When this voice is
    /// stolen, `fadeout_step` is set and `fadeout_gain` decreases linearly
    /// each sample until it reaches 0, at which point the voice is dead.
    fadeout_gain: f32,
    fadeout_step: f32,
    /// True once any generator in this voice has been triggered; gates the
    /// early-exit in `KickEngine::process()`.
    triggered: bool,
    /// Per-trigger amplitude jitter (≈±2.5% at full drift). Multiplied into
    /// the voice's tick output AFTER the layer mix and BEFORE fadeout, so a
    /// single random value perturbs the whole hit's level. 1.0 when
    /// `drift_amount` is 0 — preserves deterministic v0.5.x behavior.
    amp_scale: f32,
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
            mid_noise_amp_env: AmpEnvelope::new(sample_rate),
            mid_noise: NoiseGen::new(sample_rate),
            top_click: ClickGen::new(sample_rate),
            top_amp_env: AmpEnvelope::new(sample_rate),
            metal_phase: 0.0,
            sample_rate,
            fadeout_gain: 1.0,
            fadeout_step: 0.0,
            triggered: false,
            amp_scale: 1.0,
        }
    }

    fn set_sample_rate(&mut self, sample_rate: f32) {
        *self = Self::new(sample_rate);
    }

    /// Voice is still producing audio: one of its generators has state AND
    /// its fadeout hasn't fully killed it. The noise envelope is checked
    /// even though it's typically the shortest — a user-tuned long-noise
    /// preset shouldn't get cut off if it outlasts the tone envelope.
    fn is_active(&self) -> bool {
        if !self.triggered {
            return false;
        }
        if self.fadeout_gain <= 0.0 {
            return false;
        }
        self.sub_amp_env.is_active()
            || self.mid_amp_env.is_active()
            || self.mid_noise_amp_env.is_active()
            || self.top_click.is_active()
    }

    fn trigger(
        &mut self,
        params: &KickParams,
        drift: &mut Drift,
        sample_rate: f32,
    ) {
        self.fadeout_gain = 1.0;
        self.fadeout_step = 0.0;
        self.triggered = true;

        // Analog drift — three axes, all gated by the same `drift_amount`:
        //  - pitch:  per-layer (sub/mid sample independently for a tiny detune)
        //  - amp:    per-trigger (one value scales the whole voice's output)
        //  - decay:  per-trigger (one value scales every amp envelope's tau)
        // Phase stays deterministic so identical hits keep identical low end.
        let sub_pf = drift.pitch_jitter(params.drift_amount);
        let mid_pf = drift.pitch_jitter(params.drift_amount);
        let env_drift = drift.sample_envelope(params.drift_amount);
        let mut amp_scale = env_drift.amp_scale;
        let mut decay_scale = env_drift.decay_scale;

        // Accent: 909-style velocity boost. Lifts amplitude moderately and
        // extends the decay so accented hits cut through the mix. Both
        // multipliers compose with the drift values above so accented hits
        // still get their per-trigger drift jitter on top.
        if params.accent && params.accent_amount > 0.0 {
            let a = params.accent_amount.min(1.0);
            amp_scale *= 1.0 + 0.3 * a;
            decay_scale *= 1.0 + 0.5 * a;
        }
        self.amp_scale = amp_scale;

        // SUB
        self.sub_pitch_env.trigger(
            params.sub_fstart * sub_pf,
            params.sub_fend * sub_pf,
            params.sub_sweep_ms / 1000.0,
            params.sub_sweep_curve,
        );
        self.sub_amp_env
            .trigger(params.decay_ms * decay_scale, params.drift_amount);
        self.sub_osc.trigger(params.sub_phase_offset);

        // MID
        self.mid_pitch_env.trigger(
            params.mid_fstart * mid_pf,
            params.mid_fend * mid_pf,
            params.mid_sweep_ms / 1000.0,
            params.mid_sweep_curve,
        );
        self.mid_amp_env
            .trigger(params.mid_decay_ms * decay_scale, params.drift_amount);
        // Noise gets its own short envelope (gated to attack). Legacy
        // presets that omit `mid_noise_decay_ms` deserialize as 0.0 — a
        // value < 1.0 would crash the noise instantly via the AmpEnvelope
        // tau formula. Treat anything below 1 ms as the legacy "sustained"
        // sentinel and fall back to the tone's decay.
        let noise_decay_ms = if params.mid_noise_decay_ms >= 1.0 {
            params.mid_noise_decay_ms
        } else {
            params.mid_decay_ms
        };
        self.mid_noise_amp_env
            .trigger(noise_decay_ms * decay_scale, params.drift_amount);
        self.mid_osc.trigger(params.mid_phase_offset);
        self.mid_noise.trigger();

        // TOP
        self.top_click.regenerate(
            sample_rate,
            params.top_decay_ms,
            params.top_freq,
            params.top_bw,
        );
        self.top_click.trigger();
        self.top_amp_env
            .trigger(params.top_decay_ms * decay_scale, params.drift_amount);
        self.metal_phase = 0.0;
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

        // SUB: sine with pitch sweep + amp envelope. Per-voice soft-clip
        // sits BEFORE the VCA (amp env), matching 909 architecture
        // (VCO → clipper → VCA). At kick_clip_mode=0 / drive=0 the
        // shaper is bit-identical pass-through, so v0.5.x behavior is
        // preserved for every existing preset.
        let sub_freq = self.sub_pitch_env.tick();
        let sub_amp = self.sub_amp_env.tick();
        let sub_raw = self.sub_osc.tick(sub_freq) * params.sub_gain;
        let sub_shaped = voice_clip::apply(
            params.kick_clip_mode,
            params.kick_clip_drive,
            sub_raw,
        );
        let sub = sub_shaped * sub_amp;

        // MID: sine path and noise path are now SEPARATE — tone goes
        // through pitch env + voice-clip + tone amp env, noise goes
        // through its own short attack-gated env. This matches how a
        // 909 actually generates the mid layer (tone VCO into clipper
        // into VCA, noise generator into its own short VCA in parallel).
        // Without the split, raising mid_noise_gain bled hiss into the
        // entire tail; with it, noise contributes only the snap.
        let mid_freq = self.mid_pitch_env.tick();
        let mid_amp = self.mid_amp_env.tick();
        let mid_noise_amp = self.mid_noise_amp_env.tick();
        let mid_tone_raw = self.mid_osc.tick(mid_freq) * params.mid_tone_gain * params.mid_gain;
        let mid_tone_shaped = voice_clip::apply(
            params.kick_clip_mode,
            params.kick_clip_drive,
            mid_tone_raw,
        );
        let mid_noise_raw =
            self.mid_noise.tick(params.mid_noise_color) * params.mid_noise_gain * params.mid_gain;
        let mid = mid_tone_shaped * mid_amp + mid_noise_raw * mid_noise_amp;

        // TOP: click transient with optional metallic ring modulation
        let top_raw = self.top_click.tick();
        let top_amp = self.top_amp_env.tick();
        let top = if params.top_metal > 0.001 {
            let mod_freq = params.top_freq * 2.4142;
            let mod_out = (self.metal_phase * std::f32::consts::TAU).sin();
            self.metal_phase += mod_freq / self.sample_rate;
            if self.metal_phase >= 1.0 {
                self.metal_phase -= self.metal_phase.floor();
            }
            let ring = 1.0 + params.top_metal * mod_out;
            top_raw * ring * params.top_gain * top_amp
        } else {
            top_raw * params.top_gain * top_amp
        };

        // Advance fadeout BEFORE computing the sample. With the previous
        // post-compute order, the final sample of a fading voice was
        // multiplied by ~`fadeout_step` (≈ 0.004 at 5 ms / 48 kHz) instead
        // of zero — which on a sustained voice produced a -54 dB residual
        // step at the boundary, audible as an occasional tic on
        // back-to-back retriggers (voice stealing). Decrementing first
        // means the last computed sample lands at exactly fadeout_gain=0
        // and the transition to the dead voice is silent.
        if self.fadeout_step > 0.0 {
            self.fadeout_gain -= self.fadeout_step;
            if self.fadeout_gain <= 0.0 {
                self.fadeout_gain = 0.0;
                self.triggered = false;
            }
        }

        (sub + mid + top) * self.amp_scale * self.fadeout_gain
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
    /// 909-style clap voice. Triggered in parallel with the kick when
    /// `params.clap_on` is true, mixed into the output before saturation
    /// and EQ so the clap goes through the same mastering chain.
    clap: ClapVoice,
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
            clap: ClapVoice::new(sample_rate),
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        for v in &mut self.voices {
            v.set_sample_rate(sample_rate);
        }
        self.saturation = Saturation::new(sample_rate);
        self.master_eq = MasterEq::new();
        self.clap = ClapVoice::new(sample_rate);
    }

    /// Push a hit into the first free ring slot. Silently drops if the ring
    /// is full (pathological — 12 slots covers 4 hits × 3 overlapping steps,
    /// which is beyond musically reasonable).
    #[cfg(test)]
    fn push_pending_internal(&mut self, samples_until: u32) {
        for slot in &mut self.pending {
            if !slot.live {
                slot.samples_until = samples_until;
                slot.live = true;
                return;
            }
        }
    }

    /// Advance the ring by one sample. Returns the number of hits that fired
    /// this sample (caller should fire one engine trigger per fired hit).
    fn tick_pending_internal(&mut self) -> usize {
        let mut count = 0;
        for slot in &mut self.pending {
            if !slot.live {
                continue;
            }
            if slot.samples_until == 0 {
                count += 1;
                slot.live = false;
            } else {
                slot.samples_until -= 1;
            }
        }
        count
    }

    #[cfg(test)]
    pub fn push_pending(&mut self, samples_until: u32) {
        self.push_pending_internal(samples_until);
    }

    #[cfg(test)]
    pub fn tick_pending_for_test(&mut self) -> usize {
        self.tick_pending_internal()
    }

    pub fn trigger(&mut self, params: &KickParams) {
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
            &mut self.drift,
            self.sample_rate,
        );

        if params.clap_on {
            self.clap.set_params(
                self.sample_rate,
                params.clap_freq,
                params.clap_tail_ms,
            );
            self.clap.trigger();
        }
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

        for (l, r) in output_left.iter_mut().zip(output_right.iter_mut()) {
            // Fire any scheduled hits whose countdown reached zero this sample.
            let n_fired = self.tick_pending_internal();
            for _ in 0..n_fired {
                if self.voices[self.active_voice].is_active() {
                    self.voices[self.active_voice].start_fadeout(self.sample_rate);
                    self.active_voice = (self.active_voice + 1) % NUM_VOICES;
                    if self.voices[self.active_voice].is_active() {
                        self.voices[self.active_voice].start_fadeout(self.sample_rate);
                    }
                }
                self.voices[self.active_voice].trigger(
                    params,
                    &mut self.drift,
                    self.sample_rate,
                );
            }

            // Sum all voices (some may be fading out).
            let mut mixed = 0.0f32;
            for v in &mut self.voices {
                mixed += v.tick(params);
            }

            // Clap layer: summed pre-saturation so drive/EQ shape it too.
            if params.clap_on {
                mixed += self.clap.tick() * params.clap_level;
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

    /// Whether any voice is still producing audio, a scheduled hit is
    /// waiting to fire, or the clap layer is still ringing.
    pub fn is_active(&self) -> bool {
        self.voices.iter().any(|v| v.is_active())
            || self.pending.iter().any(|p| p.live)
            || self.clap.is_active()
    }
}

/// All parameters needed by the engine for one process() call.
#[derive(Clone, Copy)]
pub struct KickParams {
    pub master_gain: f32,
    pub decay_ms: f32,

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
    /// Decay time for the MID noise channel's own amp envelope. Below
    /// 1.0 ms acts as a legacy sentinel meaning "sustain with the tone"
    /// (i.e. fall back to `mid_decay_ms`) — this lets v0.5.x preset JSON
    /// deserialize without instantly muting noise on load.
    pub mid_noise_decay_ms: f32,

    // TOP
    pub top_gain: f32,
    pub top_decay_ms: f32,
    pub top_freq: f32,
    pub top_bw: f32,
    pub top_metal: f32,

    // Saturation (master-bus, post-envelope)
    pub sat_mode: u8,
    pub sat_drive: f32,
    pub sat_mix: f32,

    // Per-voice soft-clip (pre-amp-envelope, in `KickVoice::tick`). 0 = Off.
    // See `dsp::voice_clip` for mode constants.
    pub kick_clip_mode: u8,
    pub kick_clip_drive: f32,

    // Drift
    pub drift_amount: f32,

    // Accent — 909-style velocity boost. `accent` is a per-trigger flag set
    // by `plugin.rs` from the sequencer's accent bits (false for manual
    // triggers). `accent_amount` is the host param scaling how much accent
    // lifts a hit. At `accent_amount = 0` the flag has no effect, so
    // existing presets stay deterministic.
    pub accent: bool,
    pub accent_amount: f32,

    // EQ
    pub eq_tilt_db: f32,
    pub eq_low_boost_db: f32,
    pub eq_notch_freq: f32,
    pub eq_notch_q: f32,
    pub eq_notch_depth_db: f32,

    // CLAP
    pub clap_on: bool,
    pub clap_level: f32,
    pub clap_freq: f32,
    pub clap_tail_ms: f32,
}

impl Default for KickParams {
    fn default() -> Self {
        Self {
            master_gain: 1.0,
            decay_ms: 400.0,

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
            mid_noise_decay_ms: 30.0,

            top_gain: 0.25,
            top_decay_ms: 6.0,
            top_freq: 3500.0,
            top_bw: 1.5,
            top_metal: 0.0,

            sat_mode: 0,
            sat_drive: 0.0,
            sat_mix: 1.0,

            kick_clip_mode: 0,
            kick_clip_drive: 0.0,

            drift_amount: 0.0,

            accent: false,
            accent_amount: 0.0,

            eq_tilt_db: 0.0,
            eq_low_boost_db: 0.0,
            eq_notch_freq: 250.0,
            eq_notch_q: 0.0,
            eq_notch_depth_db: 12.0,

            clap_on: false,
            clap_level: 0.9,
            clap_freq: 1200.0,
            clap_tail_ms: 180.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sub_phase_deterministic_at_max_drift() {
        // With drift_amount=1.0, two identical triggers must produce identical
        // SUB-layer phase on sample 0. Isolate the sub by zeroing mid/top/clap.
        let params = KickParams {
            drift_amount: 1.0,
            mid_gain: 0.0,
            top_gain: 0.0,
            clap_on: false,
            ..KickParams::default()
        };
        let mut e1 = KickEngine::new(44100.0);
        let mut e2 = KickEngine::new(44100.0);
        e1.trigger(&params);
        e2.trigger(&params);
        let (mut l1, mut r1) = (vec![0.0f32; 1], vec![0.0f32; 1]);
        let (mut l2, mut r2) = (vec![0.0f32; 1], vec![0.0f32; 1]);
        e1.process(&mut l1, &mut r1, &params);
        e2.process(&mut l2, &mut r2, &params);
        assert_eq!(
            l1[0], l2[0],
            "sub phase must be deterministic on trigger regardless of drift"
        );
    }

    #[test]
    fn trigger_produces_nonzero_output() {
        let mut engine = KickEngine::new(44100.0);
        let params = KickParams::default();
        engine.trigger(&params);
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
        engine.trigger(&params);
        let mut left = vec![0.0f32; 22050];
        let mut right = vec![0.0f32; 22050];
        engine.process(&mut left, &mut right, &params);
        assert!(!engine.is_active(), "engine should be inactive after decay");
    }

    #[test]
    fn zero_drift_is_deterministic_across_triggers() {
        // Regression guard: at drift_amount=0, every trigger must produce
        // identical samples. amp_scale and decay_scale must collapse to
        // exactly 1.0 so v0.5.x users don't experience any change in their
        // existing presets. Voice stealing means the second trigger lands
        // in a different slot, so we capture the first hit's tail before
        // retriggering and compare the early-attack windows separately.
        let params = KickParams {
            drift_amount: 0.0,
            ..KickParams::default()
        };
        let mut engine = KickEngine::new(44100.0);
        engine.trigger(&params);
        let mut l1 = vec![0.0f32; 256];
        let mut r1 = vec![0.0f32; 256];
        engine.process(&mut l1, &mut r1, &params);

        let mut engine2 = KickEngine::new(44100.0);
        engine2.trigger(&params);
        let mut l2 = vec![0.0f32; 256];
        let mut r2 = vec![0.0f32; 256];
        engine2.process(&mut l2, &mut r2, &params);

        for i in 0..256 {
            assert_eq!(
                l1[i], l2[i],
                "drift_amount=0 must be deterministic at sample {i}"
            );
        }
    }

    #[test]
    fn accent_no_op_when_amount_is_zero() {
        // The per-step accent flag must stay inert when the host param
        // `accent_amount` is 0 — guarantees v0.5.x preset behavior is
        // unchanged regardless of where accent flags happen to be set.
        let p_dry = KickParams {
            accent: false,
            accent_amount: 0.0,
            ..KickParams::default()
        };
        let p_flag = KickParams {
            accent: true,
            accent_amount: 0.0,
            ..KickParams::default()
        };
        let mut e1 = KickEngine::new(44100.0);
        let mut e2 = KickEngine::new(44100.0);
        e1.trigger(&p_dry);
        e2.trigger(&p_flag);
        let mut l1 = vec![0.0f32; 256];
        let mut r1 = vec![0.0f32; 256];
        let mut l2 = vec![0.0f32; 256];
        let mut r2 = vec![0.0f32; 256];
        e1.process(&mut l1, &mut r1, &p_dry);
        e2.process(&mut l2, &mut r2, &p_flag);
        for i in 0..256 {
            assert_eq!(
                l1[i], l2[i],
                "accent flag must not affect output at amount=0 (sample {i})"
            );
        }
    }

    #[test]
    fn accent_increases_peak_amplitude() {
        // With non-trivial accent_amount, an accented hit should peak
        // measurably higher than an unaccented one. Catches a regression
        // where the boost is wired but `accent_amount` plumbing breaks
        // (stuck at 0 in collect_kick_params or similar).
        let p_dry = KickParams {
            accent: false,
            accent_amount: 1.0,
            sat_mode: 0, // bypass master saturation so we measure raw level
            ..KickParams::default()
        };
        let p_acc = KickParams {
            accent: true,
            accent_amount: 1.0,
            sat_mode: 0,
            ..KickParams::default()
        };
        let mut e1 = KickEngine::new(44100.0);
        let mut e2 = KickEngine::new(44100.0);
        e1.trigger(&p_dry);
        e2.trigger(&p_acc);
        let mut l1 = vec![0.0f32; 1024];
        let mut r1 = vec![0.0f32; 1024];
        let mut l2 = vec![0.0f32; 1024];
        let mut r2 = vec![0.0f32; 1024];
        e1.process(&mut l1, &mut r1, &p_dry);
        e2.process(&mut l2, &mut r2, &p_acc);
        let p1 = l1.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
        let p2 = l2.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
        assert!(
            p2 > p1 * 1.15,
            "accent at amount=1.0 should boost peak ≥15% (dry {p1} vs accented {p2})"
        );
    }

    #[test]
    fn short_noise_decay_silences_noise_before_tone() {
        // With mid_tone muted and only noise enabled, a 5 ms noise envelope
        // should die well before a 250 ms test window ends. Uses sub also
        // muted so we measure noise in isolation. Catches a regression where
        // the noise envelope is silently routed off `mid_amp_env` again.
        let params = KickParams {
            sub_gain: 0.0,
            mid_gain: 1.0,
            mid_tone_gain: 0.0,
            mid_noise_gain: 0.5,
            mid_decay_ms: 250.0,
            mid_noise_decay_ms: 5.0,
            top_gain: 0.0,
            sat_mode: 0,
            ..KickParams::default()
        };
        let mut engine = KickEngine::new(44100.0);
        engine.trigger(&params);

        // Sample at ~50 ms — well past the 5 ms noise decay (~10 tau).
        let early_n = (0.050 * 44100.0) as usize;
        let mut early_l = vec![0.0f32; early_n];
        let mut early_r = vec![0.0f32; early_n];
        engine.process(&mut early_l, &mut early_r, &params);
        // Then sample for another 50 ms — by here the noise should be
        // effectively silent (<-60 dB peak); previous "sustained" code
        // would still have audible hiss because mid_amp_env hasn't decayed.
        let late_n = (0.050 * 44100.0) as usize;
        let mut late_l = vec![0.0f32; late_n];
        let mut late_r = vec![0.0f32; late_n];
        engine.process(&mut late_l, &mut late_r, &params);

        let early_peak = early_l.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
        let late_peak = late_l.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
        assert!(
            early_peak > 0.005,
            "noise should be audible during attack (early peak {early_peak})"
        );
        assert!(
            late_peak < early_peak * 0.05,
            "noise should be ≥26 dB down by 100 ms with 5 ms decay (late peak {late_peak} vs early {early_peak})"
        );
    }

    #[test]
    fn legacy_noise_decay_falls_back_to_tone_decay() {
        // Old preset JSONs deserialize `mid_noise_decay_ms` to 0.0. Feeding
        // that to AmpEnvelope::trigger directly would crash the noise
        // instantly (tau ≈ 0). The trigger path treats anything < 1 ms as
        // a sentinel and reuses `mid_decay_ms`, so a legacy preset's noise
        // should still be audible well into the tail.
        let legacy = KickParams {
            sub_gain: 0.0,
            mid_gain: 1.0,
            mid_tone_gain: 0.0,
            mid_noise_gain: 0.5,
            mid_decay_ms: 200.0,
            mid_noise_decay_ms: 0.0, // legacy default
            top_gain: 0.0,
            sat_mode: 0,
            ..KickParams::default()
        };
        let mut engine = KickEngine::new(44100.0);
        engine.trigger(&legacy);
        let n = (0.080 * 44100.0) as usize; // 80 ms in
        let mut l = vec![0.0f32; n];
        let mut r = vec![0.0f32; n];
        engine.process(&mut l, &mut r, &legacy);
        // Take the last few ms — should still have measurable noise content.
        let tail_start = n.saturating_sub((0.005 * 44100.0) as usize);
        let tail_peak = l[tail_start..]
            .iter()
            .fold(0.0f32, |a, &b| a.max(b.abs()));
        assert!(
            tail_peak > 0.005,
            "legacy noise (decay_ms=0 → mid_decay fallback) silenced too early; tail_peak {tail_peak}"
        );
    }

    #[test]
    fn voice_clip_off_matches_pre_clip_baseline() {
        // The whole point of opting in to voice-clip is that legacy presets
        // don't change. Two engines, identical params, one with explicit
        // kick_clip_mode=0 and one with kick_clip_drive=0 — both must produce
        // bit-identical output.
        let p_off = KickParams {
            kick_clip_mode: 0,
            kick_clip_drive: 0.5, // drive without mode is still off
            ..KickParams::default()
        };
        let p_zero_drive = KickParams {
            kick_clip_mode: 1, // tanh, but...
            kick_clip_drive: 0.0, // ...drive=0 short-circuits to identity
            ..KickParams::default()
        };
        let mut e1 = KickEngine::new(44100.0);
        let mut e2 = KickEngine::new(44100.0);
        e1.trigger(&p_off);
        e2.trigger(&p_zero_drive);
        let mut l1 = vec![0.0f32; 256];
        let mut r1 = vec![0.0f32; 256];
        let mut l2 = vec![0.0f32; 256];
        let mut r2 = vec![0.0f32; 256];
        e1.process(&mut l1, &mut r1, &p_off);
        e2.process(&mut l2, &mut r2, &p_zero_drive);
        for i in 0..256 {
            assert_eq!(
                l1[i], l2[i],
                "voice clip off-path should be bit-identical at sample {i}"
            );
        }
    }

    #[test]
    fn voice_clip_changes_output_when_engaged() {
        // Sanity: with the clip engaged at non-trivial drive, the engine's
        // sample stream must measurably differ from the unclipped baseline.
        // Catches a regression where the clip is wired but param plumbing
        // fails to reach `voice_clip::apply` (mode/drive stuck at 0).
        let p_dry = KickParams {
            kick_clip_mode: 0,
            kick_clip_drive: 0.0,
            ..KickParams::default()
        };
        let p_clipped = KickParams {
            kick_clip_mode: 1, // Tanh
            kick_clip_drive: 0.8,
            ..KickParams::default()
        };
        let mut e1 = KickEngine::new(44100.0);
        let mut e2 = KickEngine::new(44100.0);
        e1.trigger(&p_dry);
        e2.trigger(&p_clipped);
        let mut l1 = vec![0.0f32; 512];
        let mut r1 = vec![0.0f32; 512];
        let mut l2 = vec![0.0f32; 512];
        let mut r2 = vec![0.0f32; 512];
        e1.process(&mut l1, &mut r1, &p_dry);
        e2.process(&mut l2, &mut r2, &p_clipped);
        let diff: f32 = l1.iter().zip(l2.iter()).map(|(a, b)| (a - b).abs()).sum();
        assert!(
            diff > 0.5,
            "engaged voice clip should measurably alter output (sum-abs diff = {diff})"
        );
    }

    #[test]
    fn drift_amp_scale_varies_consecutive_triggers() {
        // At drift_amount=1.0, the new amp_scale per-trigger jitter must
        // make consecutive triggers measurably differ in peak amplitude.
        // Guards against amp_scale being silently dropped from the tick
        // chain. Use two fresh engines with fresh LCG state so the test
        // exercises sequential drift samples (not engine-to-engine parity).
        let params = KickParams {
            drift_amount: 1.0,
            mid_gain: 0.0,
            top_gain: 0.0,
            clap_on: false,
            ..KickParams::default()
        };
        let mut engine = KickEngine::new(44100.0);
        engine.trigger(&params);
        let mut l1 = vec![0.0f32; 1024];
        let mut r1 = vec![0.0f32; 1024];
        engine.process(&mut l1, &mut r1, &params);

        // Wait long enough for the first hit to fully decay so voice
        // stealing's fadeout doesn't muddle the peak comparison.
        let mut tail_l = vec![0.0f32; 22050];
        let mut tail_r = vec![0.0f32; 22050];
        engine.process(&mut tail_l, &mut tail_r, &params);

        engine.trigger(&params);
        let mut l2 = vec![0.0f32; 1024];
        let mut r2 = vec![0.0f32; 1024];
        engine.process(&mut l2, &mut r2, &params);

        let peak1 = l1.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
        let peak2 = l2.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
        assert!(peak1 > 0.01 && peak2 > 0.01, "expected audible peaks");
        assert!(
            (peak1 - peak2).abs() > 1e-5,
            "consecutive triggers at full drift should differ in peak ({peak1} vs {peak2})"
        );
    }

    #[test]
    fn retrigger_no_panic() {
        let mut engine = KickEngine::new(44100.0);
        let params = KickParams::default();
        engine.trigger(&params);
        let mut left = vec![0.0f32; 64];
        let mut right = vec![0.0f32; 64];
        engine.process(&mut left, &mut right, &params);
        engine.trigger(&params);
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
        engine.trigger(&params);
        let mut left_sat = vec![0.0f32; 512];
        let mut right_sat = vec![0.0f32; 512];
        engine.process(&mut left_sat, &mut right_sat, &params);

        let mut engine2 = KickEngine::new(44100.0);
        params.sat_mode = 0; // Off
        engine2.trigger(&params);
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
        engine.trigger(&params);
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
        engine.trigger(&params);
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
        engine.trigger(&params);
        engine.process(&mut l_ref, &mut r_ref, &params);
        let (_, ref_max) = max_abs_delta(&l_ref);

        // Now render a retrigger scenario in a fresh engine: trigger,
        // process some samples, trigger again, process a long window that
        // covers the full 5 ms crossfade and then some.
        let mut engine = KickEngine::new(44100.0);
        let mut l = vec![0.0f32; 2048];
        let mut r = vec![0.0f32; 2048];
        engine.trigger(&params);
        // Run the first hit for 128 samples (well inside decay, past
        // attack ramp).
        engine.process(&mut l[..128], &mut r[..128], &params);
        // Retrigger and render the transition window.
        engine.trigger(&params);
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
        engine.trigger(&params);

        // Process well into the decay (~0.7 ms) but before the attack ramp
        // has fully finished on a fresh hit, so we're sampling the steady
        // decay tail.
        let mut left = vec![0.0f32; 128];
        let mut right = vec![0.0f32; 128];
        engine.process(&mut left, &mut right, &params);
        let prev = left[127];
        assert!(prev.abs() > 0.01, "expected nonzero decay tail, got {}", prev);

        // Retrigger, then process a single sample.
        engine.trigger(&params);
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
        engine.trigger(&params);
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

    // ---- Pending ring (sequencer scheduling) ----

    #[test]
    fn pending_ring_fires_hits_in_order() {
        let mut eng = KickEngine::new(48000.0);
        eng.push_pending(0);
        eng.push_pending(720);
        eng.push_pending(1440);

        let mut fired: Vec<usize> = Vec::new();
        for sample in 0..1500 {
            let n = eng.tick_pending_for_test();
            for _ in 0..n {
                fired.push(sample);
            }
        }
        assert_eq!(fired.len(), 3, "should fire exactly 3 hits, got {fired:?}");
        assert_eq!(fired[0], 0);
        assert_eq!(fired[1], 720);
        assert_eq!(fired[2], 1440);
    }

    #[test]
    fn pending_ring_ignores_dead_slots() {
        let mut eng = KickEngine::new(48000.0);
        for _ in 0..2000 {
            let _ = eng.tick_pending_for_test();
        }
        let mut count = 0;
        for _ in 0..10 {
            count += eng.tick_pending_for_test();
        }
        assert_eq!(count, 0, "idle ring should fire nothing");
    }

    // ---- Metal ring mod ----

    #[test]
    fn metal_zero_is_identical_to_no_metal() {
        let params = KickParams::default();
        assert!(params.top_metal < 0.001);
        let mut e1 = KickEngine::new(48000.0);
        e1.trigger(&params);
        let mut l1 = vec![0.0f32; 1024];
        let mut r1 = vec![0.0f32; 1024];
        e1.process(&mut l1, &mut r1, &params);

        let mut e2 = KickEngine::new(48000.0);
        e2.trigger(&params);
        let mut l2 = vec![0.0f32; 1024];
        let mut r2 = vec![0.0f32; 1024];
        e2.process(&mut l2, &mut r2, &params);
        assert_eq!(l1, l2, "metal=0 must be deterministic / bit-identical");
    }

    #[test]
    fn metal_changes_top_output() {
        let params_no = KickParams {
            top_metal: 0.0,
            top_gain: 1.0,
            mid_gain: 0.0,
            sub_gain: 0.0,
            ..KickParams::default()
        };
        let params_yes = KickParams {
            top_metal: 0.8,
            ..params_no
        };

        let mut e1 = KickEngine::new(48000.0);
        e1.trigger(&params_no);
        let mut l1 = vec![0.0f32; 512];
        let mut r1 = vec![0.0f32; 512];
        e1.process(&mut l1, &mut r1, &params_no);

        let mut e2 = KickEngine::new(48000.0);
        e2.trigger(&params_yes);
        let mut l2 = vec![0.0f32; 512];
        let mut r2 = vec![0.0f32; 512];
        e2.process(&mut l2, &mut r2, &params_yes);

        let diff: f32 = l1.iter().zip(l2.iter()).map(|(a, b)| (a - b).abs()).sum();
        assert!(diff > 0.1, "metal should change the click character, diff={diff}");
    }

    // ---- Clap layer ----

    #[test]
    fn engine_clap_silent_when_disabled() {
        let mut eng = KickEngine::new(48000.0);
        let params = KickParams {
            clap_on: false,
            ..KickParams::default()
        };
        eng.trigger(&params);
        let mut l = vec![0.0f32; 4096];
        let mut r = vec![0.0f32; 4096];
        eng.process(&mut l, &mut r, &params);
        // baseline kick-only output
        let baseline: Vec<f32> = l.clone();

        let mut eng2 = KickEngine::new(48000.0);
        eng2.trigger(&params);
        let mut l2 = vec![0.0f32; 4096];
        let mut r2 = vec![0.0f32; 4096];
        eng2.process(&mut l2, &mut r2, &params);
        assert_eq!(baseline, l2, "clap off must be deterministic kick-only");
    }

    #[test]
    fn engine_clap_adds_output_when_enabled() {
        let off_params = KickParams {
            clap_on: false,
            ..KickParams::default()
        };
        let on_params = KickParams {
            clap_on: true,
            ..KickParams::default()
        };

        let mut e_off = KickEngine::new(48000.0);
        e_off.trigger(&off_params);
        let mut l_off = vec![0.0f32; 16384];
        let mut r_off = vec![0.0f32; 16384];
        e_off.process(&mut l_off, &mut r_off, &off_params);

        let mut e_on = KickEngine::new(48000.0);
        e_on.trigger(&on_params);
        let mut l_on = vec![0.0f32; 16384];
        let mut r_on = vec![0.0f32; 16384];
        e_on.process(&mut l_on, &mut r_on, &on_params);

        // After the kick body has decayed (late in the buffer) the clap
        // tail should still be contributing energy — off buffer is silent,
        // on buffer is not.
        let tail = |s: &[f32]| -> f32 {
            (s[8000..10000]
                .iter()
                .map(|x| x * x)
                .sum::<f32>()
                / 2000.0)
                .sqrt()
        };
        let off_rms = tail(&l_off);
        let on_rms = tail(&l_on);
        assert!(
            on_rms > off_rms + 1e-5,
            "clap on should add tail energy: off={off_rms} on={on_rms}"
        );
    }

    /// Scan engine output for sample-to-sample jumps that would be audible as
    /// clicks. Covers default params, clap-on, and metal-on configurations.
    #[test]
    fn no_discontinuities_in_output() {
        let configs: Vec<(&str, KickParams)> = vec![
            ("default", KickParams::default()),
            (
                "clap_on",
                KickParams {
                    clap_on: true,
                    clap_level: 0.5,
                    ..KickParams::default()
                },
            ),
            (
                "metal",
                KickParams {
                    top_metal: 0.8,
                    top_gain: 0.7,
                    ..KickParams::default()
                },
            ),
            (
                "clap+metal",
                KickParams {
                    clap_on: true,
                    clap_level: 0.5,
                    top_metal: 0.6,
                    top_gain: 0.7,
                    ..KickParams::default()
                },
            ),
        ];

        for (name, params) in &configs {
            let mut engine = KickEngine::new(48000.0);
            engine.trigger(params);
            let n = 8192;
            let mut left = vec![0.0f32; n];
            let mut right = vec![0.0f32; n];
            engine.process(&mut left, &mut right, params);

            // Skip sample 0 (always ~0 due to attack ramp).
            let (idx, jump) = max_abs_delta(&left[1..]);
            assert!(
                jump < 0.5,
                "[{name}] discontinuity at sample {}: delta = {jump:.4}",
                idx + 1
            );

            // Also verify no NaN/inf snuck through.
            for (i, &s) in left.iter().enumerate() {
                assert!(s.is_finite(), "[{name}] non-finite at sample {i}: {s}");
            }
        }
    }
}
