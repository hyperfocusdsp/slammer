//! The `Plugin` impl for Niner — glues together parameters, DSP engine,
//! telemetry, and the egui editor. Parameter definitions themselves live in
//! [`crate::params`]; DSP is in [`crate::dsp`].

use nih_plug::prelude::*;
use parking_lot::Mutex;
use std::sync::Arc;

use crate::dsp::dj_filter::DjFilter;
use crate::dsp::engine::KickEngine;
use crate::dsp::master_bus::MasterBus;
use crate::dsp::spectrum::SpectrumAnalyzer;
use crate::dsp::tube::TubeWarmth;
use crate::logging;
use crate::params::{collect_kick_params, NinerParams};
use crate::presets::PresetManager;
use crate::sequencer::{self, Sequencer};
use crate::util::messages::{self, UiToDsp};
use crate::util::telemetry::{self, MeterShared, SpectrumShared, TelemetryProducer};

use std::sync::atomic::Ordering;

/// Final-stage safety clipper applied per-sample after master volume,
/// before the output buffer. Signals below `SC_THRESHOLD` pass through
/// unchanged (bit-identical passthrough for normal-loudness material);
/// signals above roll off smoothly via `tanh` and asymptote to
/// `SC_CEILING` without ever reaching it. This prevents the DAC from
/// hard-clipping when a preset pushes internal gain above 0 dBFS and
/// the user's macro-comp / limiter is disengaged (the default state,
/// where the entire master-bus chain is bypassed per the gate in the
/// per-sample loop below).
const SC_THRESHOLD: f32 = 0.85;
const SC_CEILING: f32 = 0.999;
const SC_KNEE: f32 = SC_CEILING - SC_THRESHOLD;

#[inline(always)]
fn soft_clip_safety(x: f32) -> f32 {
    let a = x.abs();
    if a < SC_THRESHOLD {
        x
    } else {
        let over = a - SC_THRESHOLD;
        x.signum() * (SC_THRESHOLD + SC_KNEE * (over / SC_KNEE).tanh())
    }
}

pub struct Niner {
    params: Arc<NinerParams>,
    engine: KickEngine,
    master_bus: MasterBus,
    /// Master-volume-driven tube warmth stage. Engages automatically when
    /// the master volume knob is pushed past 0 dB; silent (bit-identical
    /// bypass) below unity gain.
    tube_warmth: TubeWarmth,
    dj_filter: DjFilter,
    sample_rate: f32,
    pub telemetry_tx: Option<TelemetryProducer>,
    pub telemetry_rx_holder: Option<telemetry::TelemetryConsumer>,
    /// Lock-free GR meter state shared with the editor.
    pub meter_shared: Arc<MeterShared>,
    /// Lock-free spectrum-bin state shared with the editor (64 log-spaced
    /// dB bands). Audio thread writes on FFT completion; GUI reads per frame.
    pub spectrum_shared: Arc<SpectrumShared>,
    /// FFT analyzer owned by the audio thread. Fed every sample; publishes
    /// to `spectrum_shared` once per FFT_SIZE samples.
    spectrum: SpectrumAnalyzer,
    /// UI → DSP ring. The audio thread owns the consumer; the editor thread
    /// takes the producer when `editor()` is called. Wrapped in `Mutex<Option>`
    /// purely so ownership can move once at editor init — it is never locked
    /// on the audio thread.
    pub ui_tx_holder: Arc<Mutex<Option<rtrb::Producer<UiToDsp>>>>,
    ui_rx: rtrb::Consumer<UiToDsp>,
    pub preset_manager: Arc<Mutex<PresetManager>>,
    /// 16-step pattern sequencer shared with the editor.
    pub sequencer: Arc<Sequencer>,
    /// Audio-thread-owned sample counter within the current step (standalone mode).
    seq_sample_counter: u64,
    /// Audio-thread-owned current step index (mirrored to `sequencer.current_step`).
    seq_current_step: usize,
    /// True once transport.playing has ever reported false — used to distinguish
    /// a real DAW (which stops/starts) from a standalone backend (which always
    /// reports playing=true).
    host_ever_stopped: bool,
    /// Last step index fired in host-sync mode, for edge detection across buffers.
    last_host_step: Option<usize>,
    /// Previous buffer's `sequencer.running` value — used to detect the
    /// play-toggle rising edge and reset to step 1.
    seq_running_prev: bool,
    /// Glitch-bisect knob, set once in `initialize()` from the
    /// `NINER_DISABLE_SPECTRUM` env var. When true, the audio thread
    /// skips the spectrum analyzer's `feed_sample` call (which runs an
    /// FFT every 1024 samples). Lets us A/B test whether the FFT is the
    /// source of the v0.6.0 RT-only crunchiness — Autokit on the same
    /// box has no audio-thread FFT and is glitch-free.
    spectrum_disabled: bool,
}

