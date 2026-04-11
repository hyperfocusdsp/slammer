//! The `Plugin` impl for Slammer — glues together parameters, DSP engine,
//! telemetry, and the egui editor. Parameter definitions themselves live in
//! [`crate::params`]; DSP is in [`crate::dsp`].

use nih_plug::prelude::*;
use parking_lot::Mutex;
use std::sync::Arc;

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
    const VENDOR: &'static str = "REXIST";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
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
                UiToDsp::Trigger { velocity } => {
                    self.engine.trigger(&kick_params, velocity);
                }
            }
        }

        while let Some(event) = context.next_event() {
            if let NoteEvent::NoteOn { velocity, .. } = event {
                self.engine.trigger(&kick_params, velocity);
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
                            self.engine.trigger(&collect_kick_params(&self.params), 1.0);
                            if self.params.flam_on.value() {
                                let gap_samples = ((self.params.flam_spread_ms.value() * 0.001)
                                    * self.sample_rate)
                                    .round() as u32;
                                let humanize = self.params.flam_humanize.value();
                                self.engine.schedule_ghost(gap_samples, humanize);
                            }
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
                    self.engine.trigger(&collect_kick_params(&self.params), 1.0);
                    if self.params.flam_on.value() {
                        let gap_samples = ((self.params.flam_spread_ms.value() * 0.001)
                            * self.sample_rate)
                            .round() as u32;
                        let humanize = self.params.flam_humanize.value();
                        self.engine.schedule_ghost(gap_samples, humanize);
                    }
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
                        self.engine.trigger(&collect_kick_params(&self.params), 1.0);
                        if self.params.flam_on.value() {
                            let gap_samples = ((self.params.flam_spread_ms.value() * 0.001)
                                * self.sample_rate)
                                .round() as u32;
                            let humanize = self.params.flam_humanize.value();
                            self.engine.schedule_ghost(gap_samples, humanize);
                        }
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
            let _ = self.engine.process(output_left, output_right, &kick_params);

            // Snapshot bypass-able state once per buffer. Smoothed macro
            // params are pulled inside the per-sample loop.
            let limiter_on = self.params.comp_limit_on.value();
            let sr = self.sample_rate;

            let mut post_peak = 0.0f32;
            let mut max_gr_db = 0.0f32;

            for (l, r) in output_left.iter_mut().zip(output_right.iter_mut()) {
                // Smoothed pulls — cheap scalar arithmetic.
                let amount = self.params.comp_amount.smoothed.next();
                let react = self.params.comp_react.smoothed.next();
                let drive = self.params.comp_drive.smoothed.next();
                let master_gain = self.params.master_volume.smoothed.next();

                // Macro → DSP mapping (see planning doc §"Macro mappings").
                let threshold_db = -6.0 + amount * -24.0; // -6 .. -30
                let ratio = 2.0 + amount * 8.0; // 2:1 .. 10:1
                let attack_ms = 30.0 + react * (1.5 - 30.0); // 30 .. 1.5
                let release_ms = 400.0 + react * (40.0 - 400.0); // 400 .. 40

                self.master_bus.set_times(attack_ms, release_ms, sr);

                // Bypass when fully clean to stay bit-identical to the
                // pre-comp build on default settings.
                let (cl, cr) = if amount > 0.0001 || drive > 0.001 || limiter_on {
                    self.master_bus.process_sample(
                        *l,
                        *r,
                        threshold_db,
                        ratio,
                        drive,
                        limiter_on,
                    )
                } else {
                    (*l, *r)
                };

                // Tube warmth: engaged automatically when the master volume
                // knob is pushed past 0 dB (linear gain > 1.0). `amount`
                // ramps 0 → 1 as the gain climbs from unity to +6 dB. Below
                // unity the stage is bit-identical bypass, so nothing
                // changes for users who don't push the knob.
                const UNITY_TO_PLUS_6DB: f32 = 1.995_262_3 - 1.0; // db_to_gain(6) − 1
                let warmth_amount =
                    ((master_gain - 1.0) / UNITY_TO_PLUS_6DB).clamp(0.0, 1.0);
                let (wl, wr) = self.tube_warmth.process_sample(cl, cr, warmth_amount);

                let ol = wl * master_gain;
                let or_ = wr * master_gain;
                *l = ol;
                *r = or_;

                let sample_peak = ol.abs().max(or_.abs());
                if sample_peak > post_peak {
                    post_peak = sample_peak;
                }
                let gr = self.master_bus.last_gr_db();
                if gr > max_gr_db {
                    max_gr_db = gr;
                }
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
    const CLAP_ID: &'static str = "com.rexist.slammer";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Kick drum synthesizer");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Synthesizer,
        ClapFeature::Drum,
    ];
}

impl Vst3Plugin for Slammer {
    const VST3_CLASS_ID: [u8; 16] = *b"SlammerKickSy01\0";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[
        Vst3SubCategory::Instrument,
        Vst3SubCategory::Synth,
        Vst3SubCategory::Drum,
    ];
}
