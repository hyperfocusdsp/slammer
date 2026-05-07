use crate::ui::theme;
use nih_plug_egui::egui;

pub struct KnobResponse {
    pub changed: bool,
    pub reset: bool,
    /// Inner click-and-drag response of the knob rect itself (not the
    /// surrounding column). `None` only on the very first frame before
    /// allocation; populated for every subsequent frame. Callers attach
    /// `.context_menu()` to this when they want a right-click menu (e.g.
    /// MIDI Learn) anchored on the knob, not the label below.
    pub response: Option<egui::Response>,
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
    knob_inner(
        ui,
        id,
        value,
        min,
        max,
        default,
        label,
        "",
        format_value,
        diameter,
        core_color,
        false,
    )
}

/// Compact variant: tighter knob-to-label spacing for dense clusters
/// (e.g. the v0.6.0 SAT/CLIP stacked sub-rows). Visual rendering of the
/// knob itself is identical; only the surrounding box padding and the
/// gap before the label shrink — saves ~9 px of vertical room per knob.
///
/// `tooltip` adds a hover-text bubble explaining the abbreviated label
/// (e.g. label="CDRV" + tooltip="Voice clip drive — per-voice
/// waveshaper amount before amp envelope"). Pass an empty string to
/// suppress.
#[allow(clippy::too_many_arguments)]
pub fn knob_compact(
    ui: &mut egui::Ui,
    id: egui::Id,
    value: &mut f32,
    min: f32,
    max: f32,
    default: f32,
    label: &str,
    tooltip: &str,
    format_value: impl Fn(f32) -> String,
    diameter: f32,
    core_color: egui::Color32,
) -> KnobResponse {
    knob_inner(
        ui,
        id,
        value,
        min,
        max,
        default,
        label,
        tooltip,
        format_value,
        diameter,
        core_color,
        true,
    )
}

