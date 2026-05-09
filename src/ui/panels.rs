//! Static panel layout: header chrome, groove/divider drawing, and the
//! SUB / TOP / MID / SAT / EQ / MASTER knob rows.
//!
//! These functions are pure layout helpers — each one takes the `ui`, the
//! parameter setter, the shared `NinerParams`, and whatever rect metadata
//! it needs, and paints directly. No mutable state is kept between calls.

use nih_plug::prelude::*;
use nih_plug::util;
use nih_plug_egui::egui;

use crate::dsp::spectrum::{BINS as SPECTRUM_BINS, DB_CEIL, DB_FLOOR};
use crate::params::NinerParams;
use crate::ui::knob;
use crate::ui::theme;
use crate::ui::widgets::{
    attach_midi_learn_menu_for_param, draw_groove, draw_inset_display_no_glass, draw_led,
    draw_rack_ear, draw_screw, lit_rect_default, param_knob, param_knob_compact,
};

pub const KNOB_SIZE: f32 = 32.0;
pub const KNOB_SPACING: f32 = 52.0;
pub const RACK_EAR_W: f32 = 16.0;
pub const CONTENT_LEFT: f32 = RACK_EAR_W + 14.0;
pub const HEADER_H: f32 = 28.0;
/// Uniform chrome-button height. Every chrome cap (TEST/SAVE/DEL/PLAY/STOP/
/// CLEAR/BOUNCE/DICE, preset arrows, SAT-row LCD arrows, the preset
/// display, the SAT-row LCDs) renders at this height so the panel reads as
/// a coherent piece of hardware rather than a collection of mismatched
/// widgets.
pub const CHROME_H: f32 = 22.0;
/// Square chrome cap dimension (preset arrows, SAT-row LCD arrows). Same
/// as CHROME_H — these are square buttons.
pub const CHROME_SQ: f32 = 22.0;

/// Which view the OUTPUT display is currently showing. Toggled by clicking
/// the display itself. Lives in egui `Memory` (temp) so it persists across
/// widget-tree rebuilds within a session and resets on full plugin reopen.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DisplayMode {
    Waveform,
    Spectrum,
    Off,
}

impl DisplayMode {
    fn toggled(self) -> Self {
        match self {
            DisplayMode::Waveform => DisplayMode::Spectrum,
            DisplayMode::Spectrum => DisplayMode::Off,
            DisplayMode::Off => DisplayMode::Waveform,
        }
    }
}

fn display_mode_id() -> egui::Id {
    egui::Id::new("output_display_mode")
}

/// Read the current OUTPUT-display mode from egui Memory. Defaults to
/// Waveform if the key has never been set.
pub fn display_mode(ctx: &egui::Context) -> DisplayMode {
    ctx.data_mut(|d| d.get_temp::<DisplayMode>(display_mode_id()))
        .unwrap_or(DisplayMode::Waveform)
}

fn set_display_mode(ctx: &egui::Context, mode: DisplayMode) {
    ctx.data_mut(|d| d.insert_temp(display_mode_id(), mode));
}