impl Default for Niner {
    fn default() -> Self {
        let (telem_tx, telem_rx) = telemetry::channel();
        let (ui_tx, ui_rx) = messages::channel();
        let params = Arc::new(NinerParams::default());
        let sequencer = Arc::new(Sequencer::new(
            Arc::clone(&params.seq_steps),
            Arc::clone(&params.seq_accents),
        ));
        Self {
            params,
            engine: KickEngine::new(44100.0),
            master_bus: MasterBus::new(),
            tube_warmth: TubeWarmth::new(),
            dj_filter: DjFilter::new(),
            sample_rate: 44100.0,
            telemetry_tx: Some(telem_tx),
            telemetry_rx_holder: Some(telem_rx),
            meter_shared: MeterShared::new(),
            spectrum_shared: SpectrumShared::new(),
            spectrum: SpectrumAnalyzer::new(44100.0),
            ui_tx_holder: Arc::new(Mutex::new(Some(ui_tx))),
            ui_rx,
            preset_manager: Arc::new(Mutex::new(PresetManager::new())),
            sequencer,
            seq_sample_counter: 0,
            seq_current_step: 0,
            host_ever_stopped: false,
            last_host_step: None,
            seq_running_prev: false,
            // Real value populated in `initialize()` from the env var so
            // the standalone honors it. VST3/CLAP hosts honor it too if the
            // user set the variable in the DAW launch environment.
            spectrum_disabled: false,
        }
    }
}

impl Plugin for Niner {
    const NAME: &'static str = "Niner";
    const VENDOR: &'static str = "Hyperfocus DSP";
    const URL: &'static str = "https://hyperfocusdsp.com";
    const EMAIL: &'static str = "hello@hyperfocusdsp.com";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    const MIDI_INPUT: MidiConfig = MidiConfig::Basic;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::None;
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        tracing::info!("editor() called");
        let params = Arc::clone(&self.params);
        let telemetry_rx = self.telemetry_rx_holder.take();
        let ui_tx = self.ui_tx_holder.lock().take();
        let preset_manager = Arc::clone(&self.preset_manager);
        let sequencer = Arc::clone(&self.sequencer);
        let meter = Arc::clone(&self.meter_shared);
        let spectrum = Arc::clone(&self.spectrum_shared);
        crate::ui::editor::create(
            self.params.editor_state.clone(),
            params,
            telemetry_rx,
            ui_tx,
            preset_manager,
            sequencer,
            meter,
            spectrum,
        )
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.sample_rate = buffer_config.sample_rate;
        logging::init();
        tracing::info!(
            "Niner v{} initialized — sr: {}",
            Self::VERSION,
            self.sample_rate
        );
        self.engine.set_sample_rate(self.sample_rate);
        self.master_bus.prepare(self.sample_rate);
        self.dj_filter.set_sample_rate(self.sample_rate);
        // Recompute log-spaced FFT band edges for the new rate and clear the
        // ring so stale samples from a different rate don't leak into the
        // next spectrum.
        self.spectrum.set_sample_rate(self.sample_rate);
        // nih-plug has already deserialized `params.seq_steps` at this
        // point; copy the bitmask into the sequencer atomics so the first
        // `process()` call sees the restored pattern.
        self.sequencer.restore_from_persist();

