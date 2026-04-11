//! Static panel layout: header chrome, groove/divider drawing, and the
//! SUB / TOP / MID / SAT / EQ / MASTER knob rows.
//!
//! These functions are pure layout helpers — each one takes the `ui`, the
//! parameter setter, the shared `SlammerParams`, and whatever rect metadata
//! it needs, and paints directly. No mutable state is kept between calls.

use nih_plug::prelude::*;
use nih_plug::util;
use nih_plug_egui::egui;

use crate::params::SlammerParams;
use crate::ui::knob;
use crate::ui::theme;
use crate::ui::widgets::{
    draw_groove, draw_inset_display, draw_led, draw_rack_ear, draw_screw, param_knob,
};

pub const BASE_W: f32 = 680.0;
#[allow(dead_code)]
pub const BASE_H: f32 = 444.0;
pub const KNOB_SIZE: f32 = 32.0;
pub const KNOB_SPACING: f32 = 52.0;
pub const RACK_EAR_W: f32 = 16.0;
pub const CONTENT_LEFT: f32 = RACK_EAR_W + 14.0;
pub const HEADER_H: f32 = 28.0;

/// Draw the panel background, rack ears, screws, bevels, and title strip.
/// Returns the vertical center of the header band, which downstream code
/// uses to align the preset bar and test button.
pub fn draw_chrome(ui: &egui::Ui, panel_rect: egui::Rect) -> f32 {
    let w = panel_rect.width();
    let h = panel_rect.height();
    let header_center_y = panel_rect.top() + HEADER_H * 0.5;

    let painter = ui.painter();
    painter.rect_filled(panel_rect, 0.0, theme::BG_PANEL);
    // Top edge band
    painter.rect_filled(
        egui::Rect::from_min_size(panel_rect.min, egui::vec2(w, 12.0)),
        0.0,
        theme::BG_PANEL_EDGE,
    );
    // Bottom edge band
    painter.rect_filled(
        egui::Rect::from_min_size(
            egui::pos2(panel_rect.left(), panel_rect.bottom() - 12.0),
            egui::vec2(w, 12.0),
        ),
        0.0,
        theme::BG_PANEL_EDGE,
    );

    // Rack ears
    draw_rack_ear(painter, panel_rect.left(), panel_rect.top(), RACK_EAR_W, h);
    draw_rack_ear(
        painter,
        panel_rect.right() - RACK_EAR_W,
        panel_rect.top(),
        RACK_EAR_W,
        h,
    );

    // Screws (corners)
    let screw_r = 5.0;
    draw_screw(
        painter,
        panel_rect.left() + 8.0,
        panel_rect.top() + 18.0,
        screw_r,
    );
    draw_screw(
        painter,
        panel_rect.left() + 8.0,
        panel_rect.bottom() - 18.0,
        screw_r,
    );
    draw_screw(
        painter,
        panel_rect.right() - 8.0,
        panel_rect.top() + 18.0,
        screw_r,
    );
    draw_screw(
        painter,
        panel_rect.right() - 8.0,
        panel_rect.bottom() - 18.0,
        screw_r,
    );

    // Title text + power LED
    painter.text(
        egui::pos2(panel_rect.left() + CONTENT_LEFT, header_center_y),
        egui::Align2::LEFT_CENTER,
        "SLAMMER",
        egui::FontId::new(16.0, egui::FontFamily::Monospace),
        theme::WHITE,
    );
    draw_led(
        painter,
        panel_rect.left() + CONTENT_LEFT + 120.0,
        header_center_y,
        true,
    );

    painter.text(
        egui::pos2(panel_rect.right() - CONTENT_LEFT, header_center_y - 2.0),
        egui::Align2::RIGHT_CENTER,
        "KICK SYNTHESIZER",
        egui::FontId::new(8.0, egui::FontFamily::Monospace),
        theme::TEXT_DIM,
    );
    painter.text(
        egui::pos2(panel_rect.right() - CONTENT_LEFT, header_center_y + 8.0),
        egui::Align2::RIGHT_CENTER,
        format!("v{}", env!("CARGO_PKG_VERSION")),
        egui::FontId::new(8.0, egui::FontFamily::Monospace),
        egui::Color32::from_rgb(0x44, 0x44, 0x44),
    );

    header_center_y
}