/// Draw the panel background, rack ears, screws, bevels, and title strip.
/// Returns the vertical center of the header band, which downstream code
/// uses to align the preset bar and test button.
///
/// When `chassis` is `Some`, the baked photoreal chassis PNG replaces the
/// procedural `BG_PANEL` fill. As the bake grows in subsequent iterations,
/// more procedural chrome (ears, screws, edge bands, grooves) will move
/// into the texture and the corresponding draw calls below will gate on
/// `chassis.is_none()`. When `None` (texture failed to decode/load), the
/// full procedural fallback runs unchanged.
pub fn draw_chrome(
    ui: &mut egui::Ui,
    panel_rect: egui::Rect,
    chassis: Option<&egui::TextureHandle>,
    screws: Option<&egui::TextureHandle>,
) -> f32 {
    let w = panel_rect.width();
    let h = panel_rect.height();
    // Sit the header strip below the recessed top edge band (y < 12) so it's
    // fully on the panel surface and clears the OUTPUT/COMP bezel tops at y=38.
    let header_center_y = panel_rect.top() + 23.0;

    // Precompute instrument_text positions before the painter borrow.
    let test_right = panel_rect.left()
        + CONTENT_LEFT
        + 111.0
        + 40.0
        + crate::ui::layout_overrides::offset_for(ui.ctx(), "header.test_btn").x;
    let kick_pos = crate::ui::layout_overrides::instrument_text(
        ui,
        "header.kick_synthesizer",
        egui::pos2(test_right + 66.0, header_center_y),
        egui::vec2(120.0, 12.0),
        egui::Align2::LEFT_CENTER,
    );
    let ver_pos = crate::ui::layout_overrides::instrument_text(
        ui,
        "header.version",
        egui::pos2(test_right + 162.0, header_center_y),
        egui::vec2(50.0, 12.0),
        egui::Align2::LEFT_CENTER,
    );

    // Pre-fetch screw offsets (read-only) before the painter borrow.
    // Base angles give each corner a distinct realistic rotation.
    const SCREW_BASE_ANGLES: [f32; 4] = [0.30, 1.05, 0.68, 1.82]; // TL TR BL BR
    const SCREW_R: f32 = 7.9; // sized to cover the baked circle + match user's BL resize
    let screw_bases = [
        (panel_rect.left() + 8.0, panel_rect.top() + 18.0),
        (panel_rect.right() - 8.0, panel_rect.top() + 18.0),
        (panel_rect.left() + 8.0, panel_rect.bottom() - 18.0),
        (panel_rect.right() - 8.0, panel_rect.bottom() - 18.0),
    ];
    let screw_keys = ["screw.tl", "screw.tr", "screw.bl", "screw.br"];
    let screw_offsets: [egui::Vec2; 4] =
        std::array::from_fn(|i| crate::ui::layout_overrides::offset_for(ui.ctx(), screw_keys[i]));
    let screw_scales: [f32; 4] = std::array::from_fn(|i| {
        crate::ui::layout_overrides::override_for(ui.ctx(), screw_keys[i]).size_scale
    });

    let painter = ui.painter();
    if let Some(t) = chassis {
        painter.image(
            t.id(),
            panel_rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
        // Composite real screws over the (possibly clean) plate. Gated on
        // SCREWS_BAKED so the 1×1 placeholder in tree doesn't paint.
        if crate::ui::widgets::SCREWS_BAKED.load(std::sync::atomic::Ordering::Relaxed) {
            if let Some(s) = screws {
                painter.image(
                    s.id(),
                    panel_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }
        }
    } else {
        // Procedural fallback: BG fill + edge bands + rack ears + screws.
        // All baked into the texture in the `Some` branch.
        painter.rect_filled(panel_rect, 0.0, theme::BG_PANEL);
        painter.rect_filled(
            egui::Rect::from_min_size(panel_rect.min, egui::vec2(w, 12.0)),
            0.0,
            theme::BG_PANEL_EDGE,
        );
        painter.rect_filled(
            egui::Rect::from_min_size(
                egui::pos2(panel_rect.left(), panel_rect.bottom() - 12.0),
                egui::vec2(w, 12.0),
            ),
            0.0,
            theme::BG_PANEL_EDGE,
        );
        draw_rack_ear(painter, panel_rect.left(), panel_rect.top(), RACK_EAR_W, h);
        draw_rack_ear(
            painter,
            panel_rect.right() - RACK_EAR_W,
            panel_rect.top(),
            RACK_EAR_W,
            h,
        );
        let screw_r = 5.0;
        for (sx, sy) in [
            (panel_rect.left() + 8.0, panel_rect.top() + 18.0),
            (panel_rect.left() + 8.0, panel_rect.bottom() - 18.0),
            (panel_rect.right() - 8.0, panel_rect.top() + 18.0),
            (panel_rect.right() - 8.0, panel_rect.bottom() - 18.0),
        ] {
            draw_screw(painter, sx, sy, screw_r);
        }
    }

    // Draw hex screws on top of chassis (always, regardless of bake state).
    for i in 0..4 {
        let (bx, by) = screw_bases[i];
        let off = screw_offsets[i];
        let scale = screw_scales[i];
        crate::ui::widgets::draw_hex_screw(
            painter,
            bx + off.x,
            by + off.y,
            SCREW_R * scale,
            SCREW_BASE_ANGLES[i],
        );
    }

    painter.text(
        kick_pos,
        egui::Align2::LEFT_CENTER,
        "KICK SYNTHESIZER",
        egui::FontId::new(8.0, egui::FontFamily::Monospace),
        theme::TEXT_DIM,
    );
    painter.text(
        ver_pos,
        egui::Align2::LEFT_CENTER,
        format!("v{}", env!("CARGO_PKG_VERSION")),
        egui::FontId::new(8.0, egui::FontFamily::Monospace),
        theme::TEXT_DIM,
    );
    // Register screw instruments AFTER painter is last used (NLL: borrow ends here).
    // Registering after the chassis/procedural content ensures screws are on top
    // of everything in the Foreground hit-test layer.
    for i in 0..4 {
        let (bx, by) = screw_bases[i];
        let off = screw_offsets[i];
        let scale = screw_scales[i];
        let base_rect = egui::Rect::from_center_size(
            egui::pos2(bx, by),
            egui::vec2(SCREW_R * 2.0 * scale, SCREW_R * 2.0 * scale),
        );
        let _ =
            crate::ui::layout_overrides::instrument(ui, screw_keys[i], base_rect).translate(off); // instrument already applies offset internally
        let _ = off; // suppress unused warning
    }
    header_center_y
}

/// Draw the "TEST" button in the header. Returns true if it was clicked
/// this frame (so the caller can dispatch a trigger).
pub fn test_button(ui: &mut egui::Ui, panel_rect: egui::Rect, header_center_y: f32) -> bool {
    let btn_x = panel_rect.left() + CONTENT_LEFT + 111.0;
    let btn_w = 40.0;
    let btn_h = crate::ui::layout_overrides::chrome_height(ui.ctx());
    let btn_y = header_center_y - btn_h * 0.5;
    let btn_rect = crate::ui::layout_overrides::instrument(
        ui,
        "header.test_btn",
        egui::Rect::from_min_size(egui::pos2(btn_x, btn_y), egui::vec2(btn_w, btn_h)),
    );
    let resp = ui.interact(
        btn_rect,
        egui::Id::new("test_trigger"),
        egui::Sense::click(),
    );
    let pressed = resp.is_pointer_button_down_on();
    let press_amount =
        ui.ctx()
            .animate_bool_with_time(egui::Id::new("test_btn_anim"), pressed, 0.06);
    {
        let painter = ui.painter();
        let r = crate::ui::layout_overrides::chrome_rounding(ui.ctx(), 3.0);
        crate::ui::widgets::draw_button_3d(painter, btn_rect, press_amount, r);
        let text_offset = press_amount * crate::ui::widgets::BTN_PRESS_TRAVEL;
        painter.text(
            btn_rect.center() + egui::vec2(0.0, text_offset),
            egui::Align2::CENTER_CENTER,
            "TEST",
            egui::FontId::new(10.0, egui::FontFamily::Monospace),
            theme::WHITE,
        );
    }
    let clicked = resp.clicked();
    if resp.hovered() {
        resp.on_hover_text_at_pointer("hit T to trigger");
    }
    clicked
}

/// Draw the master row (OUTPUT waveform / spectrum display + master knobs).
///
/// The OUTPUT display can render either a rolling peak waveform or a 64-band
/// log-frequency spectrum with peak-hold dots; clicking the display itself
/// toggles between the two.
pub struct MasterRow<'a> {
    pub master_y: f32,
    pub wf_left: f32,
    pub wf_width: f32,
    pub wf_height: f32,
    /// Two-slice view over the editor's waveform ring buffer. Concatenate
    /// `older` then `newer` for oldest-first iteration. Either slice can
    /// be empty; their combined length is the number of valid points.
    pub waveform_peaks_older: &'a [f32],
    pub waveform_peaks_newer: &'a [f32],
    /// Latest dB-per-band snapshot from the audio thread. Indices 0..BINS,
    /// values in `[DB_FLOOR, DB_CEIL]` (i.e. -60..0).
    pub spectrum_bins: &'a [f32; SPECTRUM_BINS],
    /// Decayed peak-hold line for the spectrum view. Same units as `spectrum_bins`.
    pub spectrum_peak_hold: &'a [f32; SPECTRUM_BINS],
    /// Which of the two views to draw this frame.
    pub display_mode: DisplayMode,
    /// Smoothed gain reduction (positive dB) from the master-bus compressor,
    /// for the GR overlay bar drawn on top of the OUTPUT display.
    pub gr_db: f32,
    /// Cycles-baked glass reflection PNG (`assets/display_reflection.png`),
    /// painted over the lit content as the final layer when present. When
    /// `None`, the procedural sheen drawn inside `draw_inset_display`
    /// remains the visible glass effect.
    pub display_reflection: Option<&'a egui::TextureHandle>,
}

impl<'a> MasterRow<'a> {
    pub fn draw(
        &self,
        ui: &mut egui::Ui,
        setter: &ParamSetter,
        params: &NinerParams,
        panel_rect: egui::Rect,
    ) {
        // The full OUTPUT display rect — also the hit target for the
        // waveform↔spectrum toggle. Interact *before* painting so clicks
        // register on the z-top layer regardless of which content we draw.
        // The instrument() wrapper applies any layout-editor offset; we
        // pull dx/dy out of the resulting rect and shift the local
        // wf_left / master_y so all downstream painters move with it.
        let base_display_rect = egui::Rect::from_min_size(
            egui::pos2(self.wf_left, self.master_y),
            egui::vec2(self.wf_width, self.wf_height),
        );
        let display_rect =
            crate::ui::layout_overrides::instrument(ui, "master.output_display", base_display_rect);
        let display_dx = display_rect.left() - base_display_rect.left();
        let display_dy = display_rect.top() - base_display_rect.top();
        let wf_left = self.wf_left + display_dx;
        let master_y = self.master_y + display_dy;
        let wf_width = self.wf_width;
        let wf_height = self.wf_height;
        // Visual display width. Base = midpoint between previous attempts.
        // size_scale from the layout editor's corner-drag resize is applied so
        // the user can widen/narrow the display live.
        let display_scale =
            crate::ui::layout_overrides::override_for(ui.ctx(), "master.output_display").size_scale;
        let display_paint_w = (wf_width - 6.0) * display_scale;
        let toggle_resp = ui
            .interact(
                display_rect,
                egui::Id::new("output_display_toggle"),
                egui::Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text("Click to cycle: bars / spectrum / off");
        if toggle_resp.clicked() {
            set_display_mode(ui.ctx(), self.display_mode.toggled());
        }

        // Compute lit early so instrument() can borrow ui mutably before the
        // painter immutable borrow is taken. Uses display_paint_w so lit
        // extends to the DECAY knob right edge while knobs_x is unchanged.
        let lit = lit_rect_default(wf_left, master_y, display_paint_w, wf_height);
        // GR section independent instrument — must be before painter borrow.
        let gr_base_rect = egui::Rect::from_min_size(
            egui::pos2(lit.left(), lit.top() + 2.0),
            egui::vec2(lit.width(), 11.0),
        );
        let gr_instrumented =
            crate::ui::layout_overrides::instrument(ui, "master.gr_section", gr_base_rect);
        let gr_dx = gr_instrumented.left() - gr_base_rect.left();
        let gr_dy = gr_instrumented.top() - gr_base_rect.top();

        // LIM chip base rect computed before painter. The interact is registered
        // AFTER master.comp_macro (below) so it wins Foreground click priority.
        // offset_for() (read-only) gives us the saved offset for drawing.
        let lim_base_rect = {
            let kx = wf_left + wf_width + 16.0;
            let sx = kx + KNOB_SPACING * 3.0 + 4.0;
            let sw = (panel_rect.right() - CONTENT_LEFT - sx).max(0.0);
            let comp_lit =
                crate::ui::widgets::lit_rect_default(sx + 2.0, master_y, sw - 4.0, wf_height);
            let base_cx = panel_rect.right() - CONTENT_LEFT - 4.0;
            let base_cy = comp_lit.top() + 4.0;
            egui::Rect::from_center_size(egui::pos2(base_cx, base_cy), egui::vec2(34.0, 12.0))
        };
        let lim_off = crate::ui::layout_overrides::offset_for(ui.ctx(), "master.lim_chip");

        let painter = ui.painter();
        // Display background ends at the DECAY knob right edge (display_paint_w).
        // DECAY/DRIFT/VOL and AMT/RCT/DRV sit on the chassis, not the display.
        painter.rect_filled(
            egui::Rect::from_min_size(
                egui::pos2(wf_left, master_y),
                egui::vec2(display_paint_w, wf_height),
            ),
            3.0,
            theme::BG_DISPLAY,
        );
        // Inset display under-content (frame + lit BG + scan-lines + red
        // ambient glow). Glass reflection is painted later, after bars/
        // waveform, so it sits on top of the lit content like real glass.
        draw_inset_display_no_glass(
            painter,
            wf_left,
            master_y,
            display_paint_w,
            wf_height,
            crate::ui::widgets::DisplayInsets::DEFAULT,
        );
        let mode_label = match self.display_mode {
            DisplayMode::Waveform => "OUTPUT",
            DisplayMode::Spectrum => "SPECTRUM",
            DisplayMode::Off => "OFF",
        };
        painter.text(
            egui::pos2(lit.left() + 2.0, lit.top() + 1.0),
            egui::Align2::LEFT_TOP,
            mode_label,
            egui::FontId::new(6.0, egui::FontFamily::Monospace),
            theme::RED_GHOST,
        );
        match self.display_mode {
            DisplayMode::Waveform => {
                let total = self.waveform_peaks_older.len() + self.waveform_peaks_newer.len();
                if total > 0 {
                    let n = total;
                    let mid_y = lit.top() + lit.height() / 2.0;
                    for (i, &peak) in self
                        .waveform_peaks_older
                        .iter()
                        .chain(self.waveform_peaks_newer.iter())
                        .enumerate()
                    {
                        let x = lit.left() + 2.0 + (i as f32 / n as f32) * (lit.width() - 4.0);
                        let amp = peak.min(1.0) * lit.height() * 0.475;
                        painter.line_segment(
                            [egui::pos2(x, mid_y - amp), egui::pos2(x, mid_y + amp)],
                            egui::Stroke::new(1.2, theme::RED_WAVEFORM),
                        );
                    }
                }
            }
            DisplayMode::Off => {}
            DisplayMode::Spectrum => {
                // Leave ~10 px at the top clear for the GR overlay + label;
                // bars grow up from `bars_bottom` toward `bars_top`.
                let bars_bottom = lit.bottom() - 2.0;
                let bars_top = lit.top() + 10.0;
                let plot_h = (bars_bottom - bars_top).max(1.0);
                let db_span = DB_CEIL - DB_FLOOR; // 60 dB
                let usable_w = (lit.width() - 4.0).max(1.0);
                let bar_slot = usable_w / SPECTRUM_BINS as f32;
                // 1 px gap between bars if the slot is wide enough; otherwise
                // draw a flush 1 px bar so low-width displays still render.
                let bar_w = (bar_slot - 1.0).max(1.0);
                for i in 0..SPECTRUM_BINS {
                    let db = self.spectrum_bins[i].clamp(DB_FLOOR, DB_CEIL);
                    let norm = ((db - DB_FLOOR) / db_span).clamp(0.0, 1.0);
                    let x = lit.left() + 2.0 + i as f32 * bar_slot;
                    let h = norm * plot_h;
                    if h > 0.5 {
                        painter.rect_filled(
                            egui::Rect::from_min_size(
                                egui::pos2(x, bars_bottom - h),
                                egui::vec2(bar_w, h),
                            ),
                            0.0,
                            theme::RED_WAVEFORM,
                        );
                    }

                    // Peak-hold dot — a 1.2 px slab at the current hold level.
                    let hold_db = self.spectrum_peak_hold[i].clamp(DB_FLOOR, DB_CEIL);
                    let hold_norm = ((hold_db - DB_FLOOR) / db_span).clamp(0.0, 1.0);
                    if hold_norm > (norm + 0.005) {
                        let hold_y = bars_bottom - hold_norm * plot_h;
                        painter.rect_filled(
                            egui::Rect::from_min_size(
                                egui::pos2(x, hold_y),
                                egui::vec2(bar_w, 1.2),
                            ),
                            0.0,
                            theme::RED_LED,
                        );
                    }
                }
            }
        }

        // Cycles-baked glass reflection overlay. Painted AFTER bars /
        // waveform so the highlights sit on top of lit content like real
        // glass; painted BEFORE the GR overlay + mode label so those stay
        // crisp and readable above the reflection.
        if let Some(handle) = self.display_reflection {
            painter.image(
                handle.id(),
                lit,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }

        // ── Gain-reduction overlay bar ──
        // Painted along the top of the lit rect, just under the mode
        // label. Fills right-to-left, 0..18 dB of reduction maps to 0..full
        // width. gr_dx/gr_dy come from the instrument() call above the
        // painter borrow, so the GR section moves independently.
        {
            let gr_max_db = 18.0f32;
            let gr_norm = (self.gr_db / gr_max_db).clamp(0.0, 1.0);
            // "GR" label left-aligned with BPM text (lit.left() + 2).
            let gr_label_x = lit.left() + 2.0 + gr_dx;
            let bar_x_base = lit.left() + 14.0; // bar starts just right of "GR"
            let bar_y_base = lit.top() + 2.0;
            let bar_w_total = lit.right() - bar_x_base - 2.0;
            let bar_h = 3.0;
            let bar_x = bar_x_base + gr_dx;
            let bar_y = lit.top() + 2.0 + gr_dy;
            // "GR" left-aligned with BPM, vertically centred on bar.
            painter.text(
                egui::pos2(gr_label_x, bar_y + bar_h * 0.5),
                egui::Align2::LEFT_CENTER,
                "GR",
                egui::FontId::new(6.0, egui::FontFamily::Monospace),
                theme::RED_LED,
            );
            // Housing (dim red, full width).
            painter.rect_filled(
                egui::Rect::from_min_size(egui::pos2(bar_x, bar_y), egui::vec2(bar_w_total, bar_h)),
                1.0,
                theme::RED_AMBIENT,
            );
            // Active fill (right-to-left).
            if gr_norm > 0.0 {
                let fill_w = gr_norm * bar_w_total;
                let fill_color = if gr_norm < 0.33 {
                    theme::RED_WAVEFORM
                } else {
                    theme::RED_LED
                };
                painter.rect_filled(
                    egui::Rect::from_min_size(
                        egui::pos2(bar_x + bar_w_total - fill_w, bar_y),
                        egui::vec2(fill_w, bar_h),
                    ),
                    1.0,
                    fill_color,
                );
            }
            // Tick marks + labels every 3 dB. Major ticks (multiples of 6)
            // get a number; minor ticks (3, 9, 15) get a short tick line only.
            let label_y = bar_y + bar_h + 1.0;
            let label_font = egui::FontId::new(6.0, egui::FontFamily::Monospace);
            for db in [0.0f32, 3.0, 6.0, 9.0, 12.0, 15.0] {
                let frac = db / gr_max_db;
                let lx = bar_x + bar_w_total - frac * bar_w_total;
                painter.text(
                    egui::pos2(lx, label_y),
                    egui::Align2::CENTER_TOP,
                    format!("{}", db as i32),
                    label_font.clone(),
                    theme::RED_LED,
                );
            }
        }
        // Knob value readout — rendered via a temp data slot set by
        // knob.rs. Linger for ~500 ms after the last hover/drag so a quick
        // tweak doesn't blink the value off the moment you release. The
        // expiry timestamp travels alongside the text in ctx.data; we
        // request another repaint at expiry time so the value disappears
        // cleanly without needing further input to flush egui.
        let knob_text: Option<String> =
            ui.ctx().data(|d| d.get_temp(egui::Id::new("knob_display")));
        let expires: Option<std::time::Instant> = ui
            .ctx()
            .data(|d| d.get_temp(egui::Id::new("knob_display_expires")));
        if let (Some(text), Some(expires)) = (knob_text, expires) {
            let now = std::time::Instant::now();
            if now < expires {
                ui.ctx().request_repaint_after(expires - now);
                let readout_rect = egui::Rect::from_min_size(
                    egui::pos2(lit.right() - 158.0, lit.bottom() - 16.0),
                    egui::vec2(156.0, 14.0),
                );
                crate::ui::seven_seg::draw_7seg_text(ui.painter(), readout_rect, &text);
            }
        }

        // Master knobs strip to the right of the display
        let knob_row_y = master_y + 4.0;
        // Use self.wf_left (base, pre-instrument) so knobs stay fixed when the
        // display is resized or moved via the layout editor.
        let knobs_x = self.wf_left + wf_width + 16.0;
        let master_knob_rect = egui::Rect::from_min_size(
            egui::pos2(knobs_x, knob_row_y),
            egui::vec2(KNOB_SPACING * 3.0, KNOB_SIZE + 30.0),
        );
        let master_knob_rect =
            crate::ui::layout_overrides::instrument(ui, "master.knobs", master_knob_rect);
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(master_knob_rect), |ui| {
            ui.horizontal(|ui| {
                param_knob(
                    ui,
                    setter,
                    "decay",
                    "DECAY",
                    &params.decay_ms,
                    50.0,
                    3000.0,
                    400.0,
                    |v| format!("{v:.0}ms"),
                    KNOB_SIZE,
                    theme::SECTION_MASTER,
                );
                param_knob(
                    ui,
                    setter,
                    "drift",
                    "DRIFT",
                    &params.drift_amount,
                    0.0,
                    1.0,
                    0.0,
                    |v| format!("{:.0}%", v * 100.0),
                    KNOB_SIZE,
                    theme::SECTION_MASTER,
                );
                // Master volume is stored as gain and displayed in dB.
                // unmodulated_plain_value, not value() — master volume's
                // Logarithmic(10ms) smoother would otherwise drag the knob.
                let mut vol_db = util::gain_to_db(params.master_volume.unmodulated_plain_value());
                let resp = knob::knob(
                    ui,
                    egui::Id::new("master"),
                    &mut vol_db,
                    -60.0,
                    6.0,
                    0.0,
                    "VOL",
                    |v| {
                        if v <= -59.0 {
                            "-inf".into()
                        } else {
                            format!("{v:.1}dB")
                        }
                    },
                    KNOB_SIZE,
                    theme::SECTION_MASTER,
                );
                if resp.changed {
                    setter.begin_set_parameter(&params.master_volume);
                    setter.set_parameter(&params.master_volume, util::db_to_gain(vol_db));
                    setter.end_set_parameter(&params.master_volume);
                }
                if let Some(r) = resp.response.as_ref() {
                    attach_midi_learn_menu_for_param(r, &params.master_volume);
                }
            });
        });

        // ── Compressor strip (right of master knobs) ──
        // Occupies the free ~100 px slot at the far right of the master row.
        // Three macro knobs (AMT / REACT / DRIVE) + a clickable LIM LED.
        {
            let strip_x = knobs_x + KNOB_SPACING * 3.0 + 4.0;
            let strip_right = panel_rect.right() - CONTENT_LEFT;
            let strip_w = (strip_right - strip_x).max(0.0);
            if strip_w >= 80.0 {
                let comp_lit = lit_rect_default(strip_x + 2.0, master_y, strip_w - 4.0, wf_height);
                // LIM toggle — positioned at the right of the comp strip
                // (≈ DRV right edge). lim_off comes from the instrument()
                // call before the painter borrow so the chip is draggable.
                let lim_cx = lim_base_rect.center().x + lim_off.x;
                let lim_cy = lim_base_rect.center().y + lim_off.y;
                let lim_on = params.comp_limit_on.value();
                // COMP label — same font/size as LIM/CLAP/POST, movable.
                let comp_label_pos = crate::ui::layout_overrides::instrument_text(
                    ui,
                    "master.comp_label",
                    egui::pos2(comp_lit.left() + 2.0, lim_cy),
                    egui::vec2(40.0, 12.0),
                    egui::Align2::LEFT_CENTER,
                );
                ui.painter().text(
                    comp_label_pos,
                    egui::Align2::LEFT_CENTER,
                    "COMP",
                    egui::FontId::new(8.0, egui::FontFamily::Monospace),
                    theme::TEXT_DIM,
                );
                draw_led(ui.painter(), lim_cx, lim_cy, lim_on);
                ui.painter().text(
                    egui::pos2(lim_cx - 7.0, lim_cy),
                    egui::Align2::RIGHT_CENTER,
                    "LIM",
                    egui::FontId::new(8.0, egui::FontFamily::Monospace),
                    if lim_on {
                        theme::WHITE
                    } else {
                        theme::TEXT_DIM
                    },
                );
                let lim_rect = egui::Rect::from_center_size(
                    egui::pos2(lim_cx - 4.0, lim_cy),
                    egui::vec2(28.0, 12.0),
                );
                let lim_resp = ui.interact(
                    lim_rect,
                    egui::Id::new("comp_lim_toggle"),
                    egui::Sense::click(),
                );
                if lim_resp.clicked() {
                    setter.begin_set_parameter(&params.comp_limit_on);
                    setter.set_parameter(&params.comp_limit_on, !lim_on);
                    setter.end_set_parameter(&params.comp_limit_on);
                }

                // Three macro knobs, laid out in a tight horizontal row.
                // Sized to match the PRECISE strip directly below so the two
                // clusters read as a vertical pair, not as two unrelated
                // groups with different metrics.
                let small_knob = 18.0f32;
                let knob_cell_w = small_knob + 10.0;
                let row_w = knob_cell_w * 3.0 + 6.0;
                let row_x = strip_x + ((strip_w - row_w) * 0.5).max(4.0);
                let row_y = master_y + 14.0;
                let comp_rect = egui::Rect::from_min_size(
                    egui::pos2(row_x, row_y),
                    egui::vec2(row_w, small_knob + 24.0),
                );
                let comp_rect =
                    crate::ui::layout_overrides::instrument(ui, "master.comp_macro", comp_rect);
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(comp_rect), |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    ui.horizontal(|ui| {
                        param_knob_compact(
                            ui,
                            setter,
                            "comp_amt",
                            "AMT",
                            "",
                            &params.comp_amount,
                            0.0,
                            1.0,
                            0.0,
                            |v| format!("{:.0}%", v * 100.0),
                            small_knob,
                            theme::KNOB_METAL,
                        );
                        // RCT is a "link" macro: dragging it writes the
                        // legacy inverse-coupled atk/rel formula directly
                        // into comp_atk_ms / comp_rel_ms via the setter,
                        // so the precision knobs follow along. Dragging
                        // ATK or REL individually leaves RCT stale —
                        // that's the intended break-out behavior.
                        let rct_changed = param_knob_compact(
                            ui,
                            setter,
                            "comp_rct",
                            "RCT",
                            "",
                            &params.comp_react,
                            0.0,
                            1.0,
                            0.35,
                            |v| format!("{:.0}%", v * 100.0),
                            small_knob,
                            theme::KNOB_METAL,
                        );
                        if rct_changed {
                            // Read the user's dialled value, not the smoothed
                            // mid-ramp state (Linear(10ms) smoother).
                            let react = params.comp_react.unmodulated_plain_value();
                            let atk_ms = 30.0 + react * (1.5 - 30.0);
                            let rel_ms = 400.0 + react * (40.0 - 400.0);
                            setter.begin_set_parameter(&params.comp_atk_ms);
                            setter.set_parameter(&params.comp_atk_ms, atk_ms);
                            setter.end_set_parameter(&params.comp_atk_ms);
                            setter.begin_set_parameter(&params.comp_rel_ms);
                            setter.set_parameter(&params.comp_rel_ms, rel_ms);
                            setter.end_set_parameter(&params.comp_rel_ms);
                        }
                        param_knob_compact(
                            ui,
                            setter,
                            "comp_drv",
                            "DRV",
                            "",
                            &params.comp_drive,
                            0.0,
                            1.0,
                            0.0,
                            |v| format!("{:.0}%", v * 100.0),
                            small_knob,
                            theme::KNOB_METAL,
                        );
                    });
                });
            }
        }
        // Register LIM interact AFTER comp_macro so it wins Foreground priority.
        // Painter is no longer live here so &mut ui is available.
        crate::ui::layout_overrides::instrument(ui, "master.lim_chip", lim_base_rect);
    }
}

