//! The `Plugin` impl for Slammer — glues together parameters, DSP engine,
//! telemetry, and the egui editor. Parameter definitions themselves live in
//! [`crate::params`]; DSP is in [`crate::dsp`].

use nih_plug::prelude::*;
use parking_lot::Mutex;
use std::sync::Arc;

use crate::dsp::dj_filter::DjFilter;
use crate::dsp::engine::KickEngine;
use crate::dsp::master_bus::MasterBus;
use crate::dsp::tube::TubeWarmth;
use crate::logging;
use crate::params::{collect_kick_params, SlammerParams};
use crate::presets::PresetManager;
use crate::sequencer::{self, Sequencer};
use crate::util::messages::{self, UiToDsp};
use crate::util::telemetry::{self, MeterShared, TelemetryProducer};

use std::sync::atomic::Ordering;

pub struct Slammer {
    params: Arc<SlammerParams>,
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
}

impl Default for Slammer {
    fn default() -> Self {
        let (telem_tx, telem_rx) = telemetry::channel();
        let (ui_tx, ui_rx) = messages::channel();
        let params = Arc::new(SlammerParams::default());
        let sequencer = Arc::new(Sequencer::new(Arc::clone(&params.seq_steps)));
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
            ui_tx_holder: Arc::new(Mutex::new(Some(ui_tx))),
            ui_rx,
            preset_manager: Arc::new(Mutex::new(PresetManager::new())),
            sequencer,
            seq_sample_counter: 0,
            seq_current_step: 0,
            host_ever_stopped: false,
            last_host_step: None,
            seq_running_prev: false,
        }
    }
}

impl Plugin for Slammer {
    const NAME: &'static str = "Slammer";
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
        crate::ui::editor::create(
            self.params.editor_state.clone(),
            params,
            telemetry_rx,
            ui_tx,
            preset_manager,
            sequencer,
            meter,
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
            "Slammer v{} initialized — sr: {}",
            Self::VERSION,
            self.sample_rate
        );
        self.engine.set_sample_rate(self.sample_rate);
        self.master_bus.prepare(self.sample_rate);
        self.dj_filter.set_sample_rate(self.sample_rate);
        // nih-plug has already deserialized `params.seq_steps` at this
        // point; copy the bitmask into the sequencer atomics so the first
        // `process()` call sees the restored pattern.
        self.sequencer.restore_from_persist();
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
                            self.engine.trigger(&collect_kick_params(&self.params));
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
                    self.engine.trigger(&collect_kick_params(&self.params));
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
                        self.engine.trigger(&collect_kick_params(&self.params));
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

                let ol = fl * master_gain;
                let or_ = fr * master_gain;
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

impl ClapPlugin for Slammer {
    const CLAP_ID: &'static str = "com.hyperfocusdsp.slammer";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Kick drum synthesizer");
    const CLAP_MANUAL_URL: Option<&'static str> = Some("https://hyperfocusdsp.com/slammer");
    const CLAP_SUPPORT_URL: Option<&'static str> = Some("https://hyperfocusdsp.com/support");
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Synthesizer,
        ClapFeature::Drum,
    ];
}

impl Vst3Plugin for Slammer {
    // Unchanged from v0.4.x to preserve DAW-project compatibility — class ID
    // is the host-side identity for project recall and is not user-visible.
    const VST3_CLASS_ID: [u8; 16] = *b"SlammerKickSy01\0";
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