/// Draw the "TEST" button in the header. Returns true if it was clicked
/// this frame (so the caller can dispatch a trigger).
pub fn test_button(ui: &mut egui::Ui, panel_rect: egui::Rect, header_center_y: f32) -> bool {
    let btn_x = panel_rect.left() + CONTENT_LEFT + 140.0;
    let btn_w = 48.0;
    let btn_h = 22.0;
    let btn_y = header_center_y - btn_h * 0.5;
    let btn_rect = egui::Rect::from_min_size(egui::pos2(btn_x, btn_y), egui::vec2(btn_w, btn_h));
    let resp = ui.interact(
        btn_rect,
        egui::Id::new("test_trigger"),
        egui::Sense::click(),
    );
    let pressed = resp.is_pointer_button_down_on();
    {
        let painter = ui.painter();
        let top_color = if pressed {
            theme::BTN_DARK
        } else {
            theme::BTN_LIGHT
        };
        let bot_color = if pressed {
            theme::BTN_LIGHT
        } else {
            theme::BTN_DARK
        };
        painter.rect_filled(btn_rect, 3.0, bot_color);
        painter.rect_filled(
            egui::Rect::from_min_size(btn_rect.min, egui::vec2(btn_w, btn_h * 0.5)),
            3.0,
            top_color,
        );
        painter.text(
            btn_rect.center(),
            egui::Align2::CENTER_CENTER,
            "TEST",
            egui::FontId::new(12.0, egui::FontFamily::Monospace),
            theme::WHITE,
        );
    }
    let clicked = resp.clicked();
    if resp.hovered() {
        resp.on_hover_text_at_pointer("hit T to trigger");
    }
    clicked
}

/// Draw the master row (OUTPUT waveform display + master knobs).
/// `waveform_peaks` is a slice of the rolling waveform to render inside the
/// display; `knob_readout_text` is the optional 7-seg text to show in the
/// display's bottom-right corner.
pub struct MasterRow<'a> {
    pub master_y: f32,
    pub wf_left: f32,
    pub wf_width: f32,
    pub wf_height: f32,
    pub waveform_peaks: &'a [f32],
    /// Smoothed gain reduction (positive dB) from the master-bus compressor,
    /// for the GR overlay bar drawn on top of the OUTPUT display.
    pub gr_db: f32,
}