/// Draw the SUB | TOP row (labels + groove + divider + knobs).
/// Returns the y coordinate where the knob row starts (so callers can stack
/// the next row).
pub fn draw_sub_top_row(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    params: &NinerParams,
    panel_rect: egui::Rect,
    master_bottom_y: f32,
) -> f32 {
    let row_groove_y = master_bottom_y + 22.0;
    let row_knob_y = row_groove_y + 4.0;
    let divider_x = panel_rect.left() + CONTENT_LEFT + KNOB_SPACING * 5.0 - 6.0;

    {
        let painter = ui.painter();
        draw_groove(
            painter,
            panel_rect.left() + CONTENT_LEFT - 4.0,
            panel_rect.right() - CONTENT_LEFT + 4.0,
            row_groove_y,
        );
        painter.line_segment(
            [
                egui::pos2(divider_x, row_groove_y + 2.0),
                egui::pos2(divider_x, row_knob_y + KNOB_SIZE + 30.0),
            ],
            egui::Stroke::new(1.0, theme::DIVIDER),
        );
    }

    // SUB knobs
    let sub_knob_rect = egui::Rect::from_min_size(
        egui::pos2(panel_rect.left() + CONTENT_LEFT, row_knob_y),
        egui::vec2(KNOB_SPACING * 5.0, KNOB_SIZE + 30.0),
    );
    let sub_knob_rect = crate::ui::layout_overrides::instrument(ui, "row.sub.knobs", sub_knob_rect);
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(sub_knob_rect), |ui| {
        ui.horizontal(|ui| {
            param_knob(
                ui,
                setter,
                "s_g",
                "GAIN",
                &params.sub_gain,
                0.0,
                1.0,
                0.85,
                |v| format!("{:.0}%", v * 100.0),
                KNOB_SIZE,
                theme::SECTION_SUB,
            );
            param_knob(
                ui,
                setter,
                "s_fs",
                "START",
                &params.sub_fstart,
                20.0,
                800.0,
                150.0,
                |v| format!("{v:.0}Hz"),
                KNOB_SIZE,
                theme::SECTION_SUB,
            );
            param_knob(
                ui,
                setter,
                "s_fe",
                "END",
                &params.sub_fend,
                20.0,
                400.0,
                45.0,
                |v| format!("{v:.0}Hz"),
                KNOB_SIZE,
                theme::SECTION_SUB,
            );
            param_knob(
                ui,
                setter,
                "s_sw",
                "SWEEP",
                &params.sub_sweep_ms,
                5.0,
                500.0,
                60.0,
                |v| format!("{v:.0}ms"),
                KNOB_SIZE,
                theme::SECTION_SUB,
            );
            param_knob(
                ui,
                setter,
                "s_cv",
                "CURVE",
                &params.sub_sweep_curve,
                0.5,
                12.0,
                3.0,
                |v| format!("{v:.1}"),
                KNOB_SIZE,
                theme::SECTION_SUB,
            );
        });
    });

    // TOP knobs
    let top_knob_rect = egui::Rect::from_min_size(
        egui::pos2(
            panel_rect.left() + CONTENT_LEFT + KNOB_SPACING * 5.0,
            row_knob_y,
        ),
        egui::vec2(KNOB_SPACING * 5.0, KNOB_SIZE + 30.0),
    );
    let top_knob_rect = crate::ui::layout_overrides::instrument(ui, "row.top.knobs", top_knob_rect);
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(top_knob_rect), |ui| {
        ui.horizontal(|ui| {
            param_knob(
                ui,
                setter,
                "t_g",
                "GAIN",
                &params.top_gain,
                0.0,
                1.0,
                0.25,
                |v| format!("{:.0}%", v * 100.0),
                KNOB_SIZE,
                theme::SECTION_TOP,
            );
            param_knob(
                ui,
                setter,
                "t_dc",
                "DECAY",
                &params.top_decay_ms,
                1.0,
                50.0,
                6.0,
                |v| format!("{v:.1}ms"),
                KNOB_SIZE,
                theme::SECTION_TOP,
            );
            param_knob(
                ui,
                setter,
                "t_f",
                "FREQ",
                &params.top_freq,
                1000.0,
                8000.0,
                3500.0,
                |v| format!("{v:.0}Hz"),
                KNOB_SIZE,
                theme::SECTION_TOP,
            );
            param_knob(
                ui,
                setter,
                "t_bw",
                "bandwidth",
                &params.top_bw,
                0.2,
                3.0,
                1.5,
                |v| format!("{v:.1}oct"),
                KNOB_SIZE,
                theme::SECTION_TOP,
            );
            param_knob(
                ui,
                setter,
                "t_mt",
                "METAL",
                &params.top_metal,
                0.0,
                1.0,
                0.0,
                |v| format!("{:.0}%", v * 100.0),
                KNOB_SIZE,
                theme::SECTION_TOP,
            );
        });
    });

    // ── Precise compressor row (ATK / REL / KNE) ──
    // Parked in the empty grey gap to the right of the TOP knobs, aligned
    // x with the MASTER-row COMP strip so it reads as a vertical column:
    //     COMP (macros)   ← master row
    //     PRECISE         ← here
    //     CLAP            ← MID row
    {
        let small_knob = 18.0f32;
        // Match the COMP strip's x so the three clusters stack visually.
        let col_x = panel_rect.right() - CONTENT_LEFT - 96.0 + 4.0;
        // Align small-knob centers with the big TOP-row knob centers.
        let col_row_y = row_knob_y + (KNOB_SIZE - small_knob) * 0.5;

        let knob_cell_w = small_knob + 10.0;
        let row_w = knob_cell_w * 3.0 + 6.0;
        let knob_rect = egui::Rect::from_min_size(
            egui::pos2(col_x, col_row_y),
            egui::vec2(row_w, small_knob + 22.0),
        );
        let knob_rect = crate::ui::layout_overrides::instrument(ui, "top.precise_comp", knob_rect);
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(knob_rect), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.horizontal(|ui| {
                param_knob_compact(
                    ui,
                    setter,
                    "comp_atk",
                    "ATK",
                    "",
                    &params.comp_atk_ms,
                    0.3,
                    50.0,
                    20.0,
                    |v| {
                        if v < 10.0 {
                            format!("{v:.1}ms")
                        } else {
                            format!("{v:.0}ms")
                        }
                    },
                    small_knob,
                    theme::KNOB_METAL,
                );
                param_knob_compact(
                    ui,
                    setter,
                    "comp_rel",
                    "REL",
                    "",
                    &params.comp_rel_ms,
                    20.0,
                    800.0,
                    274.0,
                    |v| format!("{v:.0}ms"),
                    small_knob,
                    theme::KNOB_METAL,
                );
                param_knob_compact(
                    ui,
                    setter,
                    "comp_kne",
                    "KNE",
                    "",
                    &params.comp_knee_db,
                    0.0,
                    12.0,
                    6.0,
                    |v| format!("{v:.1}dB"),
                    small_knob,
                    theme::KNOB_METAL,
                );
            });
        });

        // PRECISE label — 8pt to match LIM/COMP/CLAP/POST, movable.
        let caption_y = col_row_y + small_knob + 32.0;
        let precise_pos = crate::ui::layout_overrides::instrument_text(
            ui,
            "top.precise_label",
            egui::pos2(col_x, caption_y),
            egui::vec2(56.0, 12.0),
            egui::Align2::LEFT_TOP,
        );
        ui.painter().text(
            precise_pos,
            egui::Align2::LEFT_TOP,
            "PRECISE",
            egui::FontId::new(8.0, egui::FontFamily::Monospace),
            theme::TEXT_DIM,
        );
    }

    // Section labels at the bottom of the section, inside the borders —
    // i.e., in the 14 px gap that used to hold the MID label above the
    // MID groove. Knobs stay at their original y; only the label moves.
    let label_y = row_knob_y + KNOB_SIZE + 34.0;
    let row_label_font = crate::ui::layout_overrides::label_font(ui.ctx(), 11.0);
    let label_size = egui::vec2(40.0, 14.0);
    let sub_pos = crate::ui::layout_overrides::instrument_text(
        ui,
        "label.sub",
        egui::pos2(panel_rect.left() + CONTENT_LEFT, label_y),
        label_size,
        egui::Align2::LEFT_TOP,
    );
    let top_pos = crate::ui::layout_overrides::instrument_text(
        ui,
        "label.top",
        egui::pos2(
            panel_rect.left() + CONTENT_LEFT + KNOB_SPACING * 5.0,
            label_y,
        ),
        label_size,
        egui::Align2::LEFT_TOP,
    );
    let painter = ui.painter();
    painter.text(
        sub_pos,
        egui::Align2::LEFT_TOP,
        "SUB",
        row_label_font.clone(),
        theme::WHITE,
    );
    painter.text(
        top_pos,
        egui::Align2::LEFT_TOP,
        "TOP",
        row_label_font,
        theme::WHITE,
    );

    row_knob_y + KNOB_SIZE + 34.0
}

