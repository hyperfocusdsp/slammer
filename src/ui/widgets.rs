//! Small shared UI primitives used throughout the editor: rack chrome,
//! screws, grooves, LEDs, inset displays, and the `param_knob` helper that
//! wraps `knob::knob` with a `FloatParam` setter.

use nih_plug::prelude::*;
use nih_plug_egui::egui;

use crate::ui::knob;
use crate::ui::theme;

/// Rack chrome: the left/right steel "ears" with ventilation slots.
pub fn draw_rack_ear(painter: &egui::Painter, x: f32, y: f32, width: f32, height: f32) {
    painter.rect_filled(
        egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(width, height)),
        0.0,
        theme::BG_RACK_EAR,
    );
    for i in 0..8 {
        let slot_y = y + 35.0 + i as f32 * 44.0;
        if slot_y + 22.0 > y + height {
            break;
        }
        painter.rect_filled(
            egui::Rect::from_min_size(
                egui::pos2(x + (width - 8.0) / 2.0, slot_y),
                egui::vec2(8.0, 22.0),
            ),
            2.0,
            theme::BG_VENT,
        );
    }
}

/// A Phillips-head rack screw.
pub fn draw_screw(painter: &egui::Painter, cx: f32, cy: f32, radius: f32) {
    let center = egui::pos2(cx, cy);
    painter.circle_filled(center, radius, theme::SCREW_LIGHT);
    painter.circle_filled(center, radius * 0.85, theme::KNOB_METAL);
    painter.circle_filled(center, radius * 0.7, theme::SCREW_DARK);
    for i in 0..6 {
        let angle = (i as f32 / 6.0) * std::f32::consts::TAU - std::f32::consts::PI / 6.0;
        let p = center + egui::vec2(angle.cos(), angle.sin()) * radius * 0.4;
        painter.circle_filled(p, 1.0, theme::SCREW_HEX);
    }
}

/// Horizontal panel groove — used to separate rows of knobs.
pub fn draw_groove(painter: &egui::Painter, left: f32, right: f32, y: f32) {
    painter.line_segment(
        [egui::pos2(left, y), egui::pos2(right, y)],
        egui::Stroke::new(1.0, theme::GROOVE_DARK),
    );
    painter.line_segment(
        [egui::pos2(left, y + 1.0), egui::pos2(right, y + 1.0)],
        egui::Stroke::new(0.5, theme::GROOVE_LIGHT),
    );
}

/// Small status LED with optional halo glow.
pub fn draw_led(painter: &egui::Painter, cx: f32, cy: f32, on: bool) {
    let center = egui::pos2(cx, cy);
    painter.circle_filled(center, 4.0, egui::Color32::from_rgb(0x08, 0x08, 0x08));
    let color = if on {
        theme::RED_LED
    } else {
        egui::Color32::from_rgb(0x2a, 0x08, 0x08)
    };
    painter.circle_filled(center, 3.0, color);
    if on {
        painter.circle_filled(center, 8.0, theme::RED_GLOW);
    }
}

/// Inset LCD-style display frame with scan-lines and a red ambient glow.
pub fn draw_inset_display(painter: &egui::Painter, x: f32, y: f32, w: f32, h: f32) {
    painter.rect_filled(
        egui::Rect::from_min_size(egui::pos2(x - 4.0, y - 4.0), egui::vec2(w + 8.0, h + 8.0)),
        4.0,
        theme::BG_DISPLAY_FRAME,
    );
    painter.rect_filled(
        egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h)),
        0.0,
        theme::BG_DISPLAY,
    );
    let mut sy = y;
    while sy < y + h {
        painter.line_segment(
            [egui::pos2(x, sy), egui::pos2(x + w, sy)],
            egui::Stroke::new(1.0, egui::Color32::from_rgba_premultiplied(0, 0, 0, 20)),
        );
        sy += 2.0;
    }
    let glow_inset = w * 0.2;
    painter.rect_filled(
        egui::Rect::from_min_size(
            egui::pos2(x + glow_inset, y + h * 0.2),
            egui::vec2(w - glow_inset * 2.0, h * 0.6),
        ),
        0.0,
        theme::RED_AMBIENT,
    );
}

/// Small arrow button used in the preset bar (`◂` / `▸`).
pub fn preset_arrow_btn(
    painter: &egui::Painter,
    rect: egui::Rect,
    glyph: &str,
    color: egui::Color32,
) {
    painter.rect_filled(rect, 2.0, theme::BTN_DARK);
    painter.rect_filled(
        egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), rect.height() * 0.4)),
        2.0,
        theme::BTN_LIGHT,
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        glyph,
        egui::FontId::new(10.0, egui::FontFamily::Monospace),
        color,
    );
}

/// Render a knob wired to a `FloatParam`. Returns `true` if the value changed.
#[allow(clippy::too_many_arguments)]
pub fn param_knob(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    id: &str,
    label: &str,
    param: &FloatParam,
    min: f32,
    max: f32,
    default: f32,
    format_value: impl Fn(f32) -> String,
    diameter: f32,
    core_color: egui::Color32,
) -> bool {
    let mut val = param.value();
    let changed = knob::knob(
        ui,
        egui::Id::new(id),
        &mut val,
        min,
        max,
        default,
        label,
        format_value,
        diameter,
        core_color,
    )
    .changed;
    if changed {
        setter.begin_set_parameter(param);
        setter.set_parameter(param, val);
        setter.end_set_parameter(param);
    }
    changed
}
