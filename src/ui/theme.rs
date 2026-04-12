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
pub const WHITE: egui::Color32 = egui::Color32::from_rgb(0xdd, 0xdd, 0xdd);
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
pub const KNOB_INDICATOR: egui::Color32 = egui::Color32::from_rgb(0xee, 0xee, 0xee);
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

// Arrow buttons
pub const BTN_LIGHT: egui::Color32 = egui::Color32::from_rgb(0x44, 0x44, 0x44);
pub const BTN_DARK: egui::Color32 = egui::Color32::from_rgb(0x1c, 0x1c, 0x1c);
pub const BTN_TEXT: egui::Color32 = egui::Color32::from_rgba_premultiplied(0x50, 0x50, 0x50, 0x73);

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