/// Draw the MID row. Returns the bottom y of the row.
pub fn draw_mid_row(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    params: &NinerParams,
    panel_rect: egui::Rect,
    sub_top_bottom_y: f32,
) -> f32 {
    let row_groove_y = sub_top_bottom_y + 14.0;
    let row_knob_y = row_groove_y + 4.0;

    {
        let painter = ui.painter();
        draw_groove(
            painter,
            panel_rect.left() + CONTENT_LEFT - 4.0,
            panel_rect.right() - CONTENT_LEFT + 4.0,
            row_groove_y,
        );
    }

    let mid_knob_rect = egui::Rect::from_min_size(
        egui::pos2(panel_rect.left() + CONTENT_LEFT, row_knob_y),
        egui::vec2(KNOB_SPACING * 10.0, KNOB_SIZE + 30.0),
    );
    let mid_knob_rect = crate::ui::layout_overrides::instrument(ui, "row.mid.knobs", mid_knob_rect);
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(mid_knob_rect), |ui| {
        ui.horizontal(|ui| {
            param_knob(
                ui,
                setter,
                "m_g",
                "GAIN",
                &params.mid_gain,
                0.0,
                1.0,
                0.5,
                |v| format!("{:.0}%", v * 100.0),
                KNOB_SIZE,
                theme::SECTION_MID,
            );
            param_knob(
                ui,
                setter,
                "m_fs",
                "START",
                &params.mid_fstart,
                100.0,
                2000.0,
                400.0,
                |v| format!("{v:.0}Hz"),
                KNOB_SIZE,
                theme::SECTION_MID,
            );
            param_knob(
                ui,
                setter,
                "m_fe",
                "END",
                &params.mid_fend,
                50.0,
                800.0,
                120.0,
                |v| format!("{v:.0}Hz"),
                KNOB_SIZE,
                theme::SECTION_MID,
            );
            param_knob(
                ui,
                setter,
                "m_sw",
                "SWEEP",
                &params.mid_sweep_ms,
                3.0,
                300.0,
                30.0,
                |v| format!("{v:.0}ms"),
                KNOB_SIZE,
                theme::SECTION_MID,
            );
            param_knob(
                ui,
                setter,
                "m_cv",
                "CURVE",
                &params.mid_sweep_curve,
                0.5,
                12.0,
                4.0,
                |v| format!("{v:.1}"),
                KNOB_SIZE,
                theme::SECTION_MID,
            );
            param_knob(
                ui,
                setter,
                "m_dc",
                "DECAY",
                &params.mid_decay_ms,
                20.0,
                1000.0,
                150.0,
                |v| format!("{v:.0}ms"),
                KNOB_SIZE,
                theme::SECTION_MID,
            );
            param_knob(
                ui,
                setter,
                "m_tn",
                "TONE",
                &params.mid_tone_gain,
                0.0,
                1.0,
                0.7,
                |v| format!("{:.0}%", v * 100.0),
                KNOB_SIZE,
                theme::SECTION_MID,
            );
            param_knob(
                ui,
                setter,
                "m_ns",
                "NOISE",
                &params.mid_noise_gain,
                0.0,
                1.0,
                0.3,
                |v| format!("{:.0}%", v * 100.0),
                KNOB_SIZE,
                theme::SECTION_MID,
            );
            param_knob(
                ui,
                setter,
                "m_nc",
                "COLOR",
                &params.mid_noise_color,
                0.0,
                1.0,
                0.4,
                |v| format!("{:.0}%", v * 100.0),
                KNOB_SIZE,
                theme::SECTION_MID,
            );
            param_knob(
                ui,
                setter,
                "m_ph",
                "PHASE",
                &params.mid_phase_offset,
                0.0,
                360.0,
                0.0,
                |v| format!("{v:.0}\u{00b0}"),
                KNOB_SIZE,
                theme::SECTION_MID,
            );
        });
    });

    // 909-style CLAP layer — sits at the bottom of the right-hand comp
    // column (COMP → PRECISE → CLAP), aligned x with the master-row COMP
    // strip so all three clusters stack visually.
    {
        let small_knob = 18.0f32;
        let clap_cx = panel_rect.right() - CONTENT_LEFT - 80.0 + 4.0;
        let clap_on = params.clap_on.value();

        // Align small-knob centers with MID big-knob centers.
        let row_y = row_knob_y + (KNOB_SIZE - small_knob) * 0.5;
        let knob_cell_w = small_knob + 10.0;
        let row_w = knob_cell_w * 3.0 + 6.0;
        let row_x = clap_cx - 4.0;
        let knob_rect = egui::Rect::from_min_size(
            egui::pos2(row_x, row_y),
            egui::vec2(row_w, small_knob + 22.0),
        );
        let knob_rect = crate::ui::layout_overrides::instrument(ui, "mid.clap_cluster", knob_rect);
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(knob_rect), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.horizontal(|ui| {
                param_knob_compact(
                    ui,
                    setter,
                    "clap_lvl",
                    "LVL",
                    "",
                    &params.clap_level,
                    0.0,
                    1.5,
                    0.9,
                    |v| format!("{:.2}", v),
                    small_knob,
                    theme::KNOB_METAL,
                );
                param_knob_compact(
                    ui,
                    setter,
                    "clap_freq",
                    "FREQ",
                    "",
                    &params.clap_freq,
                    500.0,
                    5000.0,
                    1200.0,
                    |v| {
                        if v >= 1000.0 {
                            format!("{:.1}k", v / 1000.0)
                        } else {
                            format!("{:.0}", v)
                        }
                    },
                    small_knob,
                    theme::KNOB_METAL,
                );
                param_knob_compact(
                    ui,
                    setter,
                    "clap_tail",
                    "TAIL",
                    "",
                    &params.clap_tail_ms,
                    50.0,
                    400.0,
                    180.0,
                    |v| format!("{:.0}ms", v),
                    small_knob,
                    theme::KNOB_METAL,
                );
            });
        });

        // LED + CLAP toggle chip BELOW the knob row.
        // instrument() wrapper makes it draggable in the layout editor.
        // Pre-positioned at the TAIL knob right side (right of the 3-knob row).
        let chip_top = row_y + small_knob + 32.0;
        // CLAP chip — use offset_for() to get saved position, register interact
        // after the knob cluster so it wins Foreground priority.
        let clap_base_rect =
            egui::Rect::from_min_size(egui::pos2(clap_cx - 16.0, chip_top), egui::vec2(46.0, 14.0));
        let clap_off = crate::ui::layout_overrides::offset_for(ui.ctx(), "mid.clap_chip");
        let clap_cx_chip = clap_cx - 16.0 + clap_off.x;
        let clap_cy = chip_top + 5.0 + clap_off.y;
        let clap_color = if clap_on {
            theme::WHITE
        } else {
            theme::TEXT_DIM
        };
        ui.painter().text(
            egui::pos2(clap_cx_chip, clap_cy),
            egui::Align2::LEFT_CENTER,
            "CLAP",
            egui::FontId::new(8.0, egui::FontFamily::Monospace),
            clap_color,
        );
        draw_led(ui.painter(), clap_cx_chip + 28.0, clap_cy, clap_on);
        let clap_rect = egui::Rect::from_min_size(
            egui::pos2(clap_cx_chip, clap_cy - 7.0),
            egui::vec2(40.0, 14.0),
        );
        let clap_resp = ui.interact(
            clap_rect,
            egui::Id::new("clap_toggle"),
            egui::Sense::click(),
        );
        if clap_resp.clicked() {
            setter.begin_set_parameter(&params.clap_on);
            setter.set_parameter(&params.clap_on, !clap_on);
            setter.end_set_parameter(&params.clap_on);
        }
        if clap_resp.hovered() {
            clap_resp.on_hover_cursor(egui::CursorIcon::PointingHand);
        }
        // Register CLAP chip interact AFTER the knob cluster instrument()
        // so CLAP wins Foreground click priority.
        crate::ui::layout_overrides::instrument(ui, "mid.clap_chip", clap_base_rect);
    }

    // Section label at the bottom of the section, inside the borders —
    // i.e., in the 14 px gap that used to hold the SAT label above the
    // SAT/EQ groove. Knobs stay at their original y; only the label moves.
    let label_y = row_knob_y + KNOB_SIZE + 34.0;
    let mid_pos = crate::ui::layout_overrides::instrument_text(
        ui,
        "label.mid",
        egui::pos2(panel_rect.left() + CONTENT_LEFT, label_y),
        egui::vec2(40.0, 14.0),
        egui::Align2::LEFT_TOP,
    );
    ui.painter().text(
        mid_pos,
        egui::Align2::LEFT_TOP,
        "MID",
        crate::ui::layout_overrides::label_font(ui.ctx(), 11.0),
        theme::WHITE,
    );

    row_knob_y + KNOB_SIZE + 34.0
}

