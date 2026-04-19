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
use crate::ui::panels::{self, CONTENT_LEFT, KNOB_SPACING};
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

    // Header logo texture, lazily uploaded on first paint.
    let logo_texture: Arc<Mutex<Option<egui::TextureHandle>>> = Arc::new(Mutex::new(None));
    // Footer "manufacturer mark" (Hyperfocus DSP wordmark) — same lazy
    // upload pattern as the header logo, separate handle so each can be
    // sized independently if needed.
    let hf_logo_texture: Arc<Mutex<Option<egui::TextureHandle>>> = Arc::new(Mutex::new(None));
    let dice_locks = Arc::new(std::sync::atomic::AtomicU8::new(0));

    create_egui_editor(
        editor_state,
        (),
        |ctx, _| {
            theme::setup_fonts(ctx);
            theme::setup_style(ctx);
        },
        move |ctx, setter, _state| {
            // Scaling is handled outside this callback: baseview applies the
            // window scale factor (standalone via `--dpi-scale`, DAW via
            // `Editor::set_scale_factor`), and egui's `pixels_per_point`
            // follows. We do NOT call `ctx.set_pixels_per_point()` here —
            // that fights baseview and double-scales the layout.
            let _ = &editor_state_clone;

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
                            entry.params.apply(setter, &params);
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

                    // Header logo (lazy texture upload, then painter::image).
                    {
                        let mut tex = logo_texture.lock();
                        if tex.is_none() {
                            let bytes = include_bytes!("../../assets/slammer_logo.png");
                            if let Ok(img) = image::load_from_memory(bytes) {
                                let rgba = img.to_rgba8();
                                let (w, h) = rgba.dimensions();
                                let pixels = rgba.into_raw();
                                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                                    [w as usize, h as usize],
                                    &pixels,
                                );
                                *tex = Some(ctx.load_texture(
                                    "slammer_logo",
                                    color_image,
                                    egui::TextureOptions::LINEAR,
                                ));
                            }
                        }
                        if let Some(t) = tex.as_ref() {
                            let logo_h = 18.0;
                            let logo_w = logo_h * (480.0 / 84.0);
                            let logo_rect = egui::Rect::from_min_size(
                                egui::pos2(
                                    panel_rect.left() + CONTENT_LEFT,
                                    header_center_y - logo_h * 0.5,
                                ),
                                egui::vec2(logo_w, logo_h),
                            );
                            ui.painter().image(
                                t.id(),
                                logo_rect,
                                egui::Rect::from_min_max(
                                    egui::pos2(0.0, 0.0),
                                    egui::pos2(1.0, 1.0),
                                ),
                                theme::WHITE,
                            );
                        }
                    }

                    // UI scale badge — discreet click-to-cycle, lives in the
                    // header to the left of "KICK SYNTHESIZER" so the footer
                    // chrome stays clean. Mirrors the SquelchBox `band1.rs`
                    // pattern; the new value is mirrored to a sidecar file so
                    // `slammer-launch` can forward it as `--dpi-scale` on the
                    // next standalone launch (DAWs honour `#[persist]` directly).
                    {
                        let scale = *params.ui_scale.lock();
                        let scale_text = if (scale - scale.round()).abs() < 0.05 {
                            format!("UI {:.0}×", scale)
                        } else {
                            format!("UI {:.1}×", scale)
                        };
                        // KICK SYNTHESIZER is right-aligned at right-CONTENT_LEFT
                        // and ~80px wide at 8pt mono; clear it by ~14px.
                        let badge_right = panel_rect.right() - CONTENT_LEFT - 94.0;
                        let badge_w = 50.0;
                        let badge_h = 14.0;
                        let hit = egui::Rect::from_min_size(
                            egui::pos2(badge_right - badge_w, header_center_y - badge_h * 0.5),
                            egui::vec2(badge_w, badge_h),
                        );
                        let resp = ui
                            .interact(
                                hit,
                                egui::Id::new("ui_scale_btn"),
                                egui::Sense::click(),
                            )
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text(
                                "UI scale — click to cycle (1× / 1.5× / 2×).\n\
                                 Reopen the plugin (or restart slammer) to apply.",
                            );
                        let color = if resp.hovered() {
                            theme::WHITE
                        } else {
                            theme::TEXT_DIM
                        };
                        ui.painter().text(
                            egui::pos2(badge_right, header_center_y),
                            egui::Align2::RIGHT_CENTER,
                            &scale_text,
                            egui::FontId::new(8.0, egui::FontFamily::Monospace),
                            color,
                        );
                        if resp.clicked() {
                            let mut lock = params.ui_scale.lock();
                            let next = match *lock {
                                v if v < 1.25 => 1.5,
                                v if v < 1.75 => 2.0,
                                _ => 1.0,
                            };
                            *lock = next;
                            crate::util::paths::save_ui_scale(next);
                            tracing::info!(
                                "[ui_scale] cycled → {next}× (saved; reopen plugin to apply)"
                            );
                        }
                    }

                    // ===== Test trigger (button + keyboard 'T') =====
                    let button_fired = panels::test_button(ui, panel_rect, header_center_y);
                    let key_fired = ui.input(|i| i.key_pressed(egui::Key::T));
                    if button_fired || key_fired {
                        if let Some(tx) = ui_tx.lock().as_mut() {
                            // Dropped triggers are intentional: the ring is
                            // small, and the user won't notice one missed
                            // test-kick. No panic, no log spam.
                            let _ = tx.push(UiToDsp::Trigger);
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
                    let wf_width = 7.0 * KNOB_SPACING - 16.0;
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

                    // ===== Filter (SAT/EQ right column) =====
                    {
                        let filter_top = sat_eq_bottom_y - panels::KNOB_SIZE - 26.0;
                        panels::draw_filter_cluster(
                            ui, setter, &params, panel_rect, filter_top,
                        );
                    }

                    // ===== DICE + BOUNCE (sequencer right column) =====
                    {
                        // DICE sits just below the STEP groove (sat_eq_bottom_y + 18),
                        // with enough gap to not touch the separator line.
                        let dice_top = sat_eq_bottom_y + 23.0;
                        let dice_clicked = panels::draw_dice_row(
                            ui, panel_rect, dice_top, &dice_locks,
                        );
                        if dice_clicked {
                            let locked = dice_locks.load(std::sync::atomic::Ordering::Relaxed);
                            crate::ui::randomize::randomize(setter, &params, locked);
                        }
                        // BOUNCE sits just above the footer groove (bottom - 22).
                        let bounce_top = panel_rect.bottom() - 42.0;
                        let bounce_clicked = panels::draw_bounce_button(
                            ui, panel_rect, bounce_top,
                        );
                        if bounce_clicked {
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

                    // Footer manufacturer mark — full Hyperfocus DSP wordmark
                    // (with small-caps DSP suffix and ring-as-O) left-aligned
                    // in the footer strip. Sourced from `wordmark-master.svg`,
                    // not the no-DSP `wordmark-only.svg` derivative.
                    {
                        let mut tex = hf_logo_texture.lock();
                        if tex.is_none() {
                            let bytes =
                                include_bytes!("../../assets/hyperfocus_dsp_logo.png");
                            if let Ok(img) = image::load_from_memory(bytes) {
                                let rgba = img.to_rgba8();
                                let (w, h) = rgba.dimensions();
                                let pixels = rgba.into_raw();
                                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                                    [w as usize, h as usize],
                                    &pixels,
                                );
                                *tex = Some(ctx.load_texture(
                                    "hyperfocus_dsp_logo",
                                    color_image,
                                    egui::TextureOptions::LINEAR,
                                ));
                            }
                        }
                        if let Some(t) = tex.as_ref() {
                            // Source rendered at 142×24 (rsvg-convert -h 24);
                            // displaying at 10 px tall is a 2.4× downscale
                            // that LINEAR handles cleanly without aliasing.
                            let logo_h = 10.0;
                            let [tex_w, tex_h] = t.size();
                            let logo_w = logo_h * (tex_w as f32 / tex_h as f32);
                            let strip_y = panel_rect.bottom() - 17.0;
                            let logo_rect = egui::Rect::from_min_size(
                                egui::pos2(
                                    panel_rect.left() + CONTENT_LEFT,
                                    strip_y,
                                ),
                                egui::vec2(logo_w, logo_h),
                            );
                            ui.painter().image(
                                t.id(),
                                logo_rect,
                                egui::Rect::from_min_max(
                                    egui::pos2(0.0, 0.0),
                                    egui::pos2(1.0, 1.0),
                                ),
                                theme::WHITE,
                            );
                            let logo_resp = ui.interact(
                                logo_rect,
                                egui::Id::new("hyperfocus_brand"),
                                egui::Sense::hover(),
                            );
                            if logo_resp.hovered() {
                                logo_resp.on_hover_text("Made by Hyperfocus DSP");
                            }
                        }
                    }
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