impl<'a> MasterRow<'a> {
    pub fn draw(
        &self,
        ui: &mut egui::Ui,
        setter: &ParamSetter,
        params: &SlammerParams,
        panel_rect: egui::Rect,
    ) {
        let painter = ui.painter();
        draw_inset_display(
            painter,
            self.wf_left,
            self.master_y,
            self.wf_width,
            self.wf_height,
        );
        painter.text(
            egui::pos2(self.wf_left + 4.0, self.master_y + 3.0),
            egui::Align2::LEFT_TOP,
            "OUTPUT",
            egui::FontId::new(6.0, egui::FontFamily::Monospace),
            theme::RED_GHOST,
        );
        if !self.waveform_peaks.is_empty() {
            let n = self.waveform_peaks.len();
            let mid_y = self.master_y + self.wf_height / 2.0;
            for (i, &peak) in self.waveform_peaks.iter().enumerate() {
                let x = self.wf_left + 2.0 + (i as f32 / n as f32) * (self.wf_width - 4.0);
                let amp = peak.min(1.0) * self.wf_height * 0.42;
                painter.line_segment(
                    [egui::pos2(x, mid_y - amp), egui::pos2(x, mid_y + amp)],
                    egui::Stroke::new(1.2, theme::RED_WAVEFORM),
                );
            }
        }

        // ── Gain-reduction overlay bar ──
        // Painted along the top of the OUTPUT display, just under the
        // "OUTPUT" label. Fills right-to-left, 0..18 dB of reduction maps
        // to 0..full width.
        {
            let gr_max_db = 18.0f32;
            let gr_norm = (self.gr_db / gr_max_db).clamp(0.0, 1.0);
            let bar_x = self.wf_left + 36.0; // clear the "OUTPUT" label
            let bar_y = self.master_y + 4.0;
            let bar_w_total = self.wf_width - 40.0 - 4.0;
            let bar_h = 3.0;
            // Housing (dim red, full width).
            painter.rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(bar_x, bar_y),
                    egui::vec2(bar_w_total, bar_h),
                ),
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
            // Tick labels: 0 / 6 / 12 dB, right-aligned.
            let label_y = bar_y + bar_h + 1.0;
            let label_font = egui::FontId::new(6.0, egui::FontFamily::Monospace);
            for (db, label) in &[(0.0f32, "0"), (6.0, "6"), (12.0, "12")] {
                let frac = db / gr_max_db;
                let lx = bar_x + bar_w_total - frac * bar_w_total;
                painter.text(
                    egui::pos2(lx, label_y),
                    egui::Align2::CENTER_TOP,
                    label,
                    label_font.clone(),
                    theme::TEXT_DIM,
                );
            }
            painter.text(
                egui::pos2(bar_x - 2.0, bar_y + bar_h * 0.5),
                egui::Align2::RIGHT_CENTER,
                "GR",
                egui::FontId::new(6.0, egui::FontFamily::Monospace),
                theme::TEXT_DIM,
            );
        }
        // Knob value readout — rendered via a temp data slot set by knob.rs
        let knob_text: Option<String> =
            ui.ctx().data(|d| d.get_temp(egui::Id::new("knob_display")));
        if let Some(text) = knob_text {
            let readout_rect = egui::Rect::from_min_size(
                egui::pos2(
                    self.wf_left + self.wf_width - 160.0,
                    self.master_y + self.wf_height - 18.0,
                ),
                egui::vec2(156.0, 16.0),
            );
            crate::ui::seven_seg::draw_7seg_text(ui.painter(), readout_rect, &text);
        }