        // Bisect knob for the v0.6.0 glitch hunt. Anything set to a
        // non-empty, non-"0" value disables the audio-thread FFT.
        self.spectrum_disabled = std::env::var("NINER_DISABLE_SPECTRUM")
            .map(|v| !v.is_empty() && v != "0")
            .unwrap_or(false);
        if self.spectrum_disabled {
            tracing::warn!(
                "NINER_DISABLE_SPECTRUM is set — audio-thread spectrum FFT is OFF. \
                 Spectrum display will not update."
            );
        }
        true
    }

    fn reset(&mut self) {
        // Clear compressor/limiter state so a DAW play/stop cycle doesn't
        // carry over envelope history across a seek.
        self.master_bus.prepare(self.sample_rate);
        self.dj_filter.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let num_samples = buffer.samples();
        let kick_params = collect_kick_params(&self.params);

        // Drain UI → DSP messages (lock-free, non-blocking).
        while let Ok(msg) = self.ui_rx.pop() {
            match msg {
                UiToDsp::Trigger => {
                    self.engine.trigger(&kick_params);
                }
            }
        }

        while let Some(event) = context.next_event() {
            if let NoteEvent::NoteOn { .. } = event {
                self.engine.trigger(&kick_params);
            }
        }

        // --- Sequencer advancement ---
        // Standalone backends (CPAL, JACK via PipeWire) always report
        // playing=true; real DAW hosts toggle transport on/off. Track whether
        // we've ever seen !playing to distinguish the two cases.
        let transport = context.transport();
        if !transport.playing {
            self.host_ever_stopped = true;
        }
        let is_daw = self.host_ever_stopped;
        self.sequencer.host_synced.store(is_daw, Ordering::Relaxed);
        // Signal the editor that host_synced now reflects reality.
        self.sequencer
            .transport_probed
            .store(true, Ordering::Relaxed);

        if is_daw {
            // ----- Host-synced mode -----
            // Follow the DAW: playing iff transport.playing, step position
            // derived from transport.pos_beats() (16ths = beats * 4). User's
            // PLAY button and standalone BPM are ignored here.
            let host_bpm = transport.tempo.unwrap_or(120.0) as f32;
            self.sequencer.set_display_bpm(host_bpm);
            self.sequencer
                .running_effective
                .store(transport.playing, Ordering::Relaxed);

            if transport.playing {
                if let Some(pos_beats) = transport.pos_beats() {
                    let step_float = pos_beats * 4.0;
                    let new_step = (step_float.floor() as i64)
                        .rem_euclid(sequencer::STEPS as i64)
                        as usize;
                    if self.last_host_step != Some(new_step) {
                        self.last_host_step = Some(new_step);
                        self.sequencer
                            .current_step
                            .store(new_step, Ordering::Relaxed);
                        if self.sequencer.steps[new_step].load(Ordering::Relaxed) {
                            let mut p = collect_kick_params(&self.params);
                            p.accent = self.sequencer.is_step_accented(new_step);
                            self.engine.trigger(&p);
                        }
                    }
                }
            } else {
                self.last_host_step = None;
            }
            // Reset standalone counter so a DAW stop/rewind doesn't carry over.
            self.seq_sample_counter = 0;
        } else {
            // ----- Standalone mode -----
            // Free-running at the user's BPM, gated by the PLAY button.
            // 16th-note step length. At 120 BPM @ 48 kHz this is 6000 samples,
            // well above any typical buffer size, so buffer-granular timing
            // is comfortably within one step of jitter.
            let bpm = self.sequencer.bpm();
            self.sequencer.set_display_bpm(bpm);
            let running = self.sequencer.running.load(Ordering::Relaxed);
            self.sequencer
                .running_effective
                .store(running, Ordering::Relaxed);

            // Rising edge: user just hit play — restart from step 1 and
            // trigger it immediately so the first hit lands under the finger.
            if running && !self.seq_running_prev {
                self.seq_sample_counter = 0;
                self.seq_current_step = 0;
                self.sequencer.current_step.store(0, Ordering::Relaxed);
                if self.sequencer.steps[0].load(Ordering::Relaxed) {
                    let mut p = collect_kick_params(&self.params);
                    p.accent = self.sequencer.is_step_accented(0);
                    self.engine.trigger(&p);
                }
            }
            self.seq_running_prev = running;

            if running {
                let samples_per_step = ((60.0 / bpm / 4.0) * self.sample_rate) as u64;
                self.seq_sample_counter += num_samples as u64;
                while self.seq_sample_counter >= samples_per_step {
                    self.seq_sample_counter -= samples_per_step;
                    self.seq_current_step = (self.seq_current_step + 1) % sequencer::STEPS;
                    self.sequencer
                        .current_step
                        .store(self.seq_current_step, Ordering::Relaxed);
                    if self.sequencer.steps[self.seq_current_step].load(Ordering::Relaxed) {
                        let mut p = collect_kick_params(&self.params);
                        p.accent = self.sequencer.is_step_accented(self.seq_current_step);
                        self.engine.trigger(&p);
                    }
                }
            } else {
                self.seq_sample_counter = 0;
            }
        }

        let channels = buffer.as_slice();
        if channels.len() >= 2 {
            let (left_channels, right_channels) = channels.split_at_mut(1);
            let output_left = &mut left_channels[0][..num_samples];
            let output_right = &mut right_channels[0][..num_samples];
            output_left.fill(0.0);
            output_right.fill(0.0);

            // Synth + saturation + EQ + per-voice master_gain runs inside
            // the engine. Its reported peak is pre-comp / pre-master and is
            // intentionally ignored — the telemetry ring is fed post-comp
            // below so the OUTPUT waveform reflects the compressed signal.
            let _engine_peak = self.engine.process(output_left, output_right, &kick_params);

            // Sanitize engine output — if a voice produced NaN/inf (e.g.
            // from an unstable filter), clamp to zero so the downstream
            // comp / filter / warmth state doesn't get permanently
            // corrupted. Without this, one bad sample kills audio until
            // the plugin is reloaded.
            for s in output_left.iter_mut().chain(output_right.iter_mut()) {
                if !s.is_finite() {
                    *s = 0.0;
                }
            }

            // Snapshot bypass-able state once per buffer. Smoothed macro
            // params are pulled inside the per-sample loop.
            let limiter_on = self.params.comp_limit_on.value();
            let sr = self.sample_rate;

            let mut post_peak = 0.0f32;
            let mut max_gr_db = 0.0f32;
            let mut nan_detected = false;

            for (l, r) in output_left.iter_mut().zip(output_right.iter_mut()) {
                // Smoothed pulls — cheap scalar arithmetic.
                let amount = self.params.comp_amount.smoothed.next();
                // comp_react's smoothed value is pulled to keep its
                // smoother in lockstep with host automation, but the DSP
                // no longer reads it directly — RCT is now a UI-side
                // "link" that writes into comp_atk_ms / comp_rel_ms.
                let _ = self.params.comp_react.smoothed.next();
                let drive = self.params.comp_drive.smoothed.next();
                let atk_ms = self.params.comp_atk_ms.smoothed.next();
                let rel_ms = self.params.comp_rel_ms.smoothed.next();
                let knee_db = self.params.comp_knee_db.smoothed.next();
                let master_gain = self.params.master_volume.smoothed.next();
                let filt_pos = self.params.dj_filter_pos.smoothed.next();
                let filt_res = self.params.dj_filter_res.smoothed.next();
                let filt_pre = self.params.dj_filter_pre.value();

                let threshold_db = -6.0 + amount * -24.0;
                let ratio = 2.0 + amount * 8.0;

                self.master_bus.set_times(atk_ms, rel_ms, sr);

                // DJ Filter PRE: before comp
                let (pre_l, pre_r) = if filt_pre {
                    self.dj_filter.process_sample(*l, *r, filt_pos, filt_res)
                } else {
                    (*l, *r)
                };

                let (cl, cr) = if amount > 0.0001 || drive > 0.001 || limiter_on {
                    self.master_bus.process_sample(
                        pre_l,
                        pre_r,
                        threshold_db,
                        ratio,
                        knee_db,
                        drive,
                        limiter_on,
                    )
                } else {
                    (pre_l, pre_r)
                };

                const UNITY_TO_PLUS_6DB: f32 = 1.995_262_3 - 1.0;
                let warmth_amount =
                    ((master_gain - 1.0) / UNITY_TO_PLUS_6DB).clamp(0.0, 1.0);
                let (wl, wr) = self.tube_warmth.process_sample(cl, cr, warmth_amount);

                // DJ Filter POST: after warmth, before master volume
                let (fl, fr) = if !filt_pre {
                    self.dj_filter.process_sample(wl, wr, filt_pos, filt_res)
                } else {
                    (wl, wr)
                };

                let ol = soft_clip_safety(fl * master_gain);
                let or_ = soft_clip_safety(fr * master_gain);
                if ol.is_finite() && or_.is_finite() {
                    *l = ol;
                    *r = or_;
                } else {
                    *l = 0.0;
                    *r = 0.0;
                    nan_detected = true;
                }

                let sample_peak = ol.abs().max(or_.abs());
                if sample_peak > post_peak {
                    post_peak = sample_peak;
                }
                let gr = self.master_bus.last_gr_db();
                if gr > max_gr_db {
                    max_gr_db = gr;
                }

                // Feed the spectrum analyzer with the mono sum of the final
                // post-gain output. `feed_sample` returns true once per
                // FFT_SIZE (1024) samples, at which point we publish the
                // fresh dB-per-band array to the GUI via 64 relaxed atomic
                // stores — no mutex, no heap, nothing that can trip
                // `assert_process_allocs`.
                let mono = 0.5 * (ol + or_);
                if !self.spectrum_disabled && self.spectrum.feed_sample(mono) {
                    self.spectrum_shared.store_bins(self.spectrum.bins_db());
                }
            }

            // If any sample in this buffer was non-finite, reset filter
            // and comp state once so the next buffer starts clean.
            // NOTE: no tracing/logging here — we're on the audio thread and
            // assert_process_allocs will panic on any heap allocation.
            if nan_detected {
                self.dj_filter.reset();
                self.master_bus.prepare(sr);
            }

            if let Some(ref mut tx) = self.telemetry_tx {
                tx.push(post_peak);
            }
            self.meter_shared.store_gr_db(max_gr_db);
        }

        ProcessStatus::KeepAlive
    }
}