/// Draw the SAT | EQ row. Returns the bottom y of the row.
/// Result of drawing the SAT/EQ row. `eq_knob_y` is the y-coordinate
/// where the big EQ knobs render their tops — used by `editor.rs` to
/// anchor the small FILTER cluster (FILT/RES) so its 18 px knobs stay
/// vertically centred against the 32 px EQ knobs even as the SAT cluster
/// height changes between v0.5.x and v0.6.0. `next_y` is where the
/// following row
/// should start.
pub struct SatEqRowResult {
    pub next_y: f32,
    pub eq_knob_y: f32,
}

pub fn draw_sat_eq_row(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    params: &NinerParams,
    panel_rect: egui::Rect,
    mid_bottom_y: f32,
) -> SatEqRowResult {
    // Total vertical extent of the SAT/CLIP cluster (two stacked compact
    // sub-rows of selector + 18 px knobs with `param_knob_compact` label
    // packing). 78 px = 22 (compact knob box) + 13 (label) per sub-row,
    // ×2, +4 inter-row spacing, +6 buffer above the STEP groove.
    const SAT_CLUSTER_H: f32 = 78.0;

    let row_label_y = mid_bottom_y;
    let row_groove_y = row_label_y + 14.0;
    let row_knob_y = row_groove_y + 4.0;
    let eq_divider_x = panel_rect.left() + CONTENT_LEFT + KNOB_SPACING * 4.0 + 40.0;

    let row_label_font = crate::ui::layout_overrides::label_font(ui.ctx(), 11.0);
    let sat_label_pos = crate::ui::layout_overrides::instrument_text(
        ui,
        "label.sat",
        egui::pos2(
            panel_rect.left() + CONTENT_LEFT,
            row_knob_y + SAT_CLUSTER_H - 14.0,
        ),
        egui::vec2(40.0, 14.0),
        egui::Align2::LEFT_TOP,
    );
    let eq_label_pos = crate::ui::layout_overrides::instrument_text(
        ui,
        "label.eq",
        egui::pos2(eq_divider_x + 10.0, row_knob_y + SAT_CLUSTER_H - 14.0),
        egui::vec2(30.0, 14.0),
        egui::Align2::LEFT_TOP,
    );
    {
        let painter = ui.painter();
        // SAT label tucked into the bottom-left corner of the SAT section
        // (inside the borders, not above the separator). Sits below the
        // second sub-row of the SAT cluster but above the sat_eq_return
        // boundary — i.e., still inside the SAT section. EQ stays at the
        // top-left of its sub-section.
        painter.text(
            sat_label_pos,
            egui::Align2::LEFT_TOP,
            "SAT",
            row_label_font.clone(),
            theme::WHITE,
        );
        painter.text(
            eq_label_pos,
            egui::Align2::LEFT_TOP,
            "EQ",
            row_label_font,
            theme::WHITE,
        );
        draw_groove(
            painter,
            panel_rect.left() + CONTENT_LEFT - 4.0,
            panel_rect.right() - CONTENT_LEFT + 4.0,
            row_groove_y,
        );
        painter.line_segment(
            [
                egui::pos2(eq_divider_x, row_groove_y + 2.0),
                egui::pos2(eq_divider_x, row_knob_y + SAT_CLUSTER_H),
            ],
            egui::Stroke::new(1.0, theme::DIVIDER),
        );
    }

    // SAT/CLIP cluster — two stacked sub-rows of compact LCD selectors plus
    // small comp-cluster sized knobs in the **compact** label layout
    // (`param_knob_compact` shaves the knob box to `diameter + 4` and drops
    // the gap before the label). The two LCD selectors are visual siblings:
    // master-bus saturation on top, per-voice waveshaper on bottom. The
    // bottom sub-row's three knobs are the new v0.6.0 controls. Row height
    // grew from `KNOB_SIZE + 30` to `SAT_CLUSTER_H` (78px) to fit the
    // stack with a tight margin to the STEP row below; the EQ cluster on
    // the right keeps its full-size 32 px knobs unchanged for visual
    // continuity with the master / sub / top / mid rows above.
    let sat_rect = egui::Rect::from_min_size(
        egui::pos2(panel_rect.left() + CONTENT_LEFT, row_knob_y),
        egui::vec2(KNOB_SPACING * 4.0 + 60.0, SAT_CLUSTER_H),
    );
    let sat_rect = crate::ui::layout_overrides::instrument(ui, "row.sat.cluster", sat_rect);
    const SMALL_KNOB: f32 = 18.0;
    const SAT_MODES: &[&str] = &["OFF", "SOFt", "dIOdE", "tAPE"];
    const CLIP_MODES: &[&str] = &["OFF", "tAnH", "dIOdE", "CUbIC"];
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(sat_rect), |ui| {
        ui.spacing_mut().item_spacing = egui::vec2(2.0, 4.0);

        // Top sub-row: SAT MODE compact selector + small SAT DRV + SAT MIX
        ui.horizontal(|ui| {
            crate::ui::seven_seg::lcd_selector(
                ui,
                setter,
                &params.sat_mode,
                "sat_mode",
                SAT_MODES,
                true,
            );
            ui.add_space(8.0);
            param_knob_compact(
                ui,
                setter,
                "sat_d",
                "DRV",
                "Saturation drive — input gain into the master-bus shaper",
                &params.sat_drive,
                0.0,
                1.0,
                0.0,
                |v| format!("{:.0}%", v * 100.0),
                SMALL_KNOB,
                theme::SECTION_SAT,
            );
            param_knob_compact(
                ui,
                setter,
                "sat_x",
                "MIX",
                "Saturation mix — wet/dry blend on the master bus",
                &params.sat_mix,
                0.0,
                1.0,
                1.0,
                |v| format!("{:.0}%", v * 100.0),
                SMALL_KNOB,
                theme::SECTION_SAT,
            );
        });

        // Bottom sub-row: CLIP MODE compact selector + the three new v0.6.0
        // knobs. Each knob keeps its semantic section color so the eye can
        // tell the master-bus stage (SAT, oxide red) apart from per-voice
        // (CLIP DRV, oxide red) and from MID-related controls (N DECAY,
        // forest green). ACCENT inherits the master row's electric blue.
        ui.horizontal(|ui| {
            crate::ui::seven_seg::lcd_selector(
                ui,
                setter,
                &params.kick_clip_mode,
                "clip_mode",
                CLIP_MODES,
                true,
            );
            ui.add_space(8.0);
            param_knob_compact(
                ui,
                setter,
                "clip_d",
                "CDRV",
                "Clip drive — per-voice waveshaper amount, applied to SUB+MID before each layer's amp envelope (909-style soft-clip)",
                &params.kick_clip_drive,
                0.0,
                1.0,
                0.0,
                |v| format!("{:.0}%", v * 100.0),
                SMALL_KNOB,
                theme::SECTION_SAT,
            );
            param_knob_compact(
                ui,
                setter,
                "n_dec",
                "NDEC",
                "MID noise decay — independent envelope for the noise channel. Short (15-30 ms) gates noise to attack like a real 909; longer values keep noise in the tail",
                &params.mid_noise_decay_ms,
                1.0,
                400.0,
                30.0,
                |v| format!("{:.0}ms", v),
                SMALL_KNOB,
                theme::SECTION_MID,
            );
            param_knob_compact(
                ui,
                setter,
                "acc",
                "ACC",
                "Accent amount — how much shift-clicked steps in the STEP grid get boosted (amplitude + decay) when the sequencer fires them",
                &params.accent_amount,
                0.0,
                1.0,
                0.0,
                |v| format!("{:.0}%", v * 100.0),
                SMALL_KNOB,
                theme::SECTION_MASTER,
            );
        });
    });

    // EQ: 5 knobs
    let eq_rect = egui::Rect::from_min_size(
        egui::pos2(eq_divider_x + 10.0, row_knob_y),
        egui::vec2(KNOB_SPACING * 5.0, KNOB_SIZE + 30.0),
    );
    let eq_rect = crate::ui::layout_overrides::instrument(ui, "row.eq.knobs", eq_rect);
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(eq_rect), |ui| {
        ui.horizontal(|ui| {
            param_knob(
                ui,
                setter,
                "eq_t",
                "TILT",
                &params.eq_tilt_db,
                -6.0,
                6.0,
                0.0,
                |v| format!("{v:+.1}dB"),
                KNOB_SIZE,
                theme::SECTION_EQ,
            );
            param_knob(
                ui,
                setter,
                "eq_l",
                "LOW",
                &params.eq_low_boost_db,
                -3.0,
                9.0,
                0.0,
                |v| format!("{v:+.1}dB"),
                KNOB_SIZE,
                theme::SECTION_EQ,
            );
            param_knob(
                ui,
                setter,
                "eq_nf",
                "NOTCH",
                &params.eq_notch_freq,
                100.0,
                600.0,
                250.0,
                |v| format!("{v:.0}Hz"),
                KNOB_SIZE,
                theme::SECTION_EQ,
            );
            param_knob(
                ui,
                setter,
                "eq_nq",
                "Q",
                &params.eq_notch_q,
                0.0,
                10.0,
                0.0,
                |v| format!("{v:.1}"),
                KNOB_SIZE,
                theme::SECTION_EQ,
            );
            param_knob(
                ui,
                setter,
                "eq_nd",
                "DEPTH",
                &params.eq_notch_depth_db,
                0.0,
                20.0,
                12.0,
                |v| format!("{v:.0}dB"),
                KNOB_SIZE,
                theme::SECTION_EQ,
            );
        });
    });

    // ── BOUNCE button ──
    // Fills the otherwise-empty right-hand slot of the SAT/EQ row. Exports
    // one trigger of the current sound to disk as WAV/AIFF; the actual
    // render + file write is handled by `crate::export::export_one_shot`
    // in `editor.rs` when the click flag returned here is set.
    SatEqRowResult {
        next_y: row_knob_y + SAT_CLUSTER_H,
        eq_knob_y: row_knob_y,
    }
}

/// Result of one frame of the BOUNCE + CLEAR row.
#[derive(Default)]
pub struct BounceRowResult {
    pub bounce_clicked: bool,
    pub clear_clicked: bool,
}

