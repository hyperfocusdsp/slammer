use crate::ui::theme;
use nih_plug_egui::egui;

pub struct KnobResponse {
    pub changed: bool,
    pub reset: bool,
}

/// G3 industrial knob: rubber grip ring + beveled metal core + tapered indicator.
///
/// Vertical drag changes value, shift for fine control, ctrl+click to reset.
#[allow(clippy::too_many_arguments)]
pub fn knob(
    ui: &mut egui::Ui,
    id: egui::Id,
    value: &mut f32,
    min: f32,
    max: f32,
    default: f32,
    label: &str,
    format_value: impl Fn(f32) -> String,
    diameter: f32,
    core_color: egui::Color32,
) -> KnobResponse {
    let mut result = KnobResponse {
        changed: false,
        reset: false,
    };

    ui.vertical(|ui| {
        ui.set_width(diameter + 12.0);

        let total = diameter + 12.0;
        let size = egui::vec2(total, total);
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
        let response = response.on_hover_cursor(egui::CursorIcon::ResizeVertical);

        // Ctrl+click or double-click to reset.
        // Note: response.double_clicked() is unreliable under baseview (raw
        // mouse events, no synthesised egui double-click). We track the last
        // click time ourselves using per-widget temp storage keyed by `id`.
        let ctrl_click = response.clicked() && ui.input(|i| i.modifiers.ctrl);
        let is_double = if response.clicked() {
            let now: f64 = ui.input(|i| i.time);
            let last: f64 = ui.ctx().data(|d| d.get_temp(id).unwrap_or(f64::NEG_INFINITY));
            ui.ctx().data_mut(|d| d.insert_temp(id, now));
            (now - last) < 0.35
        } else {
            false
        };
        if ctrl_click || is_double {
            *value = default;
            result.changed = true;
            result.reset = true;
        }

        // Vertical drag
        if response.dragged() {
            let delta = -response.drag_delta().y;
            let speed = if ui.input(|i| i.modifiers.shift) {
                0.001
            } else {
                0.005
            };
            *value = (*value + delta * speed * (max - min)).clamp(min, max);
            result.changed = true;
        }

        // Paint
        if ui.is_rect_visible(rect) {
            let painter = ui.painter_at(rect);
            let center = rect.center();
            let radius = diameter / 2.0;
            let norm = ((*value - min) / (max - min)).clamp(0.0, 1.0);

            // 1. Mounting recess shadow
            painter.circle_filled(
                center + egui::vec2(0.5, 1.5),
                radius + 3.0,
                theme::KNOB_RECESS,
            );
            painter.circle_filled(center, radius + 2.0, theme::KNOB_RECESS);

            // 2. Rubber grip ring (outer layer)
            painter.circle_filled(center, radius, theme::KNOB_RUBBER);
            painter.circle_filled(
                center - egui::vec2(0.0, radius * 0.1),
                radius * 0.95,
                theme::KNOB_RUBBER_HIGHLIGHT,
            );
            painter.circle_filled(center, radius * 0.88, theme::KNOB_RUBBER);

            // 3. Bevel ring
            let core_radius = radius * 0.6;
            painter.circle_filled(center, core_radius + 1.5, theme::KNOB_BEVEL);

            // 4. Metal core face — colored per section. Specular highlight
            // stays neutral so the knob reads as anodized metal rather than
            // painted plastic.
            let core_inner = egui::Color32::from_rgb(
                (core_color.r() as f32 * 0.72) as u8,
                (core_color.g() as f32 * 0.72) as u8,
                (core_color.b() as f32 * 0.72) as u8,
            );
            painter.circle_filled(center, core_radius, core_color);
            painter.circle_filled(
                center - egui::vec2(core_radius * 0.15, core_radius * 0.15),
                core_radius * 0.7,
                theme::KNOB_METAL_HIGHLIGHT,
            );
            painter.circle_filled(center, core_radius * 0.5, core_inner);

            // 5. Centre dimple
            painter.circle_filled(center, core_radius * 0.12, theme::KNOB_DIMPLE);

            // 6. Tapered indicator line
            let start_angle = std::f32::consts::PI * 0.75;
            let sweep_range = std::f32::consts::PI * 1.5;
            let angle = start_angle + sweep_range * norm;
            let ind_inner = core_radius * 0.2;
            let ind_outer = core_radius * 0.85;
            let p_inner = center + egui::vec2(angle.cos(), angle.sin()) * ind_inner;
            let p_outer = center + egui::vec2(angle.cos(), angle.sin()) * ind_outer;
            painter.line_segment(
                [p_inner, p_outer],
                egui::Stroke::new(2.0, theme::KNOB_INDICATOR),
            );

            // 7. Tick marks around outer edge
            for i in 0..=10 {
                let tick_angle = start_angle + sweep_range * (i as f32 / 10.0);
                let is_major = i % 5 == 0;
                let inner_r = radius + 2.0;
                let outer_r = radius + if is_major { 5.0 } else { 3.5 };
                let p1 = center + egui::vec2(tick_angle.cos(), tick_angle.sin()) * inner_r;
                let p2 = center + egui::vec2(tick_angle.cos(), tick_angle.sin()) * outer_r;
                let color = if is_major {
                    theme::TICK_MAJOR
                } else {
                    theme::TICK_MINOR
                };
                let width = if is_major { 1.0 } else { 0.5 };
                painter.line_segment([p1, p2], egui::Stroke::new(width, color));
            }

            // 8. Write value to display when hovered/dragged
            if response.hovered() || response.dragged() {
                let display_text = format!("{} {}", label, format_value(*value));
                ui.ctx()
                    .data_mut(|d| d.insert_temp(egui::Id::new("knob_display"), display_text));
            }
        }

        // Label below
        ui.add_space(3.0);
        ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(label)
                    .font(egui::FontId::new(9.5, egui::FontFamily::Monospace))
                    .color(theme::WHITE),
            );
        });
    });

    result
}