impl ClapPlugin for Niner {
    const CLAP_ID: &'static str = "com.hyperfocusdsp.niner";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Kick drum synthesizer");
    const CLAP_MANUAL_URL: Option<&'static str> = Some("https://hyperfocusdsp.com/niner");
    const CLAP_SUPPORT_URL: Option<&'static str> = Some("https://hyperfocusdsp.com/support");
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Synthesizer,
        ClapFeature::Drum,
    ];
}

impl Vst3Plugin for Niner {
    // Hard break in v0.7.0: the rebrand from Slammer to Niner forces a new
    // class ID. Existing DAW projects saved with the old "SlammerKickSy01\0"
    // ID will not auto-find this plugin and must be re-wired.
    const VST3_CLASS_ID: [u8; 16] = *b"NinerKickSynth01";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[
        Vst3SubCategory::Instrument,
        Vst3SubCategory::Synth,
        Vst3SubCategory::Drum,
    ];
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::engine::KickParams;
    use crate::dsp::spectrum::SpectrumAnalyzer;

    /// Render a long passage of the FULL signal chain (engine + master_bus
    /// + DJ filter + tube + soft-clip + spectrum FFT feed) at the same
    /// default-Init operating point the user reports glitches at: all
    /// distortion stages off, comp/limiter off, DJ filter centred,
    /// master_volume = 1.0 (warmth bypassed). Writes a WAV to /tmp so the
    /// waveform can be inspected, and returns the captured samples for
    /// further assertion.
    ///
    /// `bypass_spectrum` lets bisect tests turn off the FFT feed stage
    /// to localise glitches: if disabling spectrum makes them disappear,
    /// the spectrum analyzer is the culprit.
    fn render_default_kick_loop_full_chain(
        seconds: f32,
        bpm: f32,
        bypass_spectrum: bool,
    ) -> Vec<f32> {
        let sr = 48_000.0f32;
        let total = (sr * seconds) as usize;
        let samples_per_step = (sr * 60.0 / bpm / 4.0) as usize; // 16th note
        let buffer_size = 1024usize;

        let mut engine = KickEngine::new(sr);
        let mut master_bus = MasterBus::new();
        let mut tube = TubeWarmth::new();
        let mut dj = DjFilter::new();
        let mut spectrum = SpectrumAnalyzer::new(sr);
        master_bus.prepare(sr);
        dj.set_sample_rate(sr);

        let params = KickParams::default();

        let mut out = vec![0.0f32; total];
        let mut sample_in_step: usize = 0;
        let mut step: usize = 0;

        let mut i = 0usize;
        while i < total {
            let n = (i + buffer_size).min(total) - i;
            let mut buf_l = vec![0.0f32; n];
            let mut buf_r = vec![0.0f32; n];

            // Fire kicks 4-on-the-floor: every step is a hit (we'll fire on
            // step boundaries inside the per-sample loop to keep timing
            // sample-accurate).
            for j in 0..n {
                if sample_in_step == 0 {
                    engine.trigger(&params);
                }
                sample_in_step += 1;
                if sample_in_step >= samples_per_step {
                    sample_in_step = 0;
                    step = step.wrapping_add(1);
                }
                let _ = j;
            }

            // Engine fills (sums into) the buffer
            engine.process(&mut buf_l, &mut buf_r, &params);

            // Mirror plugin.rs per-sample chain at default operating point:
            // DJ filter centred (bypass), master_bus bypassed, tube bypassed,
            // soft_clip_safety, optional spectrum FFT feed.
            for j in 0..n {
                // NaN guard
                if !buf_l[j].is_finite() {
                    buf_l[j] = 0.0;
                }
                if !buf_r[j].is_finite() {
                    buf_r[j] = 0.0;
                }

                // DJ filter PRE: filt_pos=0 → bypass
                let (pre_l, pre_r) = dj.process_sample(buf_l[j], buf_r[j], 0.0, 0.0);

                // master_bus bypass gate: amount=0, drive=0, limiter=false
                let (cl, cr) = (pre_l, pre_r);

                // tube_warmth: master_gain=1.0 → amount=0 → bypass
                let (wl, wr) = tube.process_sample(cl, cr, 0.0);

                // DJ filter POST: filt_pre=true (default? let's mirror)
                // We took filt_pre branch above; nothing to do here.
                let (fl, fr) = (wl, wr);

                // soft_clip_safety + master gain
                let ol = soft_clip_safety(fl * 1.0);
                let or_ = soft_clip_safety(fr * 1.0);

                // mono mix into out
                let mono = 0.5 * (ol + or_);
                out[i + j] = mono;

                if !bypass_spectrum {
                    let _ = spectrum.feed_sample(mono);
                }
            }

            i += n;
        }

        // Write WAV for visual inspection (mono, 48 kHz, 32-bit float).
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: sr as u32,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let path = if bypass_spectrum {
            "/tmp/niner_offline_no_spectrum.wav"
        } else {
            "/tmp/niner_offline_full_chain.wav"
        };
        if let Ok(mut wav) = hound::WavWriter::create(path, spec) {
            for &s in &out {
                let _ = wav.write_sample(s);
            }
            let _ = wav.finalize();
        }

        out
    }

