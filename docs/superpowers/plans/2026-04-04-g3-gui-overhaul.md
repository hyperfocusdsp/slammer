# G3 GUI Overhaul Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restyle Slammer's egui GUI to match the G3 "Chunky Industrial" mockup — dark panel, rubber+metal knobs, 7-segment displays, rack hardware details.

**Architecture:** Four files change. `theme.rs` gets a new colour palette. `knob.rs` gets a two-layer knob renderer (rubber ring + metal core + tapered indicator). `editor.rs` gets restructured layout (SUB+TOP shared row) with panel decorations (rack ears, screws, grooves, inset displays, 7-segment LCD). `plugin.rs` gets window size bump to 680x390.

**Tech Stack:** Rust, nih-plug, nih_plug_egui (egui 0.31.1). egui has no radial gradients — approximate with concentric filled circles and alpha blending.

**Reference mockup:** `mockups/mockup-g3.html` (canvas-rendered, open in browser to compare)

**Important egui gotchas (from project history):**
- `rect_stroke` needs 4 args: `(rect, rounding, stroke, StrokeKind::Outside)`
- `Margin::same()` takes `i8`, not `f32`
- Use `clip_rect().width()` not `available_width()` in horizontal layouts

---

### Task 1: Rewrite theme.rs — G3 colour palette

**Files:**
- Modify: `src/ui/theme.rs` (full rewrite, lines 1-79)

- [ ] **Step 1: Replace all colour constants**

Replace the entire contents of `src/ui/theme.rs` with:

```rust
use nih_plug_egui::egui;
use std::sync::Arc;

// Panel
pub const BG_PANEL: egui::Color32 = egui::Color32::from_rgb(0x13, 0x13, 0x13);
pub const BG_PANEL_EDGE: egui::Color32 = egui::Color32::from_rgb(0x1e, 0x1e, 0x1e);
pub const BG_RACK_EAR: egui::Color32 = egui::Color32::from_rgb(0x14, 0x14, 0x14);
pub const BG_VENT: egui::Color32 = egui::Color32::from_rgb(0x09, 0x09, 0x09);

// Display
pub const BG_DISPLAY: egui::Color32 = egui::Color32::from_rgb(0x04, 0x02, 0x02);
pub const BG_DISPLAY_FRAME: egui::Color32 = egui::Color32::from_rgb(0x08, 0x08, 0x08);

// Red LED / accent
pub const RED_LED: egui::Color32 = egui::Color32::from_rgb(0xff, 0x1a, 0x1a);
pub const RED_GLOW: egui::Color32 = egui::Color32::from_rgba_premultiplied(0x1e, 0x03, 0x03, 0x1e);
pub const RED_AMBIENT: egui::Color32 = egui::Color32::from_rgba_premultiplied(0x0e, 0x01, 0x01, 0x59);
pub const RED_GHOST: egui::Color32 = egui::Color32::from_rgba_premultiplied(0x09, 0x01, 0x01, 0x09);
pub const RED_WAVEFORM: egui::Color32 = egui::Color32::from_rgba_premultiplied(0x40, 0x06, 0x05, 0x40);

// Text
pub const WHITE: egui::Color32 = egui::Color32::from_rgb(0xdd, 0xdd, 0xdd);
pub const TEXT_DIM: egui::Color32 = egui::Color32::from_rgb(0x55, 0x55, 0x55);
pub const TEXT_GHOST: egui::Color32 = egui::Color32::from_rgba_premultiplied(0x12, 0x12, 0x12, 0x12);

// Knob
pub const KNOB_RUBBER: egui::Color32 = egui::Color32::from_rgb(0x1a, 0x1a, 0x1a);
pub const KNOB_RUBBER_HIGHLIGHT: egui::Color32 = egui::Color32::from_rgb(0x2a, 0x2a, 0x2a);
pub const KNOB_METAL: egui::Color32 = egui::Color32::from_rgb(0x88, 0x88, 0x88);
pub const KNOB_METAL_HIGHLIGHT: egui::Color32 = egui::Color32::from_rgb(0xaa, 0xaa, 0xaa);
pub const KNOB_METAL_DARK: egui::Color32 = egui::Color32::from_rgb(0x55, 0x55, 0x55);
pub const KNOB_BEVEL: egui::Color32 = egui::Color32::from_rgb(0x66, 0x66, 0x66);
pub const KNOB_RECESS: egui::Color32 = egui::Color32::from_rgba_premultiplied(0x00, 0x00, 0x00, 0x59);
pub const KNOB_INDICATOR: egui::Color32 = egui::Color32::from_rgb(0xee, 0xee, 0xee);
pub const KNOB_DIMPLE: egui::Color32 = egui::Color32::from_rgba_premultiplied(0x00, 0x00, 0x00, 0x26);

// Grooves & hardware
pub const GROOVE_DARK: egui::Color32 = egui::Color32::from_rgba_premultiplied(0x00, 0x00, 0x00, 0x99);
pub const GROOVE_LIGHT: egui::Color32 = egui::Color32::from_rgba_premultiplied(0x05, 0x05, 0x05, 0x05);
pub const SCREW_LIGHT: egui::Color32 = egui::Color32::from_rgb(0xaa, 0xaa, 0xaa);
pub const SCREW_DARK: egui::Color32 = egui::Color32::from_rgb(0x2a, 0x2a, 0x2a);
pub const SCREW_HEX: egui::Color32 = egui::Color32::from_rgb(0x1a, 0x1a, 0x1a);

// Arrow buttons
pub const BTN_LIGHT: egui::Color32 = egui::Color32::from_rgb(0x44, 0x44, 0x44);
pub const BTN_DARK: egui::Color32 = egui::Color32::from_rgb(0x1c, 0x1c, 0x1c);
pub const BTN_TEXT: egui::Color32 = egui::Color32::from_rgba_premultiplied(0x50, 0x50, 0x50, 0x73);

// Tick marks
pub const TICK_MAJOR: egui::Color32 = egui::Color32::from_rgba_premultiplied(0x50, 0x50, 0x50, 0x73);
pub const TICK_MINOR: egui::Color32 = egui::Color32::from_rgba_premultiplied(0x30, 0x30, 0x30, 0x26);

// Divider
pub const DIVIDER: egui::Color32 = egui::Color32::from_rgba_premultiplied(0x08, 0x08, 0x08, 0x08);

// Font
pub const FONT_NAME: &str = "JetBrains Mono";

pub fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    fonts.font_data.insert(
        FONT_NAME.to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../../assets/JetBrainsMono-Regular.ttf"
        ))),
    );

    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, FONT_NAME.to_owned());
    fonts
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default()
        .insert(0, FONT_NAME.to_owned());

    ctx.set_fonts(fonts);
}

pub fn setup_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    let visuals = &mut style.visuals;

    visuals.dark_mode = true;
    visuals.panel_fill = BG_PANEL;
    visuals.window_fill = BG_PANEL;
    visuals.extreme_bg_color = BG_PANEL;

    visuals.widgets.inactive.bg_fill = BG_PANEL;
    visuals.widgets.hovered.bg_fill = BG_PANEL;
    visuals.widgets.active.bg_fill = BG_PANEL;

    visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(3);
    visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(3);
    visuals.widgets.active.corner_radius = egui::CornerRadius::same(3);

    visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, TEXT_DIM);
    visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, WHITE);

    visuals.selection.bg_fill = RED_LED;

    ctx.set_style(style);
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check 2>&1 | head -30`

