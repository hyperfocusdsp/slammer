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
pub const RED_AMBIENT: egui::Color32 =
    egui::Color32::from_rgba_premultiplied(0x0e, 0x01, 0x01, 0x59);
pub const RED_GHOST: egui::Color32 = egui::Color32::from_rgba_premultiplied(0x09, 0x01, 0x01, 0x09);
pub const RED_WAVEFORM: egui::Color32 =
    egui::Color32::from_rgba_premultiplied(0x40, 0x06, 0x05, 0x40);

// Text
// Brand bone — sampled from `assets/hyperfocus_dsp_logo.png` (#F4F1EA).
// Used everywhere we'd previously use plain "white" for foreground content,
// so all UI text/highlights pick up the same cream as the wordmark.
pub const WHITE: egui::Color32 = egui::Color32::from_rgb(0xf4, 0xf1, 0xea);
pub const TEXT_DIM: egui::Color32 = egui::Color32::from_rgb(0x55, 0x55, 0x55);
pub const TEXT_GHOST: egui::Color32 =
    egui::Color32::from_rgba_premultiplied(0x12, 0x12, 0x12, 0x12);

// Knob
pub const KNOB_RUBBER: egui::Color32 = egui::Color32::from_rgb(0x1a, 0x1a, 0x1a);
pub const KNOB_RUBBER_HIGHLIGHT: egui::Color32 = egui::Color32::from_rgb(0x2a, 0x2a, 0x2a);
pub const KNOB_METAL: egui::Color32 = egui::Color32::from_rgb(0x88, 0x88, 0x88);
pub const KNOB_METAL_HIGHLIGHT: egui::Color32 = egui::Color32::from_rgb(0xaa, 0xaa, 0xaa);
pub const KNOB_BEVEL: egui::Color32 = egui::Color32::from_rgb(0x66, 0x66, 0x66);
pub const KNOB_RECESS: egui::Color32 =
    egui::Color32::from_rgba_premultiplied(0x00, 0x00, 0x00, 0x59);
pub const KNOB_INDICATOR: egui::Color32 = egui::Color32::from_rgb(0xf4, 0xf1, 0xea);
pub const KNOB_DIMPLE: egui::Color32 =
    egui::Color32::from_rgba_premultiplied(0x00, 0x00, 0x00, 0x26);

// Section knob cores (Vintage MPC palette)
pub const SECTION_TOP: egui::Color32 = egui::Color32::from_rgb(0xd4, 0x95, 0x26); // amber
pub const SECTION_SUB: egui::Color32 = egui::Color32::from_rgb(0x85, 0x90, 0xa0); // blue-grey steel
pub const SECTION_MID: egui::Color32 = egui::Color32::from_rgb(0x4e, 0x9a, 0x52); // forest green
pub const SECTION_SAT: egui::Color32 = egui::Color32::from_rgb(0xc5, 0x2e, 0x2e); // oxide red
pub const SECTION_EQ: egui::Color32 = egui::Color32::from_rgb(0x3d, 0x44, 0x4b); // gunmetal
pub const SECTION_MASTER: egui::Color32 = egui::Color32::from_rgb(0x3a, 0x78, 0xc8); // electric blue

// Grooves & hardware
pub const GROOVE_DARK: egui::Color32 =
    egui::Color32::from_rgba_premultiplied(0x00, 0x00, 0x00, 0x99);
pub const GROOVE_LIGHT: egui::Color32 =
    egui::Color32::from_rgba_premultiplied(0x05, 0x05, 0x05, 0x05);
pub const SCREW_LIGHT: egui::Color32 = egui::Color32::from_rgb(0xaa, 0xaa, 0xaa);
pub const SCREW_DARK: egui::Color32 = egui::Color32::from_rgb(0x2a, 0x2a, 0x2a);
pub const SCREW_HEX: egui::Color32 = egui::Color32::from_rgb(0x1a, 0x1a, 0x1a);

