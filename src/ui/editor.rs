//! Editor shell: creates the egui editor, drains audio→GUI telemetry, and
//! composes the header / master row / knob panels / footer / preset bar.
//!
//! All drawing primitives and row layouts live in sibling modules
//! (`widgets`, `seven_seg`, `panels`, `preset_bar`); this file is
//! deliberately kept small so the overall flow is easy to follow.

use nih_plug::prelude::*;
use nih_plug_egui::egui;
use nih_plug_egui::{create_egui_editor, EguiState};
use parking_lot::Mutex;
use std::sync::Arc;

use crate::export::{self, ExportOutcome};
use crate::params::SlammerParams;
use crate::presets::PresetManager;
use crate::sequencer::Sequencer;
use crate::ui::panels::{self, BASE_W, CONTENT_LEFT, KNOB_SPACING};
use crate::ui::preset_bar::PresetBar;
use crate::ui::theme;
use crate::util::messages::UiToDsp;
use crate::util::telemetry::{MeterShared, TelemetryConsumer};

/// Rolling ring of recent audio peaks for the OUTPUT waveform display.
struct WaveformDisplay {
    peaks: Vec<f32>,
    max_points: usize,
}

impl WaveformDisplay {
    fn new(max_points: usize) -> Self {
        Self {
            peaks: Vec::with_capacity(max_points),
            max_points,
        }
    }

    fn push(&mut self, peak: f32) {
        if self.peaks.len() >= self.max_points {
            self.peaks.remove(0);
        }
        self.peaks.push(peak);
    }
}