    /// Repro driver: render the full chain, look for any sample-to-sample
    /// jumps that would be audible as bitcrush-style artifacts. A clean
    /// kick at default decay has its largest natural delta during the
    /// attack ramp (~0.05/sample) and the sub-sweep transition; nothing
    /// should exceed ~0.3 in clean playback. We also dump per-second
    /// max-deltas so a periodic FFT-correlated artifact would show up.
    #[test]
    fn render_full_chain_default_preset_no_audible_artifacts() {
        let secs = 5.0;
        let bpm = 120.0;
        let samples = render_default_kick_loop_full_chain(secs, bpm, false);
        let sr = 48_000.0f32 as usize;

        // Stats per second
        for s in 0..(secs as usize) {
            let start = s * sr;
            let end = ((s + 1) * sr).min(samples.len());
            let slice = &samples[start..end];
            let mut max_abs = 0.0f32;
            let mut max_delta = 0.0f32;
            let mut max_delta_idx = 0usize;
            for i in 1..slice.len() {
                let a = slice[i].abs();
                if a > max_abs { max_abs = a; }
                let d = (slice[i] - slice[i - 1]).abs();
                if d > max_delta {
                    max_delta = d;
                    max_delta_idx = i;
                }
            }
            eprintln!(
                "second {s}: peak={max_abs:.4}, max_delta={max_delta:.4} at offset {max_delta_idx}"
            );
        }

        // Hard checks
        for (i, &s) in samples.iter().enumerate() {
            assert!(s.is_finite(), "non-finite at sample {i}");
        }
        let mut worst = 0.0f32;
        let mut worst_idx = 0usize;
        for i in 1..samples.len() {
            let d = (samples[i] - samples[i - 1]).abs();
            if d > worst { worst = d; worst_idx = i; }
        }
        eprintln!("OVERALL worst delta = {worst:.6} at sample {worst_idx}");
        // Threshold tuned to catch bitcrush-style artifacts that exceed the
        // natural attack-ramp slope of a clean kick. A loose 0.5 lets the
        // attack edge through; anything dramatically above this needs to be
        // explained.
        assert!(
            worst < 0.5,
            "audible artifact: per-sample delta {worst:.6} at sample {worst_idx}"
        );
    }