        // Master knobs strip to the right of the display
        let knob_row_y = self.master_y + 4.0;
        let knobs_x = self.wf_left + self.wf_width + 16.0;
        let master_knob_rect = egui::Rect::from_min_size(
            egui::pos2(knobs_x, knob_row_y),
            egui::vec2(KNOB_SPACING * 4.0, KNOB_SIZE + 30.0),
        );
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
                    theme::KNOB_METAL,
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
                    theme::KNOB_METAL,
                );
                param_knob(
                    ui,
                    setter,
                    "vel",
                    "VEL",
                    &params.velocity_sens,
                    0.0,
                    1.0,
                    0.8,
                    |v| format!("{:.0}%", v * 100.0),
                    KNOB_SIZE,
                    theme::KNOB_METAL,
                );
                // Master volume is stored as gain and displayed in dB.
                let mut vol_db = util::gain_to_db(params.master_volume.value());
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
                    theme::KNOB_METAL,
                );
                if resp.changed {
                    setter.begin_set_parameter(&params.master_volume);
                    setter.set_parameter(&params.master_volume, util::db_to_gain(vol_db));
                    setter.end_set_parameter(&params.master_volume);
                }
            });
        });

        // ── Compressor strip (right of master knobs) ──
        // Occupies the free ~100 px slot at the far right of the master row.
        // Three macro knobs (AMT / REACT / DRIVE) + a clickable LIM LED.
        {
            let strip_x = knobs_x + KNOB_SPACING * 4.0 + 4.0;
            let strip_right = panel_rect.right() - CONTENT_LEFT;
            let strip_w = (strip_right - strip_x).max(0.0);
            if strip_w >= 80.0 {
                // Inset display-style background for visual consistency with
                // the OUTPUT screen to the left.
                draw_inset_display(
                    ui.painter(),
                    strip_x + 2.0,
                    self.master_y,
                    strip_w - 4.0,
                    self.wf_height,
                );
                ui.painter().text(
                    egui::pos2(strip_x + 6.0, self.master_y + 3.0),
                    egui::Align2::LEFT_TOP,
                    "COMP",
                    egui::FontId::new(6.0, egui::FontFamily::Monospace),
                    theme::RED_GHOST,
                );

                // LIM toggle — small LED + label, painted in the top-right
                // corner of the strip. Clickable via ui.interact().
                let lim_cx = strip_x + strip_w - 12.0;
                let lim_cy = self.master_y + 8.0;
                let lim_on = params.comp_limit_on.value();
                draw_led(ui.painter(), lim_cx, lim_cy, lim_on);
                ui.painter().text(
                    egui::pos2(lim_cx - 7.0, lim_cy),
                    egui::Align2::RIGHT_CENTER,
                    "LIM",
                    egui::FontId::new(7.0, egui::FontFamily::Monospace),
                    if lim_on { theme::WHITE } else { theme::TEXT_DIM },
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
                let small_knob = 20.0f32;
                let knob_cell_w = small_knob + 12.0;
                let row_w = knob_cell_w * 3.0 + 4.0;
                let row_x = strip_x + ((strip_w - row_w) * 0.5).max(4.0);
                let row_y = self.master_y + 14.0;
                let comp_rect = egui::Rect::from_min_size(
                    egui::pos2(row_x, row_y),
                    egui::vec2(row_w, small_knob + 24.0),
                );
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(comp_rect), |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    ui.horizontal(|ui| {
                        param_knob(
                            ui,
                            setter,
                            "comp_amt",
                            "AMT",
                            &params.comp_amount,
                            0.0,
                            1.0,
                            0.0,
                            |v| format!("{:.0}%", v * 100.0),
                            small_knob,
                            theme::KNOB_METAL,
                        );
                        param_knob(
                            ui,
                            setter,
                            "comp_rct",
                            "RCT",
                            &params.comp_react,
                            0.0,
                            1.0,
                            0.35,
                            |v| format!("{:.0}%", v * 100.0),
                            small_knob,
                            theme::KNOB_METAL,
                        );
                        param_knob(
                            ui,
                            setter,
                            "comp_drv",
                            "DRV",
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
    }
}

/// Draw the SUB | TOP row (labels + groove + divider + knobs).
/// Returns the y coordinate where the knob row starts (so callers can stack
/// the next row).
pub fn draw_sub_top_row(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    params: &SlammerParams,
    panel_rect: egui::Rect,
    master_bottom_y: f32,
) -> f32 {
    let row_label_y = master_bottom_y + 8.0;
    let row_groove_y = row_label_y + 14.0;
    let row_knob_y = row_groove_y + 4.0;
    let divider_x = panel_rect.left() + CONTENT_LEFT + KNOB_SPACING * 6.0 - 6.0;

    {
        let painter = ui.painter();
        painter.text(
            egui::pos2(panel_rect.left() + CONTENT_LEFT, row_label_y),
            egui::Align2::LEFT_TOP,
            "SUB",
            egui::FontId::new(11.0, egui::FontFamily::Monospace),
            theme::WHITE,
        );
        painter.text(
            egui::pos2(
                panel_rect.left() + CONTENT_LEFT + KNOB_SPACING * 6.0,
                row_label_y,
            ),
            egui::Align2::LEFT_TOP,
            "TOP",
            egui::FontId::new(11.0, egui::FontFamily::Monospace),
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
                egui::pos2(divider_x, row_groove_y + 2.0),
                egui::pos2(divider_x, row_knob_y + KNOB_SIZE + 30.0),
            ],
            egui::Stroke::new(1.0, theme::DIVIDER),
        );
    }

    // SUB knobs
    let sub_knob_rect = egui::Rect::from_min_size(
        egui::pos2(panel_rect.left() + CONTENT_LEFT, row_knob_y),
        egui::vec2(KNOB_SPACING * 6.0, KNOB_SIZE + 30.0),
    );
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
            param_knob(
                ui,
                setter,
                "s_ph",
                "PHASE",
                &params.sub_phase_offset,
                0.0,
                360.0,
                90.0,
                |v| format!("{v:.0}\u{00b0}"),
                KNOB_SIZE,
                theme::SECTION_SUB,
            );
        });
    });

    // TOP knobs
    let top_knob_rect = egui::Rect::from_min_size(
        egui::pos2(
            panel_rect.left() + CONTENT_LEFT + KNOB_SPACING * 6.0,
            row_knob_y,
        ),
        egui::vec2(KNOB_SPACING * 4.0, KNOB_SIZE + 30.0),
    );
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
                "BW",
                &params.top_bw,
                0.2,
                3.0,
                1.5,
                |v| format!("{v:.1}oct"),
                KNOB_SIZE,
                theme::SECTION_TOP,
            );
        });
    });

    row_knob_y + KNOB_SIZE + 34.0
}

