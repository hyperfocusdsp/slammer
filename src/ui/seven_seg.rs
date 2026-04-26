//! Polygon-based 7-segment display renderer and the LCD-style mode selector
//! used by the saturation mode switch.
//!
//! Standard segment labels:
//! ```text
//!  aaa
//! f   b
//! f   b
//!  ggg
//! e   c
//! e   c
//!  ddd
//! ```
//! Segment bitmask: bit 0=a, 1=b, 2=c, 3=d, 4=e, 5=f, 6=g. A value of 1 means
//! the segment is on.

use nih_plug::prelude::*;
use nih_plug_egui::egui;

use crate::ui::theme;
use crate::ui::widgets::draw_inset_display;

/// Return the bitmask describing which segments should light up for `ch`.
fn seg_mask(ch: char) -> u8 {
    match ch {
        '0' => 0b0111111,
        '1' => 0b0000110,
        '2' => 0b1011011,
        '3' => 0b1001111,
        '4' => 0b1100110,
        '5' => 0b1101101,
        '6' => 0b1111101,
        '7' => 0b0000111,
        '8' => 0b1111111,
        '9' => 0b1101111,
        'A' => 0b1110111,
        'b' => 0b1111100,
        'C' => 0b0111001,
        'd' => 0b1011110,
        'E' => 0b1111001,
        'F' => 0b1110001,
        'H' => 0b1110110,
        'I' => 0b0110000,
        'L' => 0b0111000,
        'O' => 0b0111111,
        'P' => 0b1110011,
        'S' => 0b1101101,
        't' => 0b1111000,
        'U' => 0b0111110,
        'i' => 0b0010000,
        'n' => 0b1010100,
        'o' => 0b1011100,
        'f' => 0b1110001,
        ' ' => 0b0000000,
        '-' => 0b1000000,
        '°' => 0b1100011,
        '%' => 0b1100011,
        '+' => 0b1110000,
        _ => 0b0000000,
    }
}

/// Map arbitrary input text characters to 7-seg-compatible forms.
/// Uppercase D/T → lowercase d/t to get correct renderable glyphs.
fn map_char_to_7seg(ch: char) -> char {
    match ch {
        'D' => 'd',
        'T' => 't',
        _ => ch,
    }
}

/// Draw a single 7-segment digit/character at `origin` with cell size `w x h`.
fn draw_7seg_char(painter: &egui::Painter, origin: egui::Pos2, w: f32, h: f32, mask: u8) {
    let on_color = theme::RED_LED;
    let off_color = egui::Color32::from_rgba_premultiplied(0x12, 0x02, 0x02, 0x12);

    let t = (w * 0.18).max(1.5);
    let gap = t * 0.15;

    let x0 = origin.x;
    let y0 = origin.y;
    let x1 = origin.x + w;
    let mid_y = origin.y + h * 0.5;
    let y1 = origin.y + h;

    let horiz_seg = |lx: f32, rx: f32, cy: f32| -> Vec<egui::Pos2> {
        let half_t = t * 0.5;
        vec![
            egui::pos2(lx + half_t + gap, cy),
            egui::pos2(lx + t + gap, cy - half_t),
            egui::pos2(rx - t - gap, cy - half_t),
            egui::pos2(rx - half_t - gap, cy),
            egui::pos2(rx - t - gap, cy + half_t),
            egui::pos2(lx + t + gap, cy + half_t),
        ]
    };

    let vert_seg = |cx: f32, ty: f32, by: f32| -> Vec<egui::Pos2> {
        let half_t = t * 0.5;
        vec![
            egui::pos2(cx, ty + half_t + gap),
            egui::pos2(cx + half_t, ty + t + gap),
            egui::pos2(cx + half_t, by - t - gap),
            egui::pos2(cx, by - half_t - gap),
            egui::pos2(cx - half_t, by - t - gap),
            egui::pos2(cx - half_t, ty + t + gap),
        ]
    };

    let segments: [(u8, Vec<egui::Pos2>); 7] = [
        (0, horiz_seg(x0, x1, y0 + t * 0.5)),
        (1, vert_seg(x1 - t * 0.5, y0, mid_y)),
        (2, vert_seg(x1 - t * 0.5, mid_y, y1)),
        (3, horiz_seg(x0, x1, y1 - t * 0.5)),
        (4, vert_seg(x0 + t * 0.5, mid_y, y1)),
        (5, vert_seg(x0 + t * 0.5, y0, mid_y)),
        (6, horiz_seg(x0, x1, mid_y)),
    ];

    for (bit, points) in segments {
        let color = if (mask >> bit) & 1 == 1 {
            on_color
        } else {
            off_color
        };
        painter.add(egui::Shape::convex_polygon(
            points,
            color,
            egui::Stroke::NONE,
        ));
    }
}