    /// Companion bisect test: re-run with spectrum FFT feed disabled.
    /// If the *first* test fails on per-second deltas correlated with the
    /// 1024-sample FFT period and this test is clean, the spectrum
    /// analyzer's call into `realfft::process_with_scratch` is the culprit.
    #[test]
    fn render_full_chain_default_preset_no_artifacts_without_spectrum() {
        let secs = 5.0;
        let bpm = 120.0;
        let samples = render_default_kick_loop_full_chain(secs, bpm, true);
        for (i, &s) in samples.iter().enumerate() {
            assert!(s.is_finite(), "non-finite at sample {i}");
        }
    }

    /// Bisect helper for the v0.6.0 glitch hunt: render with custom kick
    /// params (e.g. 909 preset values, heavy drift) and comp/limiter
    /// engaged to make sure the bug isn't hiding in a non-default preset.
    /// Writes WAV to /tmp/niner_offline_<tag>.wav.
    fn render_with_params(
        seconds: f32,
        bpm: f32,
        params: KickParams,
        comp_amount: f32,
        comp_drive: f32,
        limiter_on: bool,
        master_volume: f32,
        tag: &str,
    ) -> Vec<f32> {
        let sr = 48_000.0f32;
        let total = (sr * seconds) as usize;
        let samples_per_step = (sr * 60.0 / bpm / 4.0) as usize;
        let buffer_size = 1024usize;

        let mut engine = KickEngine::new(sr);
        let mut master_bus = MasterBus::new();
        let mut tube = TubeWarmth::new();
        let mut dj = DjFilter::new();
        let mut spectrum = SpectrumAnalyzer::new(sr);
        master_bus.prepare(sr);
        dj.set_sample_rate(sr);

        let mut out = vec![0.0f32; total];
        let mut sample_in_step = 0usize;

        let mut i = 0usize;
        while i < total {
            let n = (i + buffer_size).min(total) - i;
            let mut buf_l = vec![0.0f32; n];
            let mut buf_r = vec![0.0f32; n];

            for _ in 0..n {
                if sample_in_step == 0 {
                    engine.trigger(&params);
                }
                sample_in_step += 1;
                if sample_in_step >= samples_per_step {
                    sample_in_step = 0;
                }
            }
            engine.process(&mut buf_l, &mut buf_r, &params);

            for j in 0..n {
                if !buf_l[j].is_finite() { buf_l[j] = 0.0; }
                if !buf_r[j].is_finite() { buf_r[j] = 0.0; }

                let (pre_l, pre_r) = dj.process_sample(buf_l[j], buf_r[j], 0.0, 0.0);

                let threshold_db = -6.0 + comp_amount * -24.0;
                let ratio = 2.0 + comp_amount * 8.0;
                let knee_db = 0.0f32;
                master_bus.set_times(10.0, 100.0, sr);
                let (cl, cr) = if comp_amount > 0.0001 || comp_drive > 0.001 || limiter_on {
                    master_bus.process_sample(
                        pre_l, pre_r,
                        threshold_db, ratio, knee_db,
                        comp_drive, limiter_on,
                    )
                } else {
                    (pre_l, pre_r)
                };

                const UNITY_TO_PLUS_6DB: f32 = 1.995_262_3 - 1.0;
                let warmth_amount = ((master_volume - 1.0) / UNITY_TO_PLUS_6DB).clamp(0.0, 1.0);
                let (wl, wr) = tube.process_sample(cl, cr, warmth_amount);

                let ol = soft_clip_safety(wl * master_volume);
                let or_ = soft_clip_safety(wr * master_volume);

                let mono = 0.5 * (ol + or_);
                out[i + j] = mono;
                let _ = spectrum.feed_sample(mono);
            }

            i += n;
        }

        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: sr as u32,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let path = format!("/tmp/niner_offline_{tag}.wav");
        if let Ok(mut wav) = hound::WavWriter::create(&path, spec) {
            for &s in &out {
                let _ = wav.write_sample(s);
            }
            let _ = wav.finalize();
        }

        out
    }

    /// 909-preset render: drift engaged, Diode saturation, Tanh voice clip,
    /// 16th-note retriggering. Mirrors what the user's most-likely operating
    /// point looks like.
    #[test]
    fn render_909_preset_no_artifacts() {
        let mut p = KickParams::default();
        p.decay_ms = 200.0;
        p.sub_fstart = 65.0;
        p.sub_fend = 50.0;
        p.top_gain = 0.15;
        p.mid_noise_gain = 0.1;
        p.mid_noise_decay_ms = 15.0;
        p.sat_mode = 2; // Diode
        p.sat_drive = 0.1;
        p.drift_amount = 0.2;
        p.kick_clip_mode = 1; // Tanh
        p.kick_clip_drive = 0.15;
        let samples = render_with_params(5.0, 120.0, p, 0.0, 0.0, false, 1.0, "909");
        for (i, &s) in samples.iter().enumerate() {
            assert!(s.is_finite(), "non-finite at sample {i}");
        }
        let mut worst = 0.0f32; let mut worst_idx = 0usize;
        for i in 1..samples.len() {
            let d = (samples[i] - samples[i-1]).abs();
            if d > worst { worst = d; worst_idx = i; }
        }
        eprintln!("909 worst delta = {worst:.6} at sample {worst_idx}");
        assert!(worst < 0.7, "audible artifact in 909 preset: delta {worst:.6} at {worst_idx}");
    }