/// Draw the MID row. Returns the bottom y of the row.
pub fn draw_mid_row(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    params: &SlammerParams,
    panel_rect: egui::Rect,
    sub_top_bottom_y: f32,
) -> f32 {
    let row_label_y = sub_top_bottom_y;
    let row_groove_y = row_label_y + 14.0;
    let row_knob_y = row_groove_y + 4.0;

    {
        let painter = ui.painter();
        painter.text(
            egui::pos2(panel_rect.left() + CONTENT_LEFT, row_label_y),
            egui::Align2::LEFT_TOP,
            "MID",
            egui::FontId::new(11.0, egui::FontFamily::Monospace),
            theme::WHITE,
        );
        draw_groove(
            painter,
            panel_rect.left() + CONTENT_LEFT - 4.0,
            panel_rect.right() - CONTENT_LEFT + 4.0,
            row_groove_y,
        );
    }

    let mid_knob_rect = egui::Rect::from_min_size(
        egui::pos2(panel_rect.left() + CONTENT_LEFT, row_knob_y),
        egui::vec2(KNOB_SPACING * 9.0, KNOB_SIZE + 30.0),
    );
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
        });
    });

    row_knob_y + KNOB_SIZE + 34.0
}

/// Draw the SAT | EQ row. Returns the bottom y of the row.
/// Result of drawing the SAT/EQ row. `next_y` is where the following row
/// should start; `bounce_clicked` reports whether the user clicked BOUNCE
/// this frame so the caller can kick off a one-shot export.
pub struct SatEqRowResult {
    pub next_y: f32,
    pub bounce_clicked: bool,
}

pub fn draw_sat_eq_row(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    params: &SlammerParams,
    panel_rect: egui::Rect,
    mid_bottom_y: f32,
) -> SatEqRowResult {
    let row_label_y = mid_bottom_y;
    let row_groove_y = row_label_y + 14.0;
    let row_knob_y = row_groove_y + 4.0;
    let eq_divider_x = panel_rect.left() + CONTENT_LEFT + KNOB_SPACING * 4.0 + 40.0;

    {
        let painter = ui.painter();
        painter.text(
            egui::pos2(panel_rect.left() + CONTENT_LEFT, row_label_y),
            egui::Align2::LEFT_TOP,
            "SAT",
            egui::FontId::new(11.0, egui::FontFamily::Monospace),
            theme::WHITE,
        );
        painter.text(
            egui::pos2(eq_divider_x + 10.0, row_label_y),
            egui::Align2::LEFT_TOP,
            "EQ",
            egui::FontId::new(11.0, egui::FontFamily::Monospace),
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
                egui::pos2(eq_divider_x, row_knob_y + KNOB_SIZE + 30.0),
            ],
            egui::Stroke::new(1.0, theme::DIVIDER),
        );
    }

    // SAT: LCD selector + 2 knobs
    let sat_rect = egui::Rect::from_min_size(
        egui::pos2(panel_rect.left() + CONTENT_LEFT, row_knob_y),
        egui::vec2(KNOB_SPACING * 4.0 + 36.0, KNOB_SIZE + 30.0),
    );
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(sat_rect), |ui| {
        ui.horizontal(|ui| {
            crate::ui::seven_seg::lcd_selector(ui, setter, &params.sat_mode);
            ui.add_space(12.0);
            param_knob(
                ui,
                setter,
                "sat_d",
                "DRIVE",
                &params.sat_drive,
                0.0,
                1.0,
                0.0,
                |v| format!("{:.0}%", v * 100.0),
                KNOB_SIZE,
                theme::SECTION_SAT,
            );
            param_knob(
                ui,
                setter,
                "sat_x",
                "MIX",
                &params.sat_mix,
                0.0,
                1.0,
                1.0,
                |v| format!("{:.0}%", v * 100.0),
                KNOB_SIZE,
                theme::SECTION_SAT,
            );
        });
    });

    // EQ: 5 knobs
    let eq_rect = egui::Rect::from_min_size(
        egui::pos2(eq_divider_x + 10.0, row_knob_y),
        egui::vec2(KNOB_SPACING * 5.0, KNOB_SIZE + 30.0),
    );
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
    let bounce_clicked = draw_bounce_button(ui, panel_rect, row_knob_y);

    SatEqRowResult {
        next_y: row_knob_y + KNOB_SIZE + 30.0,
        bounce_clicked,
    }
}