/// Beveled "BOUNCE" + "CLEAR" buttons in the right gap of the SAT/EQ row.
/// CLEAR sits to the left of BOUNCE; clicking it wipes the sequencer
/// pattern. Both styled to match [`test_button`] so they read as part of
/// the same visual family, and both share PLAY's 40 × 22 footprint.
pub fn draw_bounce_button(
    ui: &mut egui::Ui,
    panel_rect: egui::Rect,
    top_y: f32,
) -> BounceRowResult {
    // Match PLAY + step-pad height/width so the seq-row chrome reads as a
    // single visual family.
    let btn_w = 40.0;
    let btn_h = crate::ui::layout_overrides::chrome_height(ui.ctx());
    // CLEAR's left edge sits at col_x so it's vertically aligned with
    // DICE (one row up). BOUNCE then sits 15 px to the right of CLEAR —
    // the same chassis gap that separates step-pad 16 from CLEAR. The
    // dice-row lock LEDs (S/M/T/X/E/C) are anchored to BOUNCE.left so the
    // whole right-half cluster (LEDs + BOUNCE) reads as one unit.
    let col_x = panel_rect.right() - CONTENT_LEFT - 96.0 + 4.0;
    let gap = 15.0;
    let clear_rect = crate::ui::layout_overrides::instrument(
        ui,
        "seq.clear_btn",
        egui::Rect::from_min_size(egui::pos2(col_x, top_y), egui::vec2(btn_w, btn_h)),
    );
    let btn_x = col_x + btn_w + gap;
    let btn_rect = crate::ui::layout_overrides::instrument(
        ui,
        "seq.bounce_btn",
        egui::Rect::from_min_size(egui::pos2(btn_x, top_y), egui::vec2(btn_w, btn_h)),
    );
    let clear_resp = ui.interact(clear_rect, egui::Id::new("seq_clear"), egui::Sense::click());
    let clear_press = clear_resp.is_pointer_button_down_on();
    let clear_press_amount =
        ui.ctx()
            .animate_bool_with_time(egui::Id::new("seq_clear_anim"), clear_press, 0.06);
    {
        let painter = ui.painter();
        let r = crate::ui::layout_overrides::chrome_rounding(ui.ctx(), 3.0);
        crate::ui::widgets::draw_button_3d(painter, clear_rect, clear_press_amount, r);
        let text_offset = clear_press_amount * crate::ui::widgets::BTN_PRESS_TRAVEL;
        painter.text(
            clear_rect.center() + egui::vec2(0.0, text_offset),
            egui::Align2::CENTER_CENTER,
            "CLEAR",
            egui::FontId::new(10.0, egui::FontFamily::Monospace),
            theme::WHITE,
        );
    }

    let resp = ui.interact(
        btn_rect,
        egui::Id::new("export_bounce"),
        egui::Sense::click(),
    );
    let pressed = resp.is_pointer_button_down_on();
    let press_amount =
        ui.ctx()
            .animate_bool_with_time(egui::Id::new("bounce_btn_anim"), pressed, 0.06);
    {
        let painter = ui.painter();
        let r = crate::ui::layout_overrides::chrome_rounding(ui.ctx(), 3.0);
        crate::ui::widgets::draw_button_3d(painter, btn_rect, press_amount, r);
        let text_offset = press_amount * crate::ui::widgets::BTN_PRESS_TRAVEL;
        painter.text(
            btn_rect.center() + egui::vec2(0.0, text_offset),
            egui::Align2::CENTER_CENTER,
            "BOUNCE",
            egui::FontId::new(10.0, egui::FontFamily::Monospace),
            theme::WHITE,
        );
    }
    let clicked = resp.clicked();
    let clear_clicked = clear_resp.clicked();
    if resp.hovered() {
        resp.on_hover_text_at_pointer("Export one hit to WAV/AIFF");
    }
    if clear_resp.hovered() {
        clear_resp.on_hover_text_at_pointer("Clear all 16 steps + accents");
    }
    BounceRowResult {
        bounce_clicked: clicked,
        clear_clicked,
    }
}

pub fn draw_filter_cluster(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    params: &NinerParams,
    panel_rect: egui::Rect,
    top_y: f32,
) {
    let col_x = panel_rect.right() - CONTENT_LEFT - 96.0 + 4.0;
    let small_knob = 18.0f32;

    // Align small-knob centers with SAT/EQ big-knob centers. The caller
    // passes top_y = row_knob_y + 4 (see editor.rs::filter_top calc), so
    // the center-match offset is (KNOB_SIZE - small_knob)/2 - 4 = 3.
    let knob_y = top_y + 3.0;
    let knob_cell_w = small_knob + 10.0;
    let row_w = knob_cell_w * 2.0 + 6.0;
    let knob_rect = egui::Rect::from_min_size(
        egui::pos2(col_x, knob_y),
        egui::vec2(row_w, small_knob + 22.0),
    );
    let knob_rect = crate::ui::layout_overrides::instrument(ui, "row.eq.filter_cluster", knob_rect);
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(knob_rect), |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.horizontal(|ui| {
            // unmodulated_plain_value — dj_filter_pos has Linear(5ms) smoother.
            let mut filt_val = params.dj_filter_pos.unmodulated_plain_value();
            let resp = knob::knob(
                ui,
                egui::Id::new("dj_filter_pos"),
                &mut filt_val,
                -1.0,
                1.0,
                0.0,
                "FILT",
                |v| {
                    if v.abs() < 0.001 {
                        "OFF".into()
                    } else {
                        let t = v.abs();
                        let freq = if v > 0.0 {
                            20.0 * (800.0f32 / 20.0).powf(t)
                        } else {
                            20000.0 * (200.0f32 / 20000.0).powf(t)
                        };
                        let prefix = if v > 0.0 { "HP" } else { "LP" };
                        if freq >= 1000.0 {
                            format!("{prefix}{:.1}k", freq / 1000.0)
                        } else {
                            format!("{prefix}{freq:.0}")
                        }
                    }
                },
                small_knob,
                theme::KNOB_METAL,
            );
            if resp.changed {
                setter.begin_set_parameter(&params.dj_filter_pos);
                setter.set_parameter(&params.dj_filter_pos, filt_val);
                setter.end_set_parameter(&params.dj_filter_pos);
            }
            if let Some(r) = resp.response.as_ref() {
                attach_midi_learn_menu_for_param(r, &params.dj_filter_pos);
            }

            param_knob(
                ui,
                setter,
                "dj_filt_res",
                "RES",
                &params.dj_filter_res,
                0.0,
                1.0,
                0.0,
                |v| format!("{:.0}%", v * 100.0),
                small_knob,
                theme::KNOB_METAL,
            );
        });
    });

    let caption_y = knob_y + small_knob + 32.0;

    // POST/PRE chip — instrument() wrapper makes it draggable.
    // Pre-positioned at the BOUNCE button right side.
    let bounce_right = col_x + 40.0 + 15.0 + 40.0; // col_x + CLEAR + gap + BOUNCE
    let post_base_x = bounce_right - 32.0; // label left-aligned with bounce right
    let post_base_rect =
        egui::Rect::from_min_size(egui::pos2(post_base_x, caption_y), egui::vec2(32.0, 10.0));
    let post_off = crate::ui::layout_overrides::offset_for(ui.ctx(), "filter.post_chip");
    let led_x = post_base_x + post_off.x;
    let led_y = caption_y + post_off.y;
    let pre_on = params.dj_filter_pre.value();
    let led_label = if pre_on { "PRE" } else { "POST" };
    let led_color = if pre_on {
        theme::RED_WAVEFORM
    } else {
        theme::TEXT_DIM
    };
    let led_rect = egui::Rect::from_min_size(egui::pos2(led_x, led_y), egui::vec2(32.0, 10.0));
    let led_resp = ui.interact(
        led_rect,
        egui::Id::new("dj_filter_pre_led"),
        egui::Sense::click(),
    );
    if led_resp.clicked() {
        setter.begin_set_parameter(&params.dj_filter_pre);
        setter.set_parameter(&params.dj_filter_pre, !pre_on);
        setter.end_set_parameter(&params.dj_filter_pre);
    }
    ui.painter().text(
        egui::pos2(led_x, led_y),
        egui::Align2::LEFT_TOP,
        led_label,
        egui::FontId::new(8.0, egui::FontFamily::Monospace),
        led_color,
    );
    draw_led(ui.painter(), led_x + 26.0, led_y + 4.0, pre_on);
    // Register POST chip interact AFTER filter cluster so it wins priority.
    crate::ui::layout_overrides::instrument(ui, "filter.post_chip", post_base_rect);
}

pub fn draw_dice_row(
    ui: &mut egui::Ui,
    panel_rect: egui::Rect,
    top_y: f32,
    locks: &std::sync::atomic::AtomicU8,
) -> bool {
    let col_x = panel_rect.right() - CONTENT_LEFT - 96.0 + 4.0;

    let btn_w = 40.0;
    let btn_h = crate::ui::layout_overrides::chrome_height(ui.ctx());
    let btn_rect = crate::ui::layout_overrides::instrument(
        ui,
        "header.dice_btn",
        egui::Rect::from_min_size(egui::pos2(col_x, top_y), egui::vec2(btn_w, btn_h)),
    );
    let resp = ui.interact(btn_rect, egui::Id::new("dice_btn"), egui::Sense::click());
    let pressed = resp.is_pointer_button_down_on();
    let press_amount =
        ui.ctx()
            .animate_bool_with_time(egui::Id::new("dice_btn_anim"), pressed, 0.06);
    {
        let painter = ui.painter();
        let r = crate::ui::layout_overrides::chrome_rounding(ui.ctx(), 2.0);
        crate::ui::widgets::draw_button_3d(painter, btn_rect, press_amount, r);
        let text_offset = press_amount * crate::ui::widgets::BTN_PRESS_TRAVEL;
        painter.text(
            btn_rect.center() + egui::vec2(0.0, text_offset),
            egui::Align2::CENTER_CENTER,
            "DICE",
            egui::FontId::new(10.0, egui::FontFamily::Monospace),
            theme::WHITE,
        );
    }
    let dice_clicked = resp.clicked();

    let labels = ["S", "M", "T", "X", "E", "C"];
    let current_locks = locks.load(std::sync::atomic::Ordering::Relaxed);
    // Align the 6 lock LEDs as one unit with the BOUNCE button below.
    // BOUNCE.left sits at col_x + btn_w + 15 (a 15-px chassis gap that
    // matches step-16 → CLEAR), so the LED row starts at the same x.
    // 6 LEDs at 7-px spacing fit exactly in the 40-px BOUNCE width
    // (5 gaps × 7 + ~5 LED diameter).
    let led_start_x = col_x + btn_w + 15.0;
    let led_spacing = 7.0;

    for (i, label) in labels.iter().enumerate() {
        let bit = 1u8 << i;
        let is_locked = current_locks & bit != 0;
        let lx = led_start_x + i as f32 * led_spacing;
        let ly = top_y + 2.0;

        let led_rect = egui::Rect::from_min_size(
            egui::pos2(lx - 1.0, ly - 1.0),
            egui::vec2(led_spacing, btn_h),
        );
        let led_resp = ui.interact(
            led_rect,
            egui::Id::new(("dice_lock", i)),
            egui::Sense::click(),
        );
        if led_resp.clicked() {
            locks.fetch_xor(bit, std::sync::atomic::Ordering::Relaxed);
        }

        let dot_color = if is_locked {
            theme::RED_WAVEFORM
        } else {
            egui::Color32::from_rgb(0x33, 0x22, 0x22)
        };
        ui.painter()
            .circle_filled(egui::pos2(lx + 3.0, ly + 2.0), 2.5, dot_color);

        ui.painter().text(
            egui::pos2(lx + 3.0, ly + 7.0),
            egui::Align2::CENTER_TOP,
            *label,
            egui::FontId::new(6.0, egui::FontFamily::Monospace),
            if is_locked {
                theme::WHITE
            } else {
                theme::TEXT_DIM
            },
        );
    }

    dice_clicked
}

/// UI-thread state for the tempo readout's interactive widget. Only
/// relevant in standalone mode (host-synced mode shows a read-only label).
///
/// State machine: Idle → Armed (single click) → Editing (double click).
/// Drag and arrow keys apply in Armed state; Editing uses a `TextEdit`.
#[derive(Default)]
pub struct TempoEditState {
    /// Single click armed the widget: arrow keys are live and the text
    /// is highlighted + underlined.
    pub armed: bool,
    /// Double click put the widget into text-entry mode.
    pub editing: bool,
    /// Scratch buffer for the `TextEdit` while `editing` is true.
    pub edit_buf: String,
    /// Accumulated unconsumed vertical drag pixels. Reset when drag ends.
    pub drag_accum: f32,
}