    /// Comp + limiter + drive engaged + master_volume above unity (tube
    /// warmth active). Exercises every per-sample DSP stage at once.
    #[test]
    fn render_full_chain_everything_engaged_no_artifacts() {
        let p = KickParams::default();
        let samples = render_with_params(5.0, 120.0, p, 0.5, 0.4, true, 1.4, "everything_on");
        for (i, &s) in samples.iter().enumerate() {
            assert!(s.is_finite(), "non-finite at sample {i}");
        }
        let mut worst = 0.0f32; let mut worst_idx = 0usize;
        for i in 1..samples.len() {
            let d = (samples[i] - samples[i-1]).abs();
            if d > worst { worst = d; worst_idx = i; }
        }
        eprintln!("everything-on worst delta = {worst:.6} at sample {worst_idx}");
        // Comp + limiter introduce dynamic compression so single-sample
        // deltas can be larger than the un-comped tests, but should still
        // stay well under unity.
        assert!(worst < 1.0, "audible artifact: delta {worst:.6} at {worst_idx}");
    }

    /// Heavy retriggering: 32nd notes (16/sec at 120 BPM) — voice stealing
    /// happens almost continuously. Default preset, all FX off.
    #[test]
    fn render_heavy_retrigger_no_artifacts() {
        let p = KickParams::default();
        let samples = render_with_params(3.0, 120.0 * 2.0, p, 0.0, 0.0, false, 1.0, "heavy_retrig");
        for (i, &s) in samples.iter().enumerate() {
            assert!(s.is_finite(), "non-finite at sample {i}");
        }
        let mut worst = 0.0f32; let mut worst_idx = 0usize;
        for i in 1..samples.len() {
            let d = (samples[i] - samples[i-1]).abs();
            if d > worst { worst = d; worst_idx = i; }
        }
        eprintln!("heavy_retrig worst delta = {worst:.6} at sample {worst_idx}");
        assert!(worst < 0.6, "audible artifact in heavy retrigger: delta {worst:.6} at {worst_idx}");
    }

    #[test]
    fn soft_clip_safety_passes_through_below_threshold() {
        // Anything under SC_THRESHOLD must be bit-identical to input so
        // normal-loudness material is untouched.
        for x in [-0.84f32, -0.5, -0.1, 0.0, 0.1, 0.5, 0.84] {
            assert_eq!(soft_clip_safety(x), x);
        }
    }

    #[test]
    fn soft_clip_safety_stays_within_full_scale_at_extremes() {
        // The DAC hard-clips at ±1.0 in f32→i16/i24 conversion. The
        // safety stage's job is to keep output inside that box for any
        // finite input — tanh asymptotes to SC_CEILING (0.999) but in
        // f32 arithmetic large inputs round up to exactly SC_CEILING,
        // which is still well under 1.0.
        for x in [1.0f32, 2.0, 10.0, 100.0, -1.0, -2.0, -10.0, -100.0] {
            let y = soft_clip_safety(x);
            assert!(y.abs() <= SC_CEILING, "soft_clip({}) = {} > ceiling", x, y);
            assert!(y.abs() < 1.0, "soft_clip({}) = {} would clip DAC", x, y);
        }
    }

    #[test]
    fn soft_clip_safety_is_odd_and_continuous_across_threshold() {
        // sign symmetry
        assert_eq!(soft_clip_safety(0.9), -soft_clip_safety(-0.9));
        // continuity at threshold — left-limit and right-limit must match
        let below = soft_clip_safety(0.8499);
        let above = soft_clip_safety(0.8501);
        assert!(
            (below - above).abs() < 1e-3,
            "discontinuity at threshold: {} vs {}",
            below,
            above
        );
    }

    /// Regression test for the v0.5.2 Windows crackling report: at default
    /// params, engine peak ~1.07 (sub+mid+top sums above unity). The
    /// master-bus — including the brickwall limiter — is bypassed when
    /// comp_amount, comp_drive, and limit_on are all at their defaults,
    /// so without the output safety clipper the DAC hard-clips. This test
    /// drives 32 kick hits at 120 BPM through the exact default signal
    /// path and asserts final output stays below full-scale.
    #[test]
    fn output_never_exceeds_full_scale_at_default_preset() {
        let sr = 48_000.0f32;
        let params = KickParams::default();
        let mut engine = KickEngine::new(sr);

        let samples_per_hit = (sr * 60.0 / 120.0 / 4.0) as usize; // 16th @ 120 BPM
        let total_hits = 32usize;
        let total_samples = samples_per_hit * total_hits;
        let mut l = vec![0.0f32; total_samples];
        let mut r = vec![0.0f32; total_samples];

        for hit in 0..total_hits {
            engine.trigger(&params);
            let start = hit * samples_per_hit;
            let end = start + samples_per_hit;
            engine.process(&mut l[start..end], &mut r[start..end], &params);
        }

        // Mirror plugin.rs per-sample loop with defaults: master_bus bypassed
        // (amount=0, drive=0, limit_on=false), DJ filter off, warmth off,
        // master_gain=1.0. Apply soft_clip_safety as the final stage.
        let master_gain = 1.0f32;
        let mut final_peak = 0.0f32;
        for i in 0..total_samples {
            let ol = soft_clip_safety(l[i] * master_gain);
            final_peak = final_peak.max(ol.abs());
        }

        assert!(
            final_peak < SC_CEILING,
            "final output peak {final_peak} reached/exceeded ceiling {SC_CEILING}"
        );
    }