Expected: Compiler errors in `knob.rs` and `editor.rs` referencing removed constants (`ACCENT`, `KNOB_BG`, `KNOB_RING`, `KNOB_ARC`, `TEXT_PRIMARY`, `BG_MAIN`, `BG_SECTION`, `WAVEFORM_FILL`). This is expected — those files will be rewritten in subsequent tasks.

- [ ] **Step 3: Commit**

```bash
git add src/ui/theme.rs
git commit -m "refactor(ui): rewrite theme.rs with G3 colour palette"
```

---

### Task 2: Rewrite knob.rs — two-layer industrial knob

**Files:**
- Modify: `src/ui/knob.rs` (full rewrite, lines 1-111)

The knob must render without radial gradients (egui doesn't support them). Use concentric filled circles for the rubber/metal layers.

- [ ] **Step 1: Replace knob.rs contents**

Replace the entire contents of `src/ui/knob.rs` with:

```rust
use nih_plug_egui::egui;
use crate::ui::theme;

pub struct KnobResponse {
    pub changed: bool,
    pub reset: bool,
}

/// G3 industrial knob: rubber grip ring + beveled metal core + tapered indicator.
///
/// Vertical drag changes value, shift for fine control, ctrl+click to reset.
pub fn knob(
    ui: &mut egui::Ui,
    _id: egui::Id,
    value: &mut f32,
    min: f32,
    max: f32,
    default: f32,
    label: &str,
    format_value: impl Fn(f32) -> String,
    diameter: f32,
) -> KnobResponse {
    let mut result = KnobResponse {
        changed: false,
        reset: false,
    };

    ui.vertical(|ui| {
        ui.set_width(diameter + 12.0); // extra space for tick marks

        // Allocate knob area (with tick mark margin)
        let total = diameter + 12.0;
        let size = egui::vec2(total, total);
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
        let response = response.on_hover_cursor(egui::CursorIcon::ResizeVertical);

        // Ctrl+click to reset
        if response.clicked() && ui.input(|i| i.modifiers.ctrl) {
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
            painter.circle_filled(center + egui::vec2(0.5, 1.5), radius + 3.0, theme::KNOB_RECESS);
            painter.circle_filled(center, radius + 2.0, theme::KNOB_RECESS);

            // 2. Rubber grip ring (outer layer)
            painter.circle_filled(center, radius, theme::KNOB_RUBBER);
            // Subtle highlight on top half
            painter.circle_filled(
                center - egui::vec2(0.0, radius * 0.1),
                radius * 0.95,
                theme::KNOB_RUBBER_HIGHLIGHT,
            );
            painter.circle_filled(center, radius * 0.88, theme::KNOB_RUBBER);

            // 3. Bevel ring
            let core_radius = radius * 0.6;
            painter.circle_filled(center, core_radius + 1.5, theme::KNOB_BEVEL);

            // 4. Metal core face
            painter.circle_filled(center, core_radius, theme::KNOB_METAL);
            // Highlight: lighter upper-left quadrant
            painter.circle_filled(
                center - egui::vec2(core_radius * 0.15, core_radius * 0.15),
                core_radius * 0.7,
                theme::KNOB_METAL_HIGHLIGHT,
            );
            painter.circle_filled(center, core_radius * 0.5, theme::KNOB_METAL);

            // 5. Centre dimple
            painter.circle_filled(center, core_radius * 0.12, theme::KNOB_DIMPLE);

            // 6. Tapered indicator line
            let start_angle = std::f32::consts::PI * 0.75; // 7 o'clock
            let sweep_range = std::f32::consts::PI * 1.5; // 270 degrees
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
                let color = if is_major { theme::TICK_MAJOR } else { theme::TICK_MINOR };
                let width = if is_major { 1.0 } else { 0.5 };
                painter.line_segment([p1, p2], egui::Stroke::new(width, color));
            }

            // 8. Value text (shown on hover/drag)
            if response.hovered() || response.dragged() {
                let text = format_value(*value);
                painter.text(
                    center,
                    egui::Align2::CENTER_CENTER,
                    &text,
                    egui::FontId::new(8.0, egui::FontFamily::Monospace),
                    theme::WHITE,
                );
            }
        }

        // Label below
        ui.add_space(2.0);
        ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(label)
                    .font(egui::FontId::new(7.0, egui::FontFamily::Monospace))
                    .color(theme::WHITE),
            );
        });
    });

    result
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check 2>&1 | head -30`

Expected: Errors only from `editor.rs` (removed theme constants). `knob.rs` should compile clean.

- [ ] **Step 3: Commit**

```bash
git add src/ui/knob.rs
git commit -m "refactor(ui): rewrite knob.rs with G3 industrial two-layer style"
```

---

### Task 3: Update plugin.rs window size

**Files:**
- Modify: `src/plugin.rs:116`

- [ ] **Step 1: Change window size**

In `src/plugin.rs`, change line 116:

```rust
// OLD:
editor_state: EguiState::from_size(680, 360),
// NEW:
editor_state: EguiState::from_size(680, 390),
```

- [ ] **Step 2: Commit**

```bash
git add src/plugin.rs
git commit -m "chore(ui): bump window size to 680x390 for G3 layout"
```

---

### Task 4: Rewrite editor.rs — G3 layout with panel decorations

**Files:**
- Modify: `src/ui/editor.rs` (full rewrite, lines 1-305)

This is the largest task. The editor gets: panel decorations (rack ears, screws, groove lines), restructured layout (SUB+TOP on one row), inset waveform display with red styling, 7-segment LCD mode selector, and all knobs at 32px.

- [ ] **Step 1: Replace editor.rs contents**

Replace the entire contents of `src/ui/editor.rs` with:

```rust
use nih_plug::prelude::*;
use nih_plug_egui::egui;
use nih_plug_egui::{create_egui_editor, EguiState};
use parking_lot::Mutex;
use std::sync::Arc;

use crate::plugin::SlammerParams;
use crate::ui::knob;
use crate::ui::theme;
use crate::util::telemetry::TelemetryConsumer;

const KNOB_SIZE: f32 = 32.0;
const KNOB_SPACING: f32 = 46.0;
const RACK_EAR_W: f32 = 16.0;
const CONTENT_LEFT: f32 = RACK_EAR_W + 12.0;

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
) -> Option<Box<dyn Editor>> {
    let telemetry = Arc::new(Mutex::new(telemetry_rx));
    let waveform = Arc::new(Mutex::new(WaveformDisplay::new(200)));

    create_egui_editor(
        editor_state,
        (),
        |ctx, _| {
            theme::setup_fonts(ctx);
            theme::setup_style(ctx);
        },
        move |ctx, setter, _state| {
            // Drain telemetry
            {
                let mut tel = telemetry.lock();
                let mut wf = waveform.lock();
                if let Some(ref mut rx) = *tel {
                    let mut temp = Vec::new();
                    rx.drain_into(&mut temp, 128);
                    for &p in &temp {
                        wf.push(p);
                    }
                }
            }

            egui::CentralPanel::default().show(ctx, |ui| {
                ui.set_min_size(ui.available_size());
                let panel_rect = ui.max_rect();
                let painter = ui.painter();
                let w = panel_rect.width();
                let h = panel_rect.height();

                // ===== PANEL BACKGROUND =====
                // Gradient approximation: darker centre, lighter edges
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

                // ===== RACK EARS =====
                draw_rack_ear(painter, panel_rect.left(), panel_rect.top(), RACK_EAR_W, h);
                draw_rack_ear(
                    painter,
                    panel_rect.right() - RACK_EAR_W,
                    panel_rect.top(),
                    RACK_EAR_W,
                    h,
                );

                // ===== SCREWS =====
                let screw_r = 5.0;
                draw_screw(painter, panel_rect.left() + 8.0, panel_rect.top() + 18.0, screw_r);
                draw_screw(painter, panel_rect.left() + 8.0, panel_rect.bottom() - 18.0, screw_r);
                draw_screw(painter, panel_rect.right() - 8.0, panel_rect.top() + 18.0, screw_r);
                draw_screw(painter, panel_rect.right() - 8.0, panel_rect.bottom() - 18.0, screw_r);

                // ===== HEADER =====
                let header_y = panel_rect.top() + 10.0;
                painter.text(
                    egui::pos2(panel_rect.left() + CONTENT_LEFT, header_y),
                    egui::Align2::LEFT_TOP,
                    "SLAMMER",
                    egui::FontId::new(16.0, egui::FontFamily::Monospace),
                    theme::WHITE,
                );
                // LED indicator
                draw_led(painter, panel_rect.left() + CONTENT_LEFT + 120.0, header_y + 8.0, true);

                painter.text(
                    egui::pos2(panel_rect.right() - CONTENT_LEFT, header_y),
                    egui::Align2::RIGHT_TOP,
                    "KICK SYNTHESIZER",
                    egui::FontId::new(8.0, egui::FontFamily::Monospace),
                    theme::TEXT_DIM,
                );
                painter.text(
                    egui::pos2(panel_rect.right() - CONTENT_LEFT, header_y + 11.0),
                    egui::Align2::RIGHT_TOP,
                    &format!("v{}", env!("CARGO_PKG_VERSION")),
                    egui::FontId::new(8.0, egui::FontFamily::Monospace),
                    egui::Color32::from_rgb(0x44, 0x44, 0x44),
                );

                // ===== GROOVE: below header =====
                let groove_y = panel_rect.top() + 36.0;
                draw_groove(painter, panel_rect.left() + CONTENT_LEFT - 4.0, panel_rect.right() - CONTENT_LEFT + 4.0, groove_y);

                // ===== MASTER ROW: Waveform + knobs =====
                let master_y = groove_y + 6.0;
                let wf_left = panel_rect.left() + CONTENT_LEFT;
                let wf_width = w - CONTENT_LEFT * 2.0 - 160.0;
                let wf_height = 56.0;

                // Waveform display (inset)
                draw_inset_display(painter, wf_left, master_y, wf_width, wf_height);
                // "OUTPUT" label
                painter.text(
                    egui::pos2(wf_left + 4.0, master_y + 3.0),
                    egui::Align2::LEFT_TOP,
                    "OUTPUT",
                    egui::FontId::new(6.0, egui::FontFamily::Monospace),
                    theme::RED_GHOST,
                );
                // Draw live waveform
                {
                    let wf = waveform.lock();
                    if !wf.peaks.is_empty() {
                        let n = wf.peaks.len();
                        let mid_y = master_y + wf_height / 2.0;
                        for (i, &peak) in wf.peaks.iter().enumerate() {
                            let x = wf_left + 2.0 + (i as f32 / n as f32) * (wf_width - 4.0);
                            let amp = peak.min(1.0) * wf_height * 0.42;
                            painter.line_segment(
                                [egui::pos2(x, mid_y - amp), egui::pos2(x, mid_y + amp)],
                                egui::Stroke::new(1.2, theme::RED_WAVEFORM),
                            );
                        }
                    }
                }

                // Master knobs — placed via absolute positioning within the ui
                let knob_row_y = master_y + 4.0;
                let knobs_x = wf_left + wf_width + 16.0;

                // We need to use ui layout for knobs because they are interactive widgets.
                // Position the cursor to the master knob area.
                let master_knob_rect = egui::Rect::from_min_size(
                    egui::pos2(knobs_x, knob_row_y),
                    egui::vec2(150.0, 56.0),
                );
                ui.allocate_ui_at_rect(master_knob_rect, |ui| {
                    ui.horizontal(|ui| {
                        param_knob(ui, setter, "decay", "DECAY", &params.decay_ms, 50.0, 3000.0, 400.0, |v| format!("{:.0}ms", v), KNOB_SIZE);
                        param_knob(ui, setter, "vel", "VEL", &params.velocity_sens, 0.0, 1.0, 0.8, |v| format!("{:.0}%", v * 100.0), KNOB_SIZE);
                        let mut vol_db = util::gain_to_db(params.master_volume.value());
                        if knob::knob(ui, egui::Id::new("master"), &mut vol_db, -60.0, 6.0, 0.0, "VOL",
                            |v| if v <= -59.0 { "-inf".into() } else { format!("{:.1}dB", v) }, KNOB_SIZE,
                        ).changed {
                            setter.begin_set_parameter(&params.master_volume);
                            setter.set_parameter(&params.master_volume, util::db_to_gain(vol_db));
                            setter.end_set_parameter(&params.master_volume);
                        }
                    });
                });

                // ===== ROW 1: SUB | TOP =====
                let row1_label_y = master_y + wf_height + 8.0;
                let row1_groove_y = row1_label_y + 12.0;
                let row1_knob_y = row1_groove_y + 4.0;

                // Section labels
                painter.text(
                    egui::pos2(panel_rect.left() + CONTENT_LEFT, row1_label_y),
                    egui::Align2::LEFT_TOP,
                    "SUB",
                    egui::FontId::new(10.0, egui::FontFamily::Monospace),
                    theme::WHITE,
                );

                // Vertical divider position: after 6 SUB knobs
                let divider_x = panel_rect.left() + CONTENT_LEFT + KNOB_SPACING * 6.0 + 12.0;
                painter.text(
                    egui::pos2(divider_x + 10.0, row1_label_y),
                    egui::Align2::LEFT_TOP,
                    "TOP",
                    egui::FontId::new(10.0, egui::FontFamily::Monospace),
                    theme::WHITE,
                );

                draw_groove(painter, panel_rect.left() + CONTENT_LEFT - 4.0, panel_rect.right() - CONTENT_LEFT + 4.0, row1_groove_y);

                // Vertical divider
                painter.line_segment(
                    [egui::pos2(divider_x, row1_groove_y + 2.0), egui::pos2(divider_x, row1_knob_y + KNOB_SIZE + 20.0)],
                    egui::Stroke::new(1.0, theme::DIVIDER),
                );

                // SUB knobs
                let sub_knob_rect = egui::Rect::from_min_size(
                    egui::pos2(panel_rect.left() + CONTENT_LEFT, row1_knob_y),
                    egui::vec2(KNOB_SPACING * 6.0, KNOB_SIZE + 20.0),
                );
                ui.allocate_ui_at_rect(sub_knob_rect, |ui| {
                    ui.horizontal(|ui| {
                        param_knob(ui, setter, "s_g", "GAIN", &params.sub_gain, 0.0, 1.0, 0.85, |v| format!("{:.0}%", v * 100.0), KNOB_SIZE);
                        param_knob(ui, setter, "s_fs", "START", &params.sub_fstart, 20.0, 800.0, 150.0, |v| format!("{:.0}Hz", v), KNOB_SIZE);
                        param_knob(ui, setter, "s_fe", "END", &params.sub_fend, 20.0, 400.0, 45.0, |v| format!("{:.0}Hz", v), KNOB_SIZE);
                        param_knob(ui, setter, "s_sw", "SWEEP", &params.sub_sweep_ms, 5.0, 500.0, 60.0, |v| format!("{:.0}ms", v), KNOB_SIZE);
                        param_knob(ui, setter, "s_cv", "CURVE", &params.sub_sweep_curve, 0.5, 12.0, 3.0, |v| format!("{:.1}", v), KNOB_SIZE);
                        param_knob(ui, setter, "s_ph", "PHASE", &params.sub_phase_offset, 0.0, 360.0, 90.0, |v| format!("{:.0}\u{00b0}", v), KNOB_SIZE);
                    });
                });

                // TOP knobs
                let top_knob_rect = egui::Rect::from_min_size(
                    egui::pos2(divider_x + 10.0, row1_knob_y),
                    egui::vec2(KNOB_SPACING * 4.0, KNOB_SIZE + 20.0),
                );
                ui.allocate_ui_at_rect(top_knob_rect, |ui| {
                    ui.horizontal(|ui| {
                        param_knob(ui, setter, "t_g", "GAIN", &params.top_gain, 0.0, 1.0, 0.25, |v| format!("{:.0}%", v * 100.0), KNOB_SIZE);
                        param_knob(ui, setter, "t_dc", "DECAY", &params.top_decay_ms, 1.0, 50.0, 6.0, |v| format!("{:.1}ms", v), KNOB_SIZE);
                        param_knob(ui, setter, "t_f", "FREQ", &params.top_freq, 1000.0, 8000.0, 3500.0, |v| format!("{:.0}Hz", v), KNOB_SIZE);
                        param_knob(ui, setter, "t_bw", "BW", &params.top_bw, 0.2, 3.0, 1.5, |v| format!("{:.1}oct", v), KNOB_SIZE);
                    });
                });

                // ===== ROW 2: MID =====
                let row2_label_y = row1_knob_y + KNOB_SIZE + 24.0;
                let row2_groove_y = row2_label_y + 12.0;
                let row2_knob_y = row2_groove_y + 4.0;

                painter.text(
                    egui::pos2(panel_rect.left() + CONTENT_LEFT, row2_label_y),
                    egui::Align2::LEFT_TOP,
                    "MID",
                    egui::FontId::new(10.0, egui::FontFamily::Monospace),
                    theme::WHITE,
                );
                draw_groove(painter, panel_rect.left() + CONTENT_LEFT - 4.0, panel_rect.right() - CONTENT_LEFT + 4.0, row2_groove_y);

                let mid_knob_rect = egui::Rect::from_min_size(
                    egui::pos2(panel_rect.left() + CONTENT_LEFT, row2_knob_y),
                    egui::vec2(KNOB_SPACING * 9.0, KNOB_SIZE + 20.0),
                );
                ui.allocate_ui_at_rect(mid_knob_rect, |ui| {
                    ui.horizontal(|ui| {
                        param_knob(ui, setter, "m_g", "GAIN", &params.mid_gain, 0.0, 1.0, 0.5, |v| format!("{:.0}%", v * 100.0), KNOB_SIZE);
                        param_knob(ui, setter, "m_fs", "START", &params.mid_fstart, 100.0, 2000.0, 400.0, |v| format!("{:.0}Hz", v), KNOB_SIZE);
                        param_knob(ui, setter, "m_fe", "END", &params.mid_fend, 50.0, 800.0, 120.0, |v| format!("{:.0}Hz", v), KNOB_SIZE);
                        param_knob(ui, setter, "m_sw", "SWEEP", &params.mid_sweep_ms, 3.0, 300.0, 30.0, |v| format!("{:.0}ms", v), KNOB_SIZE);
                        param_knob(ui, setter, "m_cv", "CURVE", &params.mid_sweep_curve, 0.5, 12.0, 4.0, |v| format!("{:.1}", v), KNOB_SIZE);
                        param_knob(ui, setter, "m_dc", "DECAY", &params.mid_decay_ms, 20.0, 1000.0, 150.0, |v| format!("{:.0}ms", v), KNOB_SIZE);
                        param_knob(ui, setter, "m_tn", "TONE", &params.mid_tone_gain, 0.0, 1.0, 0.7, |v| format!("{:.0}%", v * 100.0), KNOB_SIZE);
                        param_knob(ui, setter, "m_ns", "NOISE", &params.mid_noise_gain, 0.0, 1.0, 0.3, |v| format!("{:.0}%", v * 100.0), KNOB_SIZE);
                        param_knob(ui, setter, "m_nc", "COLOR", &params.mid_noise_color, 0.0, 1.0, 0.4, |v| format!("{:.0}%", v * 100.0), KNOB_SIZE);
                    });
                });

                // ===== ROW 3: SAT | EQ =====
                let row3_label_y = row2_knob_y + KNOB_SIZE + 24.0;
                let row3_groove_y = row3_label_y + 12.0;
                let row3_knob_y = row3_groove_y + 4.0;

                painter.text(
                    egui::pos2(panel_rect.left() + CONTENT_LEFT, row3_label_y),
                    egui::Align2::LEFT_TOP,
                    "SAT",
                    egui::FontId::new(10.0, egui::FontFamily::Monospace),
                    theme::WHITE,
                );

                let eq_divider_x = panel_rect.left() + CONTENT_LEFT + KNOB_SPACING * 4.0 + 40.0;
                painter.text(
                    egui::pos2(eq_divider_x + 10.0, row3_label_y),
                    egui::Align2::LEFT_TOP,
                    "EQ",
                    egui::FontId::new(10.0, egui::FontFamily::Monospace),
                    theme::WHITE,
                );

                draw_groove(painter, panel_rect.left() + CONTENT_LEFT - 4.0, panel_rect.right() - CONTENT_LEFT + 4.0, row3_groove_y);

                // Vertical divider
                painter.line_segment(
                    [egui::pos2(eq_divider_x, row3_groove_y + 2.0), egui::pos2(eq_divider_x, row3_knob_y + KNOB_SIZE + 20.0)],
                    egui::Stroke::new(1.0, theme::DIVIDER),
                );

                // SAT: LCD selector + 2 knobs
                let sat_rect = egui::Rect::from_min_size(
                    egui::pos2(panel_rect.left() + CONTENT_LEFT, row3_knob_y),
                    egui::vec2(KNOB_SPACING * 4.0 + 36.0, KNOB_SIZE + 20.0),
                );
                ui.allocate_ui_at_rect(sat_rect, |ui| {
                    ui.horizontal(|ui| {
                        lcd_selector(ui, setter, &params.sat_mode);
                        ui.add_space(4.0);
                        param_knob(ui, setter, "sat_d", "DRIVE", &params.sat_drive, 0.0, 1.0, 0.0, |v| format!("{:.0}%", v * 100.0), KNOB_SIZE);
                        param_knob(ui, setter, "sat_x", "MIX", &params.sat_mix, 0.0, 1.0, 1.0, |v| format!("{:.0}%", v * 100.0), KNOB_SIZE);
                    });
                });

                // EQ: 5 knobs
                let eq_rect = egui::Rect::from_min_size(
                    egui::pos2(eq_divider_x + 10.0, row3_knob_y),
                    egui::vec2(KNOB_SPACING * 5.0, KNOB_SIZE + 20.0),
                );
                ui.allocate_ui_at_rect(eq_rect, |ui| {
                    ui.horizontal(|ui| {
                        param_knob(ui, setter, "eq_t", "TILT", &params.eq_tilt_db, -6.0, 6.0, 0.0, |v| format!("{:+.1}dB", v), KNOB_SIZE);
                        param_knob(ui, setter, "eq_l", "LOW", &params.eq_low_boost_db, -3.0, 9.0, 0.0, |v| format!("{:+.1}dB", v), KNOB_SIZE);
                        param_knob(ui, setter, "eq_nf", "NOTCH", &params.eq_notch_freq, 100.0, 600.0, 250.0, |v| format!("{:.0}Hz", v), KNOB_SIZE);
                        param_knob(ui, setter, "eq_nq", "Q", &params.eq_notch_q, 0.0, 10.0, 0.0, |v| format!("{:.1}", v), KNOB_SIZE);
                        param_knob(ui, setter, "eq_nd", "DEPTH", &params.eq_notch_depth_db, 0.0, 20.0, 12.0, |v| format!("{:.0}dB", v), KNOB_SIZE);
                    });
                });

                // ===== FOOTER =====
                let footer_groove_y = panel_rect.bottom() - 20.0;
                draw_groove(painter, panel_rect.left() + CONTENT_LEFT - 4.0, panel_rect.right() - CONTENT_LEFT + 4.0, footer_groove_y);
                painter.text(
                    egui::pos2(panel_rect.center().x, panel_rect.bottom() - 8.0),
                    egui::Align2::CENTER_BOTTOM,
                    "REXIST INSTRUMENTS",
                    egui::FontId::new(7.0, egui::FontFamily::Monospace),
                    theme::TEXT_GHOST,
                );
            });
        },
    )
}

// ===== Drawing helpers =====

fn draw_rack_ear(painter: &egui::Painter, x: f32, y: f32, width: f32, height: f32) {
    painter.rect_filled(
        egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(width, height)),
        0.0,
        theme::BG_RACK_EAR,
    );
    // Vent slots
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

fn draw_screw(painter: &egui::Painter, cx: f32, cy: f32, radius: f32) {
    let center = egui::pos2(cx, cy);
    // Outer ring
    painter.circle_filled(center, radius, theme::SCREW_LIGHT);
    painter.circle_filled(center, radius * 0.85, theme::KNOB_METAL);
    painter.circle_filled(center, radius * 0.7, theme::SCREW_DARK);
    // Hex recess (6 short lines from centre)
    for i in 0..6 {
        let angle = (i as f32 / 6.0) * std::f32::consts::TAU - std::f32::consts::PI / 6.0;
        let p = center + egui::vec2(angle.cos(), angle.sin()) * radius * 0.4;
        painter.circle_filled(p, 1.0, theme::SCREW_HEX);
    }
}

fn draw_groove(painter: &egui::Painter, left: f32, right: f32, y: f32) {
    painter.line_segment(
        [egui::pos2(left, y), egui::pos2(right, y)],
        egui::Stroke::new(1.0, theme::GROOVE_DARK),
    );
    painter.line_segment(
        [egui::pos2(left, y + 1.0), egui::pos2(right, y + 1.0)],
        egui::Stroke::new(0.5, theme::GROOVE_LIGHT),
    );
}

fn draw_led(painter: &egui::Painter, cx: f32, cy: f32, on: bool) {
    let center = egui::pos2(cx, cy);
    // LED housing
    painter.circle_filled(center, 4.0, egui::Color32::from_rgb(0x08, 0x08, 0x08));
    // LED face
    let color = if on { theme::RED_LED } else { egui::Color32::from_rgb(0x2a, 0x08, 0x08) };
    painter.circle_filled(center, 3.0, color);
    // Glow
    if on {
        painter.circle_filled(center, 8.0, theme::RED_GLOW);
    }
}

fn draw_inset_display(painter: &egui::Painter, x: f32, y: f32, w: f32, h: f32) {
    // Outer bezel
    painter.rect_filled(
        egui::Rect::from_min_size(egui::pos2(x - 4.0, y - 4.0), egui::vec2(w + 8.0, h + 8.0)),
        4.0,
        theme::BG_DISPLAY_FRAME,
    );
    // Inner fill
    painter.rect_filled(
        egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h)),
        0.0,
        theme::BG_DISPLAY,
    );
    // Scanlines
    let mut sy = y;
    while sy < y + h {
        painter.line_segment(
            [egui::pos2(x, sy), egui::pos2(x + w, sy)],
            egui::Stroke::new(1.0, egui::Color32::from_rgba_premultiplied(0, 0, 0, 20)),
        );
        sy += 2.0;
    }
    // Red ambient glow (centre rectangle, faded)
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

/// 7-segment character map: [top, top-right, bottom-right, bottom, bottom-left, top-left, middle]
const SEG_MAP: &[(char, [bool; 7])] = &[
    ('0', [true,  true,  true,  true,  true,  true,  false]),
    ('1', [false, true,  true,  false, false, false, false]),
    ('2', [true,  true,  false, true,  true,  false, true]),
    ('3', [true,  true,  true,  true,  false, false, true]),
    ('4', [false, true,  true,  false, false, true,  true]),
    ('5', [true,  false, true,  true,  false, true,  true]),
    ('6', [true,  false, true,  true,  true,  true,  true]),
    ('7', [true,  true,  true,  false, false, false, false]),
    ('8', [true,  true,  true,  true,  true,  true,  true]),
    ('9', [true,  true,  true,  true,  false, true,  true]),
    ('A', [true,  true,  true,  false, true,  true,  true]),
    ('D', [false, true,  true,  true,  true,  false, true]),
    ('E', [true,  false, false, true,  true,  true,  true]),
    ('F', [true,  false, false, false, true,  true,  true]),
    ('I', [false, true,  true,  false, false, false, false]),
    ('O', [true,  true,  true,  true,  true,  true,  false]),
    ('P', [true,  true,  false, false, true,  true,  true]),
    ('S', [true,  false, true,  true,  false, true,  true]),
    ('T', [false, false, false, true,  true,  true,  true]),
    ('L', [false, false, false, true,  true,  true,  false]),
    ('H', [false, true,  true,  false, true,  true,  true]),
    (' ', [false, false, false, false, false, false, false]),
    ('-', [false, false, false, false, false, false, true]),
];

fn seg_lookup(ch: char) -> [bool; 7] {
    let upper = ch.to_ascii_uppercase();
    for &(c, segs) in SEG_MAP {
        if c == upper {
            return segs;
        }
    }
    [false; 7] // space for unknown
}

/// Draw a single 7-segment character at (px, py) with char_w x char_h bounding box.
fn draw_7seg_char(painter: &egui::Painter, px: f32, py: f32, ch: char, char_w: f32, char_h: f32) {
    let segs = seg_lookup(ch);
    let t = 1.8; // segment thickness
    let half_h = (char_h - 3.0) / 2.0;

    // Segment positions: each is a small rect
    // [top, top-right, bottom-right, bottom, bottom-left, top-left, middle]
    let rects = [
        // 0: top horizontal
        egui::Rect::from_min_size(egui::pos2(px + t, py), egui::vec2(char_w - t * 2.0, t)),
        // 1: top-right vertical
        egui::Rect::from_min_size(egui::pos2(px + char_w - t, py + 0.5), egui::vec2(t, half_h)),
        // 2: bottom-right vertical
        egui::Rect::from_min_size(egui::pos2(px + char_w - t, py + half_h + 1.5), egui::vec2(t, half_h)),
        // 3: bottom horizontal
        egui::Rect::from_min_size(egui::pos2(px + t, py + char_h - t), egui::vec2(char_w - t * 2.0, t)),
        // 4: bottom-left vertical
        egui::Rect::from_min_size(egui::pos2(px, py + half_h + 1.5), egui::vec2(t, half_h)),
        // 5: top-left vertical
        egui::Rect::from_min_size(egui::pos2(px, py + 0.5), egui::vec2(t, half_h)),
        // 6: middle horizontal
        egui::Rect::from_min_size(egui::pos2(px + t, py + half_h + 0.5), egui::vec2(char_w - t * 2.0, t)),
    ];

    for (i, &r) in rects.iter().enumerate() {
        let color = if segs[i] { theme::RED_LED } else { theme::RED_GHOST };
        painter.rect_filled(r, 0.0, color);
    }
}

/// Draw 7-segment text centred in a rectangle.
fn draw_7seg_text(painter: &egui::Painter, rect: egui::Rect, text: &str) {
    let char_h = rect.height() * 0.65;
    let char_w = char_h * 0.6;
    let gap = 3.0;
    let total_w = text.len() as f32 * (char_w + gap) - gap;
    let mut cx = rect.center().x - total_w / 2.0;
    let cy = rect.center().y - char_h / 2.0;
    for ch in text.chars() {
        draw_7seg_char(painter, cx, cy, ch, char_w, char_h);
        cx += char_w + gap;
    }
}

/// LCD-style mode selector: [<] 7SEG_MODE [>]
fn lcd_selector(ui: &mut egui::Ui, setter: &ParamSetter, param: &FloatParam) {
    const MODES: &[&str] = &["OFF", "SOFT", "DIODE", "TAPE"];
    let current = param.value() as usize;

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;

        // Left arrow button
        let btn_w = 18.0;
        let btn_h = 26.0;
        let (_, left_rect) = ui.allocate_space(egui::vec2(btn_w, btn_h));
        if ui.is_rect_visible(left_rect) {
            let painter = ui.painter();
            painter.rect_filled(left_rect, 3.0, theme::BTN_DARK);
            painter.rect_filled(
                egui::Rect::from_min_size(left_rect.min, egui::vec2(btn_w, btn_h * 0.4)),
                3.0,
                theme::BTN_LIGHT,
            );
            painter.text(
                left_rect.center(),
                egui::Align2::CENTER_CENTER,
                "\u{25C0}",
                egui::FontId::new(9.0, egui::FontFamily::Monospace),
                if current > 0 { theme::BTN_TEXT } else { theme::GROOVE_LIGHT },
            );
        }
        let left_resp = ui.interact(left_rect, egui::Id::new("sat_left"), egui::Sense::click());
        if left_resp.clicked() && current > 0 {
            setter.begin_set_parameter(param);
            setter.set_parameter(param, (current - 1) as f32);
            setter.end_set_parameter(param);
        }

        // LCD display with 7-segment text
        let lcd_width = 66.0;
        let lcd_height = 26.0;
        let (_, lcd_rect) = ui.allocate_space(egui::vec2(lcd_width, lcd_height));
        if ui.is_rect_visible(lcd_rect) {
            let painter = ui.painter();
            draw_inset_display(painter, lcd_rect.left(), lcd_rect.top(), lcd_width, lcd_height);
            let mode_name = MODES.get(current).unwrap_or(&"OFF");
            draw_7seg_text(painter, lcd_rect, mode_name);
        }

        // Right arrow button
        let (_, right_rect) = ui.allocate_space(egui::vec2(btn_w, btn_h));
        if ui.is_rect_visible(right_rect) {
            let painter = ui.painter();
            painter.rect_filled(right_rect, 3.0, theme::BTN_DARK);
            painter.rect_filled(
                egui::Rect::from_min_size(right_rect.min, egui::vec2(btn_w, btn_h * 0.4)),
                3.0,
                theme::BTN_LIGHT,
            );
            painter.text(
                right_rect.center(),
                egui::Align2::CENTER_CENTER,
                "\u{25B6}",
                egui::FontId::new(9.0, egui::FontFamily::Monospace),
                if current < MODES.len() - 1 { theme::BTN_TEXT } else { theme::GROOVE_LIGHT },
            );
        }
        let right_resp = ui.interact(right_rect, egui::Id::new("sat_right"), egui::Sense::click());
        if right_resp.clicked() && current < MODES.len() - 1 {
            setter.begin_set_parameter(param);
            setter.set_parameter(param, (current + 1) as f32);
            setter.end_set_parameter(param);
        }

        ui.add_space(8.0);
    });
}

fn param_knob(
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
) {
    let mut val = param.value();
    if knob::knob(ui, egui::Id::new(id), &mut val, min, max, default, label, format_value, diameter).changed {
        setter.begin_set_parameter(param);
        setter.set_parameter(param, val);
        setter.end_set_parameter(param);
    }
}
```

- [ ] **Step 2: Verify full project compiles**

Run: `cargo check 2>&1 | tail -10`

Expected: Clean compile (possibly warnings about unused imports from removed constants — those are fine).

- [ ] **Step 3: Commit**

```bash
git add src/ui/editor.rs
git commit -m "feat(ui): rewrite editor.rs with G3 industrial layout and decorations"
```

---

### Task 5: Build, run, and verify visually

**Files:** None (verification only)

- [ ] **Step 1: Run all tests**

Run: `cargo test 2>&1 | tail -5`

Expected: `test result: ok. 30 passed` (all DSP tests still pass, no UI tests exist)

- [ ] **Step 2: Build standalone**

Run: `cargo build --release 2>&1 | tail -5`

Expected: Clean build.

- [ ] **Step 3: Run standalone and verify visually**

Run: `cargo run --release 2>&1 &`

Check visually:
- Window opens at 680x390
- Dark panel with rack ears on left/right, screws in corners
- Header: "SLAMMER" left, "KICK SYNTHESIZER" right, red LED
- Groove lines between sections
- SUB and TOP on same row with vertical divider
- MID full width
- SAT with LCD selector (7-segment text) and EQ with divider
- Knobs show rubber ring + metal core, tapered indicator
- Waveform display has dark inset bezel
- "REXIST INSTRUMENTS" ghost text in footer

Close the standalone after verifying.

- [ ] **Step 4: Bundle and install plugin**

Run: `cargo run --release -p xtask -- bundle slammer --release 2>&1 | tail -5`
Then: `cp -v target/bundled/slammer.clap ~/.clap/ && cp -rv target/bundled/slammer.vst3 ~/.vst3/`

- [ ] **Step 5: Commit any fixes**

If visual verification revealed issues, fix them and commit:

```bash
git add -u
git commit -m "fix(ui): adjust G3 layout spacing and alignment"
```

If no fixes needed, skip this step.

---

### Task 6: Cleanup and final commit

**Files:** None new

- [ ] **Step 1: Run cargo clippy**

Run: `cargo clippy 2>&1 | tail -20`

Fix any warnings in the UI files.

- [ ] **Step 2: Remove unused imports/constants**

Check for and remove any dead code warnings in theme.rs, knob.rs, editor.rs.

- [ ] **Step 3: Final test run**

Run: `cargo test 2>&1 | tail -5`

Expected: All 30 tests pass.

- [ ] **Step 4: Commit cleanup**

```bash
git add -u
git commit -m "chore(ui): cleanup warnings and dead code from G3 overhaul"
```