/// UI-thread state for the sequencer row that does not belong on the
/// shared `Sequencer` (which is audio↔GUI). Tracks the click-drag paint
/// mode (`Some(true)` = drawing, `Some(false)` = erasing, `None` = no
/// drag in progress) and the last step index the pointer painted — used
/// to fill in gaps on fast drags where the pointer jumps multiple pads
/// between frames.
///
/// `click_origin` records the step index + its prior on/accent state at
/// press time so that on release-without-drag we can apply the 909-style
/// 3-state cycle (empty → active → active+accent → empty). When the drag
/// extends to other steps, the origin's accent isn't cycled — drag means
/// "clear path", click means "advance state".
#[derive(Default)]
pub struct SequencerUiState {
    pub paint_mode: Option<bool>,
    pub last_painted: Option<usize>,
    pub click_origin: Option<(usize, bool, bool)>,
    pub tempo_edit: TempoEditState,
}

/// Draw the BPM readout at `pos`.
///
/// - Host-synced: plain "{:.0} BPM · HOST" label, no interaction.
/// - Standalone:
///     - Single click arms the widget (bright text + underline, arrow keys live)
///     - Double click enters text-entry mode (digits only, 3-char limit)
///     - Vertical drag scrubs at 2 px per BPM (up = faster)
///     - Armed + Left/Right: ∓10 / ±10 BPM; Shift for ±1 BPM
///     - Up/Down are reserved for preset prev/next (see `preset_bar.rs`)
///     - Clicking elsewhere returns to Idle
///
/// BPM is clamped to [40, 240] by `Sequencer::set_bpm`.
pub fn draw_tempo_widget(
    ui: &mut egui::Ui,
    pos: egui::Pos2,
    seq: &crate::sequencer::Sequencer,
    host_synced: bool,
    state: &mut TempoEditState,
) {
    // Host-synced: early-return with the read-only label.
    if host_synced {
        state.armed = false;
        state.editing = false;
        state.edit_buf.clear();
        state.drag_accum = 0.0;
        ui.painter().text(
            pos,
            egui::Align2::LEFT_TOP,
            format!("{:.0} BPM · HOST", seq.display_bpm()),
            egui::FontId::new(9.0, egui::FontFamily::Monospace),
            theme::RED_LED,
        );
        return;
    }

    // Fixed-size hit rect so the widget doesn't jitter as the number width
    // changes. Big enough for "240 BPM" in the 9 px mono font.
    let rect = egui::Rect::from_min_size(pos, egui::vec2(48.0, 12.0));
    let font = egui::FontId::new(9.0, egui::FontFamily::Monospace);

    // --- Editing branch: TextEdit grabs focus, Enter/Esc commit/cancel. ---
    if state.editing {
        let te_id = egui::Id::new("tempo_edit_textedit");
        let te = egui::TextEdit::singleline(&mut state.edit_buf)
            .id(te_id)
            .font(font.clone())
            .char_limit(3)
            .desired_width(30.0);
        let resp = ui.put(rect, te);

        // Keep focus on the TextEdit until committed / cancelled.
        if !resp.has_focus() && !resp.lost_focus() {
            resp.request_focus();
        }

        // Strip any non-digit characters the user typed/pasted.
        state.edit_buf.retain(|c| c.is_ascii_digit());
        if state.edit_buf.len() > 3 {
            state.edit_buf.truncate(3);
        }

        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
        let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));

        if enter {
            if let Ok(n) = state.edit_buf.parse::<u32>() {
                seq.set_bpm(n as f32); // set_bpm clamps to [40, 240]
            }
            state.editing = false;
            state.edit_buf.clear();
            // Armed remains true — user can keep nudging with arrows.
            state.armed = true;
        } else if esc || resp.lost_focus() {
            state.editing = false;
            state.edit_buf.clear();
            state.armed = true;
        }
        return;
    }

    // --- Idle / Armed branch: draw the text, then handle interaction. ---
    let color = theme::RED_LED;
    let text = format!("{:.0} BPM", seq.display_bpm());
    ui.painter()
        .text(pos, egui::Align2::LEFT_TOP, &text, font.clone(), color);

    // MIDI activity dot — sits to the right of the BPM number, brightens
    // on every incoming MIDI event and decays back over ~200 ms (intensity
    // is computed in editor.rs and stashed in ctx.data so we don't have
    // to thread the counter through every panel call). Not interactive,
    // not labelled — just a discreet pulse.
    let intensity: f32 = ui
        .ctx()
        .data(|d| d.get_temp(egui::Id::new("niner_midi_activity")).unwrap_or(0.0_f32));
    if intensity > 0.0 || seq.host_synced.load(std::sync::atomic::Ordering::Relaxed) {
        // Always allocate the same rect so layout doesn't shift between
        // host-synced and standalone modes — only the colour changes.
        let _ = intensity;
    }
    {
        let dot_centre = egui::pos2(pos.x + 56.0, pos.y + 6.0);
        // Off colour matches RED_AMBIENT (the same dim red used for the
        // GR meter housing); on colour is full RED_LED. Lerp gives us a
        // smooth attack-and-decay glow.
        let off = theme::RED_AMBIENT;
        let on = theme::RED_LED;
        let lerp_u8 = |a: u8, b: u8, t: f32| -> u8 {
            let v = a as f32 + (b as f32 - a as f32) * t;
            v.clamp(0.0, 255.0) as u8
        };
        let dot_color = egui::Color32::from_rgb(
            lerp_u8(off.r(), on.r(), intensity),
            lerp_u8(off.g(), on.g(), intensity),
            lerp_u8(off.b(), on.b(), intensity),
        );
        ui.painter().circle_filled(dot_centre, 1.8, dot_color);
    }

    // Underline when armed.
    if state.armed {
        let underline_y = pos.y + 11.0;
        ui.painter().line_segment(
            [
                egui::pos2(pos.x, underline_y),
                egui::pos2(pos.x + 42.0, underline_y),
            ],
            egui::Stroke::new(1.0, theme::RED_LED),
        );
    }

    // Allocate the interactive rect.
    let response = ui.interact(
        rect,
        egui::Id::new("tempo_edit_hit"),
        egui::Sense::click_and_drag(),
    );

    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }

    // Right-click on the BPM readout → MIDI Learn for the standalone
    // tempo. The binding is keyed on `sentinel::TEMPO`, which the
    // editor's apply path special-cases (skipping the normal
    // `id_to_ptr` lookup). Bound only here in standalone mode — the
    // host-synced branch returned early above, so this code can't
    // reach a DAW project.
    if let Some(learn) = crate::ui::widgets::fetch_midi_learn_ctx(ui.ctx()) {
        crate::ui::widgets::attach_midi_learn_menu_for_target(
            &response,
            &learn,
            crate::midi_map::sentinel::TEMPO,
        );
    }

    // Double click → enter text entry.
    if response.double_clicked() {
        state.editing = true;
        state.armed = true;
        state.edit_buf = format!("{:.0}", seq.display_bpm());
        state.drag_accum = 0.0;
        return;
    }

    // Single click → arm.
    if response.clicked() {
        state.armed = true;
    }

    // Vertical drag → scrub.
    if response.dragged() {
        state.armed = true;
        state.drag_accum += response.drag_delta().y;
        let px_per_bpm = 2.0;
        // Up (negative y) = faster.
        let delta_bpm = (-state.drag_accum / px_per_bpm).trunc() as i32;
        if delta_bpm != 0 {
            state.drag_accum += delta_bpm as f32 * px_per_bpm;
            let new_bpm = (seq.bpm().round() as i32 + delta_bpm).clamp(40, 240);
            seq.set_bpm(new_bpm as f32);
        }
    }
    if response.drag_stopped() {
        state.drag_accum = 0.0;
    }

    // Click outside our rect → disarm.
    if state.armed {
        let clicked_elsewhere = ui.input(|i| {
            i.pointer.any_pressed()
                && i.pointer
                    .interact_pos()
                    .map(|p| !rect.contains(p))
                    .unwrap_or(false)
        });
        if clicked_elsewhere {
            state.armed = false;
        }
    }

    // Arrow key handling — only when armed and not editing.
    //
    // Left/Right adjust tempo (±10, or ±1 with Shift). Up/Down are left
    // alone so the preset bar (`src/ui/preset_bar.rs`) keeps owning them
    // for prev/next preset navigation in both standalone and plugin mode.
    if state.armed {
        ui.input_mut(|i| {
            let right_10 = i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight);
            let right_1 = i.consume_key(egui::Modifiers::SHIFT, egui::Key::ArrowRight);
            let left_10 = i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft);
            let left_1 = i.consume_key(egui::Modifiers::SHIFT, egui::Key::ArrowLeft);
            let delta =
                (right_10 as i32) * 10 + (right_1 as i32) - (left_10 as i32) * 10 - (left_1 as i32);
            if delta != 0 {
                let new_bpm = (seq.bpm().round() as i32 + delta).clamp(40, 240);
                seq.set_bpm(new_bpm as f32);
            }
        });
    }
}