    /// Integration test: exercise the full master chain (engine → NaN guard →
    /// comp → warmth → DJ filter → master gain) and verify the output is
    /// non-zero. Catches bugs where individual stages are fine but the
    /// plugin-level wiring silences the signal.
    #[test]
    fn full_chain_produces_output() {
        let sr = 48000.0;
        let n = 1024;

        let mut engine = KickEngine::new(sr);
        let mut master_bus = MasterBus::new();
        let mut tube = TubeWarmth::new();
        let mut filt = DjFilter::new();
        master_bus.prepare(sr);
        filt.set_sample_rate(sr);

        let params = KickParams::default();
        engine.trigger(&params);

        let mut left = vec![0.0f32; n];
        let mut right = vec![0.0f32; n];
        engine.process(&mut left, &mut right, &params);

        // Sanitize (mirrors plugin.rs)
        for s in left.iter_mut().chain(right.iter_mut()) {
            if !s.is_finite() {
                *s = 0.0;
            }
        }

        let mut peak = 0.0f32;
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            // Default params: comp bypassed, filter bypassed, warmth bypassed
            master_bus.set_times(10.0, 100.0, sr);
            let (fl, fr) = filt.process_sample(*l, *r, 0.0, 0.0);
            let (wl, wr) = tube.process_sample(fl, fr, 0.0);
            let ol = wl * 1.0; // master_gain = 1.0
            let or_ = wr * 1.0;
            *l = ol;
            *r = or_;
            peak = peak.max(ol.abs().max(or_.abs()));
        }

        assert!(
            peak > 0.01,
            "full chain must produce audible output, got peak {}",
            peak
        );
        // Also verify no NaN snuck through
        assert!(peak.is_finite(), "output must be finite");
    }

    /// Simulate rapid retriggering (120 BPM 16th notes) through the full
    /// master chain with comp engaged. Checks for discontinuities that
    /// would be audible as clicks.
    #[test]
    fn full_chain_no_clicks_on_retrigger() {
        let sr = 48000.0;
        // 120 BPM 16th notes = 8 hits/sec → 6000 samples between hits
        let hit_interval = 6000usize;
        let num_hits = 8;
        let total = hit_interval * num_hits;

        let mut engine = KickEngine::new(sr);
        let mut master_bus = MasterBus::new();
        let mut tube = TubeWarmth::new();
        let mut filt = DjFilter::new();
        master_bus.prepare(sr);
        filt.set_sample_rate(sr);

        let params = KickParams {
            clap_on: true,
            ..KickParams::default()
        };

        let mut output = vec![0.0f32; total];

        for hit in 0..num_hits {
            let start = hit * hit_interval;
            let end = (start + hit_interval).min(total);
            let len = end - start;

            engine.trigger(&params);
            let mut buf_l = vec![0.0f32; len];
            let mut buf_r = vec![0.0f32; len];
            engine.process(&mut buf_l, &mut buf_r, &params);

            // Run through master chain with comp engaged
            for i in 0..len {
                master_bus.set_times(10.0, 200.0, sr);
                let (cl, cr) = master_bus.process_sample(
                    buf_l[i], buf_r[i], -18.0, 4.0, 6.0, 0.0, false,
                );
                let (wl, _wr) = tube.process_sample(cl, cr, 0.0);
                let (fl, _fr) = filt.process_sample(wl, wl, 0.0, 0.0);
                output[start + i] = fl;
            }
        }

        // Check for hard clicks: a per-sample delta > 0.4 in the
        // compressor output is suspicious.
        let mut worst_delta = 0.0f32;
        let mut worst_idx = 0;
        for i in 1..total {
            let d = (output[i] - output[i - 1]).abs();
            if d > worst_delta {
                worst_delta = d;
                worst_idx = i;
            }
        }
        // Identify which hit boundary the worst delta is near
        let near_hit = worst_idx / hit_interval;
        eprintln!(
            "worst delta = {worst_delta:.6} at sample {worst_idx} (near hit {near_hit})"
        );
        assert!(
            worst_delta < 0.4,
            "click detected at sample {worst_idx}: delta = {worst_delta:.4}"
        );

        // Verify no NaN
        for (i, &s) in output.iter().enumerate() {
            assert!(s.is_finite(), "non-finite at sample {i}");
        }
    }
}