/// Draw an entire 7-seg text string centered inside `rect`.
pub fn draw_7seg_text(painter: &egui::Painter, rect: egui::Rect, text: &str) {
    let chars: Vec<char> = text.chars().map(map_char_to_7seg).collect();
    let n = chars.len();
    if n == 0 {
        return;
    }

    let cell_h = rect.height() * 0.85;
    let cell_w = cell_h * 0.55;
    let char_spacing = cell_w * 1.1;
    let total_width = char_spacing * (n as f32 - 1.0) + cell_w;

    let start_x = rect.center().x - total_width * 0.5;
    let start_y = rect.center().y - cell_h * 0.5;

    for (i, &ch) in chars.iter().enumerate() {
        let mask = seg_mask(ch);
        let origin = egui::pos2(start_x + i as f32 * char_spacing, start_y);
        draw_7seg_char(painter, origin, cell_w, cell_h, mask);
    }
}

/// LCD-style `[<]  LABEL  [>]` mode selector wired to a `FloatParam` that
/// holds an integer-valued mode index.
///
/// `id_source` salts the egui Ids of the two arrow buttons so multiple
/// selectors can coexist in the same frame without colliding. Pass a
/// distinct string per selector (e.g. `"sat_mode"`, `"clip_mode"`).
///
/// `modes` is the per-index display string slice — typically the same
/// length as the param's discrete-value range. Strings should only use
/// glyphs covered by `seg_mask` (numerals + a small letter set).
///
/// `compact = false` produces the full-height variant (18×26 arrows,
/// 56×22 LCD) used historically by SAT MODE. `compact = true` shrinks
/// to (14×18 arrows, 44×16 LCD) and drops the trailing 8 px gap, so two
/// compact selectors fit cleanly stacked inside one main-knob row.
pub fn lcd_selector(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    param: &FloatParam,
    id_source: &str,
    modes: &[&str],
    compact: bool,
) {
    let current = param.value() as usize;

    let (btn_w, btn_h, lcd_width, lcd_height, trailing_pad) = if compact {
        (14.0, 18.0, 44.0, 16.0, 0.0)
    } else {
        (18.0, 26.0, 56.0, 22.0, 8.0)
    };

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;

        let (_, left_rect) = ui.allocate_space(egui::vec2(btn_w, btn_h));
        if ui.is_rect_visible(left_rect) {
            draw_lcd_arrow(ui.painter(), left_rect, "\u{25C0}");
        }
        let left_resp = ui.interact(
            left_rect,
            egui::Id::new((id_source, "left")),
            egui::Sense::click(),
        );
        if left_resp.clicked() && !modes.is_empty() {
            let next = if current == 0 {
                modes.len() - 1
            } else {
                current - 1
            };
            setter.begin_set_parameter(param);
            setter.set_parameter(param, next as f32);
            setter.end_set_parameter(param);
        }

        let (_, lcd_rect) = ui.allocate_space(egui::vec2(lcd_width, lcd_height));
        if ui.is_rect_visible(lcd_rect) {
            let painter = ui.painter();
            draw_inset_display(
                painter,
                lcd_rect.left(),
                lcd_rect.top(),
                lcd_width,
                lcd_height,
            );
            let mode_name = modes.get(current).copied().unwrap_or("");
            draw_7seg_text(painter, lcd_rect, mode_name);
        }

        let (_, right_rect) = ui.allocate_space(egui::vec2(btn_w, btn_h));
        if ui.is_rect_visible(right_rect) {
            draw_lcd_arrow(ui.painter(), right_rect, "\u{25B6}");
        }
        let right_resp = ui.interact(
            right_rect,
            egui::Id::new((id_source, "right")),
            egui::Sense::click(),
        );
        if right_resp.clicked() && !modes.is_empty() {
            let next = if current + 1 >= modes.len() { 0 } else { current + 1 };
            setter.begin_set_parameter(param);
            setter.set_parameter(param, next as f32);
            setter.end_set_parameter(param);
        }

        if trailing_pad > 0.0 {
            ui.add_space(trailing_pad);
        }
    });
}

fn draw_lcd_arrow(painter: &egui::Painter, rect: egui::Rect, glyph: &str) {
    painter.rect_filled(rect, 3.0, theme::BTN_DARK);
    painter.rect_filled(
        egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), rect.height() * 0.4)),
        3.0,
        theme::BTN_LIGHT,
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        glyph,
        egui::FontId::new(9.0, egui::FontFamily::Monospace),
        theme::BTN_TEXT,
    );
}