#[allow(clippy::too_many_arguments)]
fn knob_inner(
    ui: &mut egui::Ui,
    id: egui::Id,
    value: &mut f32,
    min: f32,
    max: f32,
    default: f32,
    label: &str,
    tooltip: &str,
    format_value: impl Fn(f32) -> String,
    diameter: f32,
    core_color: egui::Color32,
    compact: bool,
) -> KnobResponse {
    let mut result = KnobResponse {
        changed: false,
        reset: false,
        response: None,
    };

    // Compact knobs need 8 px of padding (was 4) so the new tick-dot zone
    // at `radius + 2.5` stays inside `painter_at(rect)`'s clip rect for
    // the 18 px small knobs (rect = diameter+8 = 26 → centre-to-edge 13,
    // dots at radius+2.5 = 11.5 → safe). column_w stays >= 30 so labels
    // still don't wrap.
    let box_pad = if compact { 8.0 } else { 12.0 };
    let label_gap = if compact { 0.0 } else { 3.0 };
    let total = diameter + box_pad;
    // Compact mode keeps the knob box visually tight (≈ diameter + 4) but
    // widens the surrounding column to ≥ 30 px so 3- and 4-character
    // labels render on a single line without wrapping. The knob is then
    // centred horizontally inside the wider column.
    let column_w = if compact {
        (diameter + 12.0).max(total)
    } else {
        total
    };

    ui.vertical(|ui| {
        ui.set_width(column_w);
        if compact {
            // Without this, the parent's `item_spacing.y` (typically 4 px in
            // a stacked sub-row cluster) leaks into this inner vertical and
            // adds an unwanted gap between the knob box and the label.
            ui.spacing_mut().item_spacing.y = 0.0;
        }

        let size = egui::vec2(total, total);
        let knob_alloc = ui
            .allocate_ui_with_layout(
                egui::vec2(column_w, total),
                egui::Layout::top_down(egui::Align::Center),
                |ui| ui.allocate_exact_size(size, egui::Sense::click_and_drag()),
            )
            .inner;
        let (rect, response) = knob_alloc;
        let mut response = response.on_hover_cursor(egui::CursorIcon::ResizeVertical);
        if !tooltip.is_empty() {
            response = response.on_hover_text(tooltip);
        }
        // Hand the inner knob rect's response back to the caller so they
        // can attach `.context_menu()` (used by MIDI Learn). Cloned out of
        // the `ui.vertical` closure scope so the outer caller can use it
        // after the closure returns.
        result.response = Some(response.clone());

        // Ctrl+click or double-click to reset.
        // Note: response.double_clicked() is unreliable under baseview (raw
        // mouse events, no synthesised egui double-click). We track the last
        // click time ourselves using per-widget temp storage keyed by `id`.
        let ctrl_click = response.clicked() && ui.input(|i| i.modifiers.ctrl);
        let is_double = if response.clicked() {
            let now: f64 = ui.input(|i| i.time);
            let last: f64 = ui
                .ctx()
                .data(|d| d.get_temp(id).unwrap_or(f64::NEG_INFINITY));
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

            // 4. Flat plastic core. When the Cycles-baked `knob_cap.png`
            // texture is loaded, blit it tinted by `core_color` —
            // egui's painter.image multiplies the texture by the tint so
            // a single neutral-white-plastic bake renders as any section
            // colour with photoreal studio lighting. Falls through to a
            // solid base when the bake didn't load.
            //
            // The cap is blitted at `core_radius - 1.0` (slightly inside
            // the bevel ring) so the grey ring covers the bake's
            // anti-aliased boundary. Two wins from the inset:
            //   (a) the core reads as *recessed under* the bevel (the
            //       grey ring sits proud of the colored disk), and
            //   (b) the AA edge of the bake is hidden by the solid bevel,
            //       killing the pixelation/aliasing that showed when the
            //       bake's alpha falloff competed with the bevel circle.
            let visible_core_r = (core_radius - 1.0).max(core_radius * 0.85);
            let cap_handle =
                if crate::ui::widgets::KNOB_CAP_BAKED.load(std::sync::atomic::Ordering::Relaxed) {
                    crate::ui::widgets::knob_cap_handle(ui.ctx())
                } else {
                    None
                };
            if let Some(handle) = cap_handle {
                // The bake's visible disk is `cap.radius_px=110` in a
                // 256-wide canvas → fills 110/128 = 0.859 of the half.
                // Scale dest rect so the disk maps to visible_core_r.
                let cap_scale = 128.0 / 110.0;
                let cap_w = visible_core_r * 2.0 * cap_scale;
                let cap_rect = egui::Rect::from_center_size(center, egui::vec2(cap_w, cap_w));
                painter.image(
                    handle.id(),
                    cap_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    core_color,
                );
            } else {
                painter.circle_filled(center, visible_core_r, core_color);
            }
            // Subtle inner shadow at the core's outer edge — sells the
            // "recessed under the rim" depth without darkening the cap
            // surface itself. Stroke width 1.0 centered at visible
            // core_r darkens just the outer 0.5 px of the cap.
            painter.circle_stroke(
                center,
                visible_core_r,
                egui::Stroke::new(1.0, egui::Color32::from_rgba_premultiplied(0, 0, 0, 0x40)),
            );

            // 6. Indicator stem + tick dots. The stem reads as a small
            // piece of inlaid white plastic sitting in the rubber rim,
            // cut to fit perfectly between the colored core and the
            // outer edge of the rubber sleeve.
            //
            //   Inner short side: concave arc on the visible core's
            //     outer circumference (radius=`visible_core_r`).
            //   Outer short side: arc on the rubber sleeve's outer
            //     circumference (radius=`radius`) — flush with the rim,
            //     not reaching out into the tick-dot zone.
            //
            // Tick dots stay where they are (`radius + 2.5`); the stem
            // and the dots are independent paint elements separated by
            // a small gap of chassis background, just like a real knob
            // where the pointer is on the rubber sleeve and the tick
            // marks are painted on the panel beside it.
            let start_angle = std::f32::consts::PI * 0.75;
            let sweep_range = std::f32::consts::PI * 1.5;
            let dot_center_r = radius + 2.5;
            let dot_radius = 0.75;
            let indicator_outer_r = radius;
            let stem_w = 2.0;
            let half_w = stem_w * 0.5;

            // Stem at the current value's angle.
            let angle = start_angle + sweep_range * norm;
            let dir = egui::vec2(angle.cos(), angle.sin());
            let perp = egui::vec2(-dir.y, dir.x);

            // Inner concave arc — the stem's inner short side sits at
            // exactly `visible_core_r` (the *visible* outer edge of the
            // colored core, after the recessed-under-bevel inset),
            // sweeping the angle subtended by the stem's width. Each arc
            // point is on the core's outer circumference, so the stem's
            // inner edge *is* part of the core's outer curve — perfect
            // inlay fit.
            let stem_angle = dir.y.atan2(dir.x);
            let inner_half_arc = (half_w / visible_core_r).asin();
            let outer_half_arc = (half_w / indicator_outer_r).asin();
            let n_arc = 4;

            let mut points: Vec<egui::Pos2> = Vec::with_capacity(2 * n_arc + 4);

            // Outer short side: arc at `indicator_outer_r = radius` —
            // the rubber sleeve's outer circumference. The stem's outer
            // edge sits flush with the rim, curving along the same arc
            // as the sleeve's outer perimeter.
            for i in 0..=n_arc {
                let t = i as f32 / n_arc as f32;
                let a = stem_angle + outer_half_arc - 2.0 * outer_half_arc * t;
                points.push(center + egui::vec2(a.cos(), a.sin()) * indicator_outer_r);
            }
            // Inner short side: concave arc on the visible core's outer
            // edge, swept the OTHER way (top → bot) so the polygon walks
            // CCW around the stem.
            for i in 0..=n_arc {
                let t = i as f32 / n_arc as f32;
                let a = stem_angle - inner_half_arc + 2.0 * inner_half_arc * t;
                points.push(center + egui::vec2(a.cos(), a.sin()) * visible_core_r);
            }
            let _ = perp; // perp computed for clarity; arcs use angles directly

            painter.add(egui::epaint::PathShape {
                points,
                closed: true,
                fill: theme::KNOB_INDICATOR,
                stroke: egui::epaint::PathStroke::NONE,
            });

            // Tick dots — uniform round white markers. When the indicator
            // angle matches a tick angle, indicator's tip kisses the
            // dot's inner edge.
            for i in 0..=10 {
                let tick_angle = start_angle + sweep_range * (i as f32 / 10.0);
                let tdir = egui::vec2(tick_angle.cos(), tick_angle.sin());
                let dot_center = center + tdir * dot_center_r;
                painter.circle_filled(dot_center, dot_radius, theme::KNOB_INDICATOR);
            }

            // 8. Write value to display when hovered/dragged. The expiry
            // timestamp lets the OUTPUT display linger on the most-recent
            // readout for ~500 ms after the user stops interacting, so
            // tweaking a knob and releasing doesn't blink the value off
            // immediately. Reader side (panels.rs) checks the expiry
            // before rendering and schedules a repaint when it lapses.
            if response.hovered() || response.dragged() {
                let display_text = format!("{} {}", label, format_value(*value));
                let expires_at = std::time::Instant::now()
                    + std::time::Duration::from_millis(500);
                ui.ctx().data_mut(|d| {
                    d.insert_temp(egui::Id::new("knob_display"), display_text);
                    d.insert_temp::<std::time::Instant>(
                        egui::Id::new("knob_display_expires"),
                        expires_at,
                    );
                });
            }
        }

        // Label below
        ui.add_space(label_gap);
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