pub fn create(
    editor_state: Arc<EguiState>,
    params: Arc<SlammerParams>,
    telemetry_rx: Option<TelemetryConsumer>,
    ui_tx: Option<rtrb::Producer<UiToDsp>>,
    preset_manager: Arc<Mutex<PresetManager>>,
    sequencer: Arc<Sequencer>,
    meter: Arc<MeterShared>,
) -> Option<Box<dyn Editor>> {
    let telemetry = Arc::new(Mutex::new(telemetry_rx));
    let waveform = Arc::new(Mutex::new(WaveformDisplay::new(200)));
    let preset_bar = Arc::new(Mutex::new(PresetBar::new(&preset_manager)));
    let ui_tx = Arc::new(Mutex::new(ui_tx));
    let seq_ui_state = Arc::new(Mutex::new(panels::SequencerUiState::default()));
    // Remembered export dir + format, loaded lazily from disk on first build.
    // The one-shot bounce button lives in the SAT/EQ row and fires through
    // this state so the next export opens at the same directory.
    let export_state = Arc::new(Mutex::new(export::load_export_state()));
    let editor_state_clone = Arc::clone(&editor_state);
    // Visually smoothed GR meter value — instant attack, slow release, held
    // across frames so the bar doesn't flicker between audio buffers.
    let gr_display = Arc::new(Mutex::new(0.0f32));

    // Restore-last-preset state: read the name once here, apply on the first
    // frame where we've confirmed we're running standalone (not a DAW — the
    // host owns state restoration there).
    let pending_restore: Arc<Mutex<Option<String>>> =
        Arc::new(Mutex::new(crate::presets::load_last_preset_name()));

    create_egui_editor(
        editor_state,
        (),
        |ctx, _| {
            theme::setup_fonts(ctx);
            theme::setup_style(ctx);
        },
        move |ctx, setter, _state| {
            // Aspect-ratio-locked scaling: layout is done at BASE_W logical
            // pixels and egui upscales to fill whatever window the host gave us.
            let (win_w, _) = editor_state_clone.size();
            let ppp = (win_w as f32 / BASE_W).max(1.0);
            ctx.set_pixels_per_point(ppp);

            // Drain audio-thread telemetry into the waveform ring.
            drain_telemetry(&telemetry, &waveform);

            // Restore last-used preset once the audio thread has confirmed
            // we're standalone. Skipped entirely in DAW mode — the host
            // restores parameter state from the project file itself, and we
            // don't want to clobber it.
            if sequencer
                .transport_probed
                .load(std::sync::atomic::Ordering::Relaxed)
            {
                let mut pending = pending_restore.lock();
                if let Some(name) = pending.take() {
                    if !sequencer.is_host_synced() {
                        let mgr = preset_manager.lock();
                        if let Some(entry) = mgr.list_all().into_iter().find(|e| e.name == name) {
                            entry.params.apply(setter, &params, &sequencer);
                            // Reflect the selection in the preset bar UI.
                            let mut bar = preset_bar.lock();
                            bar.select_by_name(&entry.name);
                        }
                    }
                }
            }

            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    let panel_rect = ui.max_rect();

                    // ===== Panel chrome =====
                    let header_center_y = panels::draw_chrome(ui, panel_rect);

                    // ===== Test trigger (button + keyboard 'T') =====
                    let button_fired = panels::test_button(ui, panel_rect, header_center_y);
                    let key_fired = ui.input(|i| i.key_pressed(egui::Key::T));
                    if button_fired || key_fired {
                        if let Some(tx) = ui_tx.lock().as_mut() {
                            // Dropped triggers are intentional: the ring is
                            // small, and the user won't notice one missed
                            // test-kick. No panic, no log spam.
                            let _ = tx.push(UiToDsp::Trigger { velocity: 1.0 });
                        }
                    }

                    // Spacebar toggles the standalone sequencer. Gated off in
                    // DAW mode so the host's own transport owns Space.
                    if ui.input(|i| i.key_pressed(egui::Key::Space))
                        && !sequencer.is_host_synced()
                    {
                        sequencer.toggle_running();
                    }

                    // ===== Header preset bar =====
                    {
                        let mut bar = preset_bar.lock();
                        let dt = ctx.input(|i| i.unstable_dt);
                        let preset_origin_x = panel_rect.left() + CONTENT_LEFT + 196.0;
                        bar.render(
                            ui,
                            setter,
                            &params,
                            &sequencer,
                            &preset_manager,
                            preset_origin_x,
                            header_center_y,
                            dt,
                        );
                    }

                    // ===== Groove below header =====
                    let groove_y = panel_rect.top() + 36.0;
                    {
                        let painter = ui.painter();
                        crate::ui::widgets::draw_groove(
                            painter,
                            panel_rect.left() + CONTENT_LEFT - 4.0,
                            panel_rect.right() - CONTENT_LEFT + 4.0,
                            groove_y,
                        );
                    }

                    // ===== Master row (OUTPUT + master knobs + comp strip) =====
                    let master_y = groove_y + 6.0;
                    let wf_left = panel_rect.left() + CONTENT_LEFT;
                    let wf_width = 6.0 * KNOB_SPACING - 16.0;
                    let wf_height = 56.0;

                    // Pull latest GR from the audio thread and apply a one-pole
                    // visual smoother: instant attack, ~180 ms release. `dt`
                    // from egui is already the frame time.
                    let dt = ctx.input(|i| i.unstable_dt).max(1e-4);
                    let gr_live = meter.load_gr_db();
                    let gr_smoothed = {
                        let mut g = gr_display.lock();
                        if gr_live >= *g {
                            *g = gr_live;
                        } else {
                            let release_tau = 0.18; // seconds
                            let a = (-dt / release_tau).exp();
                            *g = *g * a + gr_live * (1.0 - a);
                        }
                        *g
                    };

                    {
                        let wf = waveform.lock();
                        let master_row = panels::MasterRow {
                            master_y,
                            wf_left,
                            wf_width,
                            wf_height,
                            waveform_peaks: &wf.peaks,
                            gr_db: gr_smoothed,
                        };
                        master_row.draw(ui, setter, &params, panel_rect);
                    }

                    // ===== Three knob rows =====
                    let master_bottom_y = master_y + wf_height;
                    let sub_top_bottom_y =
                        panels::draw_sub_top_row(ui, setter, &params, panel_rect, master_bottom_y);
                    let mid_bottom_y =
                        panels::draw_mid_row(ui, setter, &params, panel_rect, sub_top_bottom_y);
                    let sat_eq_result =
                        panels::draw_sat_eq_row(ui, setter, &params, panel_rect, mid_bottom_y);
                    let sat_eq_bottom_y = sat_eq_result.next_y;

                    // BOUNCE click → run the one-shot export flow. This pops
                    // a native save dialog, renders a single hit at 44.1 kHz
                    // 16-bit through the full chain, and writes WAV or AIFF.
                    // All work happens on the GUI thread — audio is untouched.
                    if sat_eq_result.bounce_clicked {
                        let mut state = export_state.lock();
                        match export::export_one_shot(&mut state, &params) {
                            ExportOutcome::Written(path) => {
                                tracing::info!("bounce written: {}", path.display());
                            }
                            ExportOutcome::Cancelled => {}
                            ExportOutcome::UnsupportedExtension(ext) => {
                                tracing::warn!("bounce: unsupported extension .{}", ext);
                            }
                            ExportOutcome::Failed(msg) => {
                                tracing::error!("bounce failed: {}", msg);
                            }
                        }
                    }

                    // ===== Step sequencer =====
                    {
                        let mut seq_ui = seq_ui_state.lock();
                        panels::draw_sequencer_row(
                            ui,
                            setter,
                            &params,
                            panel_rect,
                            sat_eq_bottom_y,
                            &sequencer,
                            &mut seq_ui,
                        );
                    }

                    // ===== Footer =====
                    panels::draw_footer(ui, panel_rect);
                });
        },
    )
}

fn drain_telemetry(
    telemetry: &Mutex<Option<TelemetryConsumer>>,
    waveform: &Mutex<WaveformDisplay>,
) {
    let mut tel = telemetry.lock();
    let mut wf = waveform.lock();
    if let Some(rx) = tel.as_mut() {
        let mut temp = Vec::new();
        rx.drain_into(&mut temp, 128);
        for &p in &temp {
            wf.push(p);
        }
    }
}