// Legacy 2-tone button (still used by sequencer step bodies; the 3D
// helper below is preferred for any new chrome).
pub const BTN_LIGHT: egui::Color32 = egui::Color32::from_rgb(0x44, 0x44, 0x44);
pub const BTN_DARK: egui::Color32 = egui::Color32::from_rgb(0x1c, 0x1c, 0x1c);
pub const BTN_TEXT: egui::Color32 = egui::Color32::from_rgba_premultiplied(0x50, 0x50, 0x50, 0x73);

// 3D button gradient stops (used by `widgets::draw_button_3d`).
// A real plastic/rubber cap has a bright spec at the top, midtone body,
// and deep self-shadow at the base — the gradient interpolates these.
pub const BTN_HIGHLIGHT_TOP: egui::Color32 = egui::Color32::from_rgb(0x5e, 0x5e, 0x5e);
pub const BTN_TOP: egui::Color32 = egui::Color32::from_rgb(0x46, 0x46, 0x46);
pub const BTN_MID: egui::Color32 = egui::Color32::from_rgb(0x2c, 0x2c, 0x2c);
pub const BTN_BOTTOM: egui::Color32 = egui::Color32::from_rgb(0x16, 0x16, 0x16);
pub const BTN_BOTTOM_DEEP: egui::Color32 = egui::Color32::from_rgb(0x0a, 0x0a, 0x0a);
pub const BTN_EDGE: egui::Color32 = egui::Color32::from_rgb(0x28, 0x28, 0x28);
pub const BTN_TOP_SHEEN: egui::Color32 =
    egui::Color32::from_rgba_premultiplied(0xff, 0xff, 0xff, 0x44);
pub const BTN_BOT_LEDGE: egui::Color32 =
    egui::Color32::from_rgba_premultiplied(0x00, 0x00, 0x00, 0x88);
pub const BTN_DROP_SHADOW: egui::Color32 =
    egui::Color32::from_rgba_premultiplied(0x00, 0x00, 0x00, 0x70);
// Recessed-mounting "well" — the chassis cutout the button cap sits in.
// Rendered as a 1-px ring around each button so the cap reads as a real
// piece of hardware seated in a panel rather than printed on it. Top-edge
// shadow is darker (TR-909-style) to suggest the cap is overhanging.
pub const BTN_WELL: egui::Color32 = egui::Color32::from_rgb(0x05, 0x05, 0x05);
pub const BTN_WELL_TOP_SHADOW: egui::Color32 =
    egui::Color32::from_rgba_premultiplied(0x00, 0x00, 0x00, 0xa0);

// Display reflection (subtle white sheen on lit area top + 1-px specular line)
pub const DISPLAY_SHEEN: egui::Color32 =
    egui::Color32::from_rgba_premultiplied(0xff, 0xff, 0xff, 0x12);
pub const DISPLAY_SPECULAR: egui::Color32 =
    egui::Color32::from_rgba_premultiplied(0xff, 0xff, 0xff, 0x18);

// Tick marks
pub const TICK_MAJOR: egui::Color32 =
    egui::Color32::from_rgba_premultiplied(0x50, 0x50, 0x50, 0x73);
pub const TICK_MINOR: egui::Color32 =
    egui::Color32::from_rgba_premultiplied(0x30, 0x30, 0x30, 0x26);

// Divider
pub const DIVIDER: egui::Color32 = egui::Color32::from_rgba_premultiplied(0x08, 0x08, 0x08, 0x08);

// Font
pub const FONT_NAME: &str = "JetBrains Mono";
pub const FONT_DIGITAL: &str = "DSEG7";

pub fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    fonts.font_data.insert(
        FONT_NAME.to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../../assets/JetBrainsMono-Regular.ttf"
        ))),
    );

    fonts.font_data.insert(
        FONT_DIGITAL.to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../../assets/DSEG7Classic-Regular.ttf"
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
    fonts
        .families
        .entry(egui::FontFamily::Name(FONT_DIGITAL.into()))
        .or_default()
        .push(FONT_DIGITAL.to_owned());

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