/// Draw the 16-step pattern sequencer row below the SAT/EQ row.
///
/// Layout (left to right):
///   [PLAY/STOP button] [BPM readout] [16 step pads]
///
/// Step state, running flag, BPM, and the current playhead are all stored
/// as atomics on the shared `Sequencer`, so this function is safe to call
/// from the UI thread with no locks. The `ui_state` argument carries the
/// click-drag paint mode across frames.
pub fn draw_sequencer_row(
    ui: &mut egui::Ui,
    _setter: &ParamSetter,
    _params: &NinerParams,
    panel_rect: egui::Rect,
    sat_eq_bottom_y: f32,
    seq: &crate::sequencer::Sequencer,
    ui_state: &mut SequencerUiState,
) {
    let row_label_y = sat_eq_bottom_y + 4.0;
    let row_groove_y = row_label_y + 14.0;
    let pad_top = row_groove_y + 6.0;
    let pad_h = crate::ui::layout_overrides::chrome_height(ui.ctx());
    let pad_w = 26.0;
    let pad_gap = 3.0;

    let host_synced = seq.is_host_synced();

    // Groove for the sequencer row.
    {
        let painter = ui.painter();
        draw_groove(
            painter,
            panel_rect.left() + CONTENT_LEFT - 4.0,
            panel_rect.right() - CONTENT_LEFT + 4.0,
            row_groove_y,
        );
    }

    // BPM readout has been hoisted to the lower-left corner of the master
    // display (see editor.rs after MasterRow.draw) so the SAT/EQ → seq
    // gap stays visually clean. The tempo state still lives in
    // `ui_state.tempo_edit` because click-to-arm and double-click-to-edit
    // are managed across frames there.

    // Play / stop button — click to toggle in standalone; shows (and is
    // disabled to) the effective host state in DAW mode.
    let play_w = 40.0;
    let play_rect = crate::ui::layout_overrides::instrument(
        ui,
        "seq.play_btn",
        egui::Rect::from_min_size(
            egui::pos2(panel_rect.left() + CONTENT_LEFT, pad_top),
            egui::vec2(play_w, pad_h),
        ),
    );
    let play_resp = ui.interact(
        play_rect,
        egui::Id::new("seq_play"),
        if host_synced {
            egui::Sense::hover()
        } else {
            egui::Sense::click()
        },
    );
    if play_resp.clicked() && !host_synced {
        seq.toggle_running();
    }
    // Right-click on the PLAY button → MIDI Learn for the sequencer
    // play toggle. Bind any pad / footswitch / encoder click to start
    // and stop the standalone sequencer remotely. No-op in host-synced
    // mode (host owns transport).
    if !host_synced {
        if let Some(learn) = crate::ui::widgets::fetch_midi_learn_ctx(ui.ctx()) {
            crate::ui::widgets::attach_midi_learn_menu_for_target(
                &play_resp,
                &learn,
                crate::midi_map::sentinel::SEQ_PLAY,
            );
        }
    }
    let running = seq.is_running_effective();
    // Running = button shows pushed-in (latched) so the user sees sequencer
    // state at a glance. Host-synced is a distinct mode and doesn't latch —
    // it just dims the label.
    let visually_pressed = running && !host_synced;
    // Slightly slower animation here than for the momentary buttons so the
    // running latch reads as deliberate state rather than a click flash.
    let play_press_amount =
        ui.ctx()
            .animate_bool_with_time(egui::Id::new("seq_play_anim"), visually_pressed, 0.10);
    {
        let painter = ui.painter();
        let r = crate::ui::layout_overrides::chrome_rounding(ui.ctx(), 3.0);
        crate::ui::widgets::draw_button_3d(painter, play_rect, play_press_amount, r);
        let label = if host_synced {
            "HOST"
        } else if running {
            "STOP"
        } else {
            "PLAY"
        };
        let label_color = if host_synced {
            theme::TEXT_DIM
        } else {
            theme::WHITE
        };
        let text_offset = play_press_amount * crate::ui::widgets::BTN_PRESS_TRAVEL;
        painter.text(
            play_rect.center() + egui::vec2(0.0, text_offset),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::new(10.0, egui::FontFamily::Monospace),
            label_color,
        );
    }

    // 16 step pads — centered after the play button, with a small gap.
    let pads_total_w = pad_w * 16.0 + pad_gap * 15.0;
    let base_pads_start_x = play_rect.right() + 12.0;
    let base_pads_top = pad_top;
    let pads_base_rect = egui::Rect::from_min_size(
        egui::pos2(base_pads_start_x, base_pads_top),
        egui::vec2(pads_total_w, pad_h),
    );
    let pads_rect = crate::ui::layout_overrides::instrument(ui, "seq.pads_cluster", pads_base_rect);
    let pads_start_x = pads_rect.left();
    let pad_top = pads_rect.top();

    // Snapshot pointer/button state once per frame for the drag logic.
    // Pointer state read once per frame. We track both buttons so the
    // drag-clear gesture works with either left or right press starting on
    // an already-active step.
    let (
        primary_down,
        primary_released,
        secondary_down,
        secondary_pressed,
        secondary_released,
        pointer_pos,
    ) = ui.input(|i| {
        (
            i.pointer.primary_down(),
            i.pointer.primary_released(),
            i.pointer.button_down(egui::PointerButton::Secondary),
            i.pointer.button_pressed(egui::PointerButton::Secondary),
            i.pointer.button_released(egui::PointerButton::Secondary),
            i.pointer.interact_pos(),
        )
    });
    let any_button_down = primary_down || secondary_down;
    let any_release = primary_released || secondary_released;
    // Release: apply the 3-state cycle for a single click on an active
    // step, then clear paint state. Drag (last_painted moved away from
    // origin) suppresses the cycle so a cross-row erase doesn't also flip
    // the start cell's accent on the way out.
    if any_release || !any_button_down {
        if let Some((origin, was_on, had_accent)) = ui_state.click_origin.take() {
            let dragged_far = ui_state.last_painted != Some(origin);
            if !dragged_far && was_on {
                if !had_accent {
                    // active(no accent) → active+accent
                    seq.toggle_accent(origin);
                } else {
                    // active+accent → empty (set_step also clears accent)
                    seq.set_step(origin, false);
                }
            }
            // was_on=false case: snap-on already happened at press time.
        }
        ui_state.paint_mode = None;
        ui_state.last_painted = None;
    }

    // Compute which step index (if any) the pointer is currently over
    // from geometry — this is used for the fill-between-frames pass so
    // fast drags that skip multiple pads in one frame still paint every
    // step in between. Geometric hit-testing also covers the small gaps
    // between pad rects, which `rect.contains` would otherwise miss.
    let pitch = pad_w + pad_gap;
    let row_top = pad_top;
    let row_bot = pad_top + pad_h;
    let hovered_step: Option<usize> = pointer_pos.and_then(|p| {
        if p.y < row_top - 4.0 || p.y > row_bot + 4.0 {
            return None;
        }
        let rel = p.x - pads_start_x;
        if rel < 0.0 {
            return None;
        }
        let idx = (rel / pitch) as usize;
        (idx < crate::sequencer::STEPS).then_some(idx)
    });

    let current = seq.current();
    for i in 0..crate::sequencer::STEPS {
        let x = pads_start_x + i as f32 * pitch;
        let rect = egui::Rect::from_min_size(egui::pos2(x, pad_top), egui::vec2(pad_w, pad_h));
        let id = egui::Id::new(("seq_step", i));
        let resp = ui.interact(rect, id, egui::Sense::click_and_drag());

        // Press handling. Two paths:
        //  - Primary press on an empty step: snap on immediately (matches
        //    real-hardware tactile feedback) and arm paint-on for drag.
        //  - Primary press on an active step: arm paint-clear for drag,
        //    but DON'T mutate state yet — the release decides between
        //    "single click cycle" (advance accent state) and "drag clear".
        //  - Secondary (right) press on any step: always paint-clear,
        //    snap-clear on the origin, never cycle.
        let primary_press = resp.drag_started() || resp.clicked();
        let secondary_press = resp.contains_pointer() && secondary_pressed;
        if primary_press {
            let was_on = seq.is_step_on(i);
            let had_accent = was_on && seq.is_step_accented(i);
            if was_on {
                ui_state.paint_mode = Some(false);
                ui_state.last_painted = Some(i);
                ui_state.click_origin = Some((i, true, had_accent));
            } else {
                ui_state.paint_mode = Some(true);
                ui_state.last_painted = Some(i);
                ui_state.click_origin = Some((i, false, false));
                seq.set_step(i, true);
            }
        } else if secondary_press {
            ui_state.paint_mode = Some(false);
            ui_state.last_painted = Some(i);
            ui_state.click_origin = None;
            if seq.is_step_on(i) {
                seq.set_step(i, false);
            }
        }

        let on = seq.is_step_on(i);
        let accented = on && seq.is_step_accented(i);
        let is_playhead = seq.is_running_effective() && i == current;
        let beat_marker = i % 4 == 0;

        // Momentary press animation — 60 ms travel matches the chrome
        // buttons up top. Reads the live `is_pointer_button_down_on` flag
        // (not the latched on/off state, which stays in the red body).
        let pressed_now = resp.is_pointer_button_down_on();
        let press =
            ui.ctx()
                .animate_bool_with_time(egui::Id::new(("seq_step_anim", i)), pressed_now, 0.06);
        let press_offset = egui::vec2(0.0, press * crate::ui::widgets::BTN_PRESS_TRAVEL);

        let painter = ui.painter();

        // Recessed well — chassis cutout the pad sits in. Painted before
        // the cap so the 1.5-px gap shows on all sides regardless of
        // on/off state.
        let well_rect = rect.expand(1.5);
        painter.rect_filled(well_rect, 3.0, theme::BTN_WELL);
        painter.line_segment(
            [
                egui::pos2(well_rect.left() + 1.0, well_rect.top() + 0.5),
                egui::pos2(well_rect.right() - 1.0, well_rect.top() + 0.5),
            ],
            egui::Stroke::new(0.6, theme::BTN_WELL_TOP_SHADOW),
        );
        painter.line_segment(
            [
                egui::pos2(well_rect.left() + 0.5, well_rect.top() + 1.0),
                egui::pos2(well_rect.left() + 0.5, well_rect.bottom() - 1.0),
            ],
            egui::Stroke::new(0.5, theme::BTN_WELL_TOP_SHADOW),
        );

        // Drop shadow under the cap — fades as the cap sinks.
        let shadow_alpha = ((1.0 - press) * 0x60 as f32) as u8;
        if shadow_alpha > 0 {
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(rect.left() + 0.5, rect.bottom() - 0.5),
                    egui::pos2(rect.right() + 1.0, rect.bottom() + 1.5),
                ),
                2.0,
                egui::Color32::from_rgba_premultiplied(0, 0, 0, shadow_alpha),
            );
        }

        // Cap body — translated downward by the press amount.
        let cap_rect = rect.translate(press_offset);
        let body_color = if on {
            theme::RED_WAVEFORM
        } else if beat_marker {
            egui::Color32::from_rgb(0x26, 0x22, 0x22)
        } else {
            egui::Color32::from_rgb(0x1a, 0x1a, 0x1a)
        };
        painter.rect_filled(cap_rect, 2.0, body_color);
        // Highlight stripe on top-half for a subtle bevel
        painter.rect_filled(
            egui::Rect::from_min_size(cap_rect.min, egui::vec2(pad_w, pad_h * 0.45)),
            2.0,
            if on {
                theme::RED_GHOST
            } else {
                egui::Color32::from_rgb(0x2a, 0x2a, 0x2a)
            },
        );
        // Tactile 1-px top sheen + 1-px bottom ledge. Top sheen alpha
        // tracks `1 - press` so the cap reads as no-longer-catching-light
        // when fully pressed.
        let sheen_alpha = ((1.0 - press) * 0x28 as f32) as u8;
        if sheen_alpha > 0 {
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(cap_rect.left() + 1.5, cap_rect.top() + 0.5),
                    egui::pos2(cap_rect.right() - 1.5, cap_rect.top() + 1.5),
                ),
                1.0,
                egui::Color32::from_rgba_premultiplied(0xff, 0xff, 0xff, sheen_alpha),
            );
        }
        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(cap_rect.left() + 1.5, cap_rect.bottom() - 1.5),
                egui::pos2(cap_rect.right() - 1.5, cap_rect.bottom() - 0.5),
            ),
            1.0,
            egui::Color32::from_rgba_premultiplied(0x00, 0x00, 0x00, 0x60),
        );

        // Playhead ring — anchored to the well, not the cap, so it stays
        // put while the cap depresses underneath it.
        if is_playhead {
            painter.rect_stroke(
                rect.expand(1.0),
                2.5,
                egui::Stroke::new(1.5, theme::WHITE),
                egui::StrokeKind::Outside,
            );
        }

        // Accent indicator — small bright tick at the bottom of the cap;
        // rides the cap so it visibly ducks when the pad is clicked.
        if accented {
            let tick_h = 3.0;
            let tick_inset = 4.0;
            let tick_rect = egui::Rect::from_min_max(
                egui::pos2(
                    cap_rect.left() + tick_inset,
                    cap_rect.bottom() - tick_h - 1.0,
                ),
                egui::pos2(cap_rect.right() - tick_inset, cap_rect.bottom() - 1.0),
            );
            painter.rect_filled(tick_rect, 1.0, theme::WHITE);
        }

        // Beat number (1, 5, 9, 13) in dim text for orientation; rides
        // the cap with everything else.
        if beat_marker {
            painter.text(
                egui::pos2(cap_rect.left() + 3.0, cap_rect.top() + 2.0),
                egui::Align2::LEFT_TOP,
                format!("{}", i + 1),
                egui::FontId::new(7.0, egui::FontFamily::Monospace),
                theme::TEXT_GHOST,
            );
        }

        if resp.hovered() {
            resp.on_hover_cursor(egui::CursorIcon::PointingHand);
        }
    }

    // Fast-drag fill: if a paint drag is in progress and the pointer has
    // jumped to a new step since the last frame, paint every step in the
    // inclusive range between the last painted index and the one the
    // pointer is currently over. Without this pass, a quick mouse swipe
    // across the row skips any pads the pointer wasn't literally over on
    // a rendered frame.
    if let (Some(mode), true, Some(hover_idx)) =
        (ui_state.paint_mode, any_button_down, hovered_step)
    {
        let from = ui_state.last_painted.unwrap_or(hover_idx);
        let (lo, hi) = if from <= hover_idx {
            (from, hover_idx)
        } else {
            (hover_idx, from)
        };
        for i in lo..=hi {
            if seq.is_step_on(i) != mode {
                seq.set_step(i, mode);
            }
        }
        ui_state.last_painted = Some(hover_idx);
    }
}

/// Draw the bottom footer groove.
pub fn draw_footer(ui: &egui::Ui, panel_rect: egui::Rect) {
    let painter = ui.painter();
    let footer_groove_y = panel_rect.bottom() - 22.0;
    draw_groove(
        painter,
        panel_rect.left() + CONTENT_LEFT - 4.0,
        panel_rect.right() - CONTENT_LEFT + 4.0,
        footer_groove_y,
    );
}