/// Beveled "BOUNCE" button in the right gap of the SAT/EQ row. Styled to
/// match [`test_button`] so it reads as part of the same visual family.
/// Returns `true` on the frame the user clicks it.
fn draw_bounce_button(ui: &mut egui::Ui, panel_rect: egui::Rect, row_knob_y: f32) -> bool {
    let btn_w = 56.0;
    let btn_h = 22.0;
    // Right-align with a small inset so the button doesn't kiss the rack ear.
    let btn_x = panel_rect.right() - CONTENT_LEFT - btn_w;
    // Vertically centered on the knob row (same as the knob caps).
    let btn_y = row_knob_y + (KNOB_SIZE - btn_h) * 0.5 + 4.0;
    let btn_rect = egui::Rect::from_min_size(egui::pos2(btn_x, btn_y), egui::vec2(btn_w, btn_h));

    let resp = ui.interact(
        btn_rect,
        egui::Id::new("export_bounce"),
        egui::Sense::click(),
    );
    let pressed = resp.is_pointer_button_down_on();
    {
        let painter = ui.painter();
        let top_color = if pressed {
            theme::BTN_DARK
        } else {
            theme::BTN_LIGHT
        };
        let bot_color = if pressed {
            theme::BTN_LIGHT
        } else {
            theme::BTN_DARK
        };
        painter.rect_filled(btn_rect, 3.0, bot_color);
        painter.rect_filled(
            egui::Rect::from_min_size(btn_rect.min, egui::vec2(btn_w, btn_h * 0.5)),
            3.0,
            top_color,
        );
        painter.text(
            btn_rect.center(),
            egui::Align2::CENTER_CENTER,
            "BOUNCE",
            egui::FontId::new(11.0, egui::FontFamily::Monospace),
            theme::WHITE,
        );
        // Small label underneath so users know what it does at a glance.
        painter.text(
            egui::pos2(btn_rect.center().x, btn_rect.bottom() + 2.0),
            egui::Align2::CENTER_TOP,
            "EXPORT",
            egui::FontId::new(6.0, egui::FontFamily::Monospace),
            theme::TEXT_DIM,
        );
    }
    let clicked = resp.clicked();
    if resp.hovered() {
        resp.on_hover_text_at_pointer("Export one hit to WAV/AIFF");
    }
    clicked
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
#[derive(Default)]
pub struct SequencerUiState {
    pub paint_mode: Option<bool>,
    pub last_painted: Option<usize>,
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
fn draw_tempo_widget(
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
            theme::TEXT_DIM,
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
    let color = if state.armed {
        theme::WHITE
    } else {
        theme::TEXT_DIM
    };
    let text = format!("{:.0} BPM", seq.display_bpm());
    ui.painter()
        .text(pos, egui::Align2::LEFT_TOP, &text, font.clone(), color);

    // Underline when armed.
    if state.armed {
        let underline_y = pos.y + 11.0;
        ui.painter().line_segment(
            [
                egui::pos2(pos.x, underline_y),
                egui::pos2(pos.x + 42.0, underline_y),
            ],
            egui::Stroke::new(1.0, theme::WHITE),
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
            let delta = (right_10 as i32) * 10 + (right_1 as i32)
                - (left_10 as i32) * 10
                - (left_1 as i32);
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
    setter: &ParamSetter,
    params: &SlammerParams,
    panel_rect: egui::Rect,
    sat_eq_bottom_y: f32,
    seq: &crate::sequencer::Sequencer,
    ui_state: &mut SequencerUiState,
) {
    let row_label_y = sat_eq_bottom_y + 4.0;
    let row_groove_y = row_label_y + 14.0;
    let pad_top = row_groove_y + 6.0;
    let pad_h = 22.0;
    let pad_w = 26.0;
    let pad_gap = 3.0;

    let host_synced = seq.is_host_synced();

    // "STEP" label + groove
    {
        let painter = ui.painter();
        painter.text(
            egui::pos2(panel_rect.left() + CONTENT_LEFT, row_label_y),
            egui::Align2::LEFT_TOP,
            "STEP",
            egui::FontId::new(11.0, egui::FontFamily::Monospace),
            theme::WHITE,
        );
        draw_groove(
            painter,
            panel_rect.left() + CONTENT_LEFT - 4.0,
            panel_rect.right() - CONTENT_LEFT + 4.0,
            row_groove_y,
        );
    }

    // BPM readout — interactive in standalone, read-only in host-synced mode.
    draw_tempo_widget(
        ui,
        egui::pos2(panel_rect.left() + CONTENT_LEFT + 60.0, row_label_y),
        seq,
        host_synced,
        &mut ui_state.tempo_edit,
    );

    // Play / stop button — click to toggle in standalone; shows (and is
    // disabled to) the effective host state in DAW mode.
    let play_w = 40.0;
    let play_rect = egui::Rect::from_min_size(
        egui::pos2(panel_rect.left() + CONTENT_LEFT, pad_top),
        egui::vec2(play_w, pad_h),
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
    {
        let painter = ui.painter();
        let running = seq.is_running_effective();
        let top = if running {
            theme::BTN_LIGHT
        } else {
            theme::BTN_DARK
        };
        let bot = if running {
            theme::BTN_DARK
        } else {
            theme::BTN_LIGHT
        };
        painter.rect_filled(play_rect, 3.0, bot);
        painter.rect_filled(
            egui::Rect::from_min_size(play_rect.min, egui::vec2(play_w, pad_h * 0.5)),
            3.0,
            top,
        );
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
        painter.text(
            play_rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::new(10.0, egui::FontFamily::Monospace),
            label_color,
        );
    }

    // 16 step pads — centered after the play button, with a small gap.
    let pads_total_w = pad_w * 16.0 + pad_gap * 15.0;
    let pads_start_x = play_rect.right() + 12.0;
    // If there's still room on the right, leave it for future features.
    let _ = pads_total_w;

    // Snapshot pointer/button state once per frame for the drag logic.
    let (primary_down, primary_released, pointer_pos) = ui.input(|i| {
        (
            i.pointer.primary_down(),
            i.pointer.primary_released(),
            i.pointer.interact_pos(),
        )
    });
    // Release always ends an active paint drag, regardless of where the
    // release happened.
    if primary_released || !primary_down {
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

        // Initial press on this pad: determine the paint mode from the
        // current state (off → draw on, on → erase) and apply it.
        if resp.drag_started() || resp.clicked() {
            let mode = !seq.is_step_on(i);
            ui_state.paint_mode = Some(mode);
            ui_state.last_painted = Some(i);
            seq.set_step(i, mode);
        }

        // Right-click cycles per-step flam: Off → Flam → Ruff → Roll → Off.
        if resp.secondary_clicked() {
            seq.cycle_flam_state(i);
        }
        let resp = resp.on_hover_text("Right-click: Flam / Ruff / Roll");

        let on = seq.is_step_on(i);
        let is_playhead = seq.is_running_effective() && i == current;
        let beat_marker = i % 4 == 0;

        let painter = ui.painter();
        // Bevel / body
        let body_color = if on {
            theme::RED_WAVEFORM
        } else if beat_marker {
            egui::Color32::from_rgb(0x26, 0x22, 0x22)
        } else {
            egui::Color32::from_rgb(0x1a, 0x1a, 0x1a)
        };
        painter.rect_filled(rect, 2.0, body_color);
        // Highlight stripe on top-half for a subtle bevel
        painter.rect_filled(
            egui::Rect::from_min_size(rect.min, egui::vec2(pad_w, pad_h * 0.45)),
            2.0,
            if on {
                theme::RED_GHOST
            } else {
                egui::Color32::from_rgb(0x2a, 0x2a, 0x2a)
            },
        );

        // Flam indicator dots above the pad — one dot per stroke beyond
        // the base hit (Flam=1 dot, Ruff=2, Roll=3).
        let flam = seq.flam_state(i);
        if on && flam > 0 {
            let n_dots = flam as usize;
            let dot_r = 1.5;
            let dot_gap = 2.0;
            let total_w = n_dots as f32 * (dot_r * 2.0) + (n_dots - 1) as f32 * dot_gap;
            let start_x = rect.center().x - total_w * 0.5 + dot_r;
            let y = rect.top() - 3.0;
            for d in 0..n_dots {
                let cx = start_x + d as f32 * (dot_r * 2.0 + dot_gap);
                painter.circle_filled(egui::pos2(cx, y), dot_r, theme::RED_GHOST);
            }
        }

        // Playhead ring
        if is_playhead {
            painter.rect_stroke(
                rect.expand(1.0),
                2.5,
                egui::Stroke::new(1.5, theme::WHITE),
                egui::StrokeKind::Outside,
            );
        }

        // Beat number (1, 5, 9, 13) in dim text for orientation
        if beat_marker {
            painter.text(
                egui::pos2(rect.left() + 3.0, rect.top() + 2.0),
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

    // SPRD + HUM flam knobs tucked into the right-hand gap.
    {
        let pads_right = pads_start_x + pitch * 16.0;
        let small_knob = 18.0f32;
        let knob_cell_w = small_knob + 10.0;
        let row_w = knob_cell_w * 2.0 + 2.0;
        let row_x = pads_right + 10.0;
        let row_y = pad_top - 6.0;
        let knob_rect = egui::Rect::from_min_size(
            egui::pos2(row_x, row_y),
            egui::vec2(row_w, small_knob + 20.0),
        );
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(knob_rect), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.horizontal(|ui| {
                param_knob(
                    ui,
                    setter,
                    "flam_sprd",
                    "SPRD",
                    &params.flam_spread_ms,
                    2.0,
                    30.0,
                    15.0,
                    |v| format!("{:.0}ms", v),
                    small_knob,
                    theme::KNOB_METAL,
                );
                param_knob(
                    ui,
                    setter,
                    "flam_hum",
                    "HUM",
                    &params.flam_humanize,
                    0.0,
                    1.0,
                    0.3,
                    |v| format!("{:.0}%", v * 100.0),
                    small_knob,
                    theme::KNOB_METAL,
                );
            });
        });
    }

    // Fast-drag fill: if a paint drag is in progress and the pointer has
    // jumped to a new step since the last frame, paint every step in the
    // inclusive range between the last painted index and the one the
    // pointer is currently over. Without this pass, a quick mouse swipe
    // across the row skips any pads the pointer wasn't literally over on
    // a rendered frame.
    if let (Some(mode), true, Some(hover_idx)) =
        (ui_state.paint_mode, primary_down, hovered_step)
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

/// Draw the bottom footer groove and the "REXIST INSTRUMENTS" brand text.
pub fn draw_footer(ui: &egui::Ui, panel_rect: egui::Rect) {
    let painter = ui.painter();
    let footer_groove_y = panel_rect.bottom() - 22.0;
    draw_groove(
        painter,
        panel_rect.left() + CONTENT_LEFT - 4.0,
        panel_rect.right() - CONTENT_LEFT + 4.0,
        footer_groove_y,
    );
    painter.text(
        egui::pos2(panel_rect.right() - CONTENT_LEFT, panel_rect.bottom() - 6.0),
        egui::Align2::RIGHT_BOTTOM,
        "REXIST INSTRUMENTS",
        egui::FontId::new(7.0, egui::FontFamily::Monospace),
        theme::TEXT_GHOST,
    );
}
