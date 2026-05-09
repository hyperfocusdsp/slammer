//! Small shared UI primitives used throughout the editor: rack chrome,
//! screws, grooves, LEDs, inset displays, and the `param_knob` helper that
//! wraps `knob::knob` with a `FloatParam` setter.

use nih_plug::prelude::*;
use nih_plug_egui::egui;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;

use crate::midi_map::{CcEncoding, MidiMapState, MidiSource, NoteBlockMap, OMNI};
use crate::ui::knob;
use crate::ui::theme;

/// Shared state stashed in `egui::Context::data` so `param_knob` (and any
/// other widget that wants MIDI Learn) can attach the right-click menu
/// without threading 3 extra args through every call site.
///
/// The editor populates this once per frame from
/// `params.midi_learn` + the editor-local `learn_armed` cell + a frozen
/// `ptr_to_id` table built from `params.param_map()`. The widgets pull it
/// out via `egui::Context::data(...)`.
#[derive(Clone)]
pub struct MidiLearnCtx {
    /// Persisted bindings. Edited via "MIDI Learn" / "Forget" menu items.
    pub state: Arc<Mutex<MidiMapState>>,
    /// `Some(param_id)` while a knob is awaiting its first incoming MIDI
    /// event. Cleared by the editor's drainer after one bind, or by the
    /// user via "Cancel MIDI Learn".
    pub armed: Arc<Mutex<Option<String>>>,
    /// Reverse lookup: raw param-pointer (cast to `usize`) → param id
    /// string from `#[id = "..."]`. Built once at editor start; never
    /// mutated. Lets widgets identify their param without needing every
    /// call site to pass an id string explicitly.
    pub ptr_to_id: Arc<HashMap<usize, String>>,
    /// Audio-thread `(channel, note) → blocked?` lookup. The widgets need
    /// it so "Forget MIDI" can clear the block bit when a note binding
    /// is removed — otherwise the kick would stay suppressed for that pad.
    pub note_block_map: Arc<NoteBlockMap>,
}

const CTX_DATA_KEY: &str = "niner_midi_learn_ctx";

/// Stash the shared `MidiLearnCtx` into `egui::Context::data` so widgets
/// can attach a MIDI Learn menu without needing it threaded through every
/// argument list. Call once per frame from the editor's update closure.
pub fn install_midi_learn_ctx(ctx: &egui::Context, learn: MidiLearnCtx) {
    ctx.data_mut(|d| {
        d.insert_temp::<MidiLearnCtx>(egui::Id::new(CTX_DATA_KEY), learn);
    });
}

fn midi_learn_ctx(ctx: &egui::Context) -> Option<MidiLearnCtx> {
    ctx.data(|d| d.get_temp::<MidiLearnCtx>(egui::Id::new(CTX_DATA_KEY)))
}

/// Public alias of [`attach_midi_learn_menu`] for callers outside this
/// module that have a `&FloatParam` in scope. Used by knobs rendered with
/// the raw [`knob::knob`] helper (e.g. master volume, DJ filter) which
/// can't go through `param_knob`/`param_knob_compact` because they need
/// custom value-to-display mapping (dB conversion, HP/LP labelling).
pub fn attach_midi_learn_menu_for_param(response: &egui::Response, param: &FloatParam) {
    attach_midi_learn_menu(response, param as *const FloatParam as usize);
}

/// Attach a "MIDI Learn / Forget MIDI" right-click menu to a knob's
/// response, anchored at the param identified by `param_ptr` (raw
/// `*const FloatParam as usize`). No-op if the editor hasn't published a
/// MIDI Learn context this frame, or if the pointer isn't in the
/// `ptr_to_id` table (e.g. an internal-only knob).
fn attach_midi_learn_menu(response: &egui::Response, param_ptr: usize) {
    let ctx = response.ctx.clone();
    let Some(learn) = midi_learn_ctx(&ctx) else {
        return;
    };
    let Some(param_id) = learn.ptr_to_id.get(&param_ptr).cloned() else {
        return;
    };
    attach_midi_learn_menu_for_target(response, &learn, &param_id);
}

/// Shared body of the MIDI Learn right-click menu. Used directly by
/// non-`Param` widgets (the BPM display, sequencer play button) that
/// pass a sentinel string from [`crate::midi_map::sentinel`] in place
/// of an actual param id.
pub fn attach_midi_learn_menu_for_target(
    response: &egui::Response,
    learn: &MidiLearnCtx,
    target_id: &str,
) {
    let current_binding = learn.state.lock().binding_for_param(target_id);
    let armed_now = learn
        .armed
        .lock()
        .as_deref()
        .map(|s| s == target_id)
        .unwrap_or(false);

    response.context_menu(|ui| {
        if let Some((ch, source, encoding)) = current_binding {
            let ch_label = if ch == OMNI {
                "OMNI".to_string()
            } else {
                format!("ch {}", ch + 1)
            };
            let enc_suffix = match encoding {
                CcEncoding::Absolute => "",
                CcEncoding::BinaryOffset => " (rel)",
                CcEncoding::Centered => " (rel·c)",
            };
            let src_label = match source {
                MidiSource::Cc(cc) => format!("CC {cc}{enc_suffix}"),
                MidiSource::NoteOn(note) => format!("Note {note}"),
            };
            ui.label(format!("Mapped: {ch_label} · {src_label}"));
            ui.separator();
        }
        if armed_now {
            ui.label("⏳ waiting for MIDI…");
            if ui.button("Cancel MIDI Learn").clicked() {
                *learn.armed.lock() = None;
                ui.close_menu();
            }
        } else if ui.button("MIDI Learn").clicked() {
            *learn.armed.lock() = Some(target_id.to_string());
            ui.close_menu();
        }
        // Manual encoding override for CC bindings, in case the
        // bind-time auto-detect heuristic guessed wrong. The three
        // options match `CcEncoding`. Notes are always absolute and
        // get no picker.
        if let Some((_, MidiSource::Cc(_), encoding)) = current_binding {
            ui.separator();
            let mut e = encoding;
            ui.label("Encoding:");
            if ui
                .radio_value(&mut e, CcEncoding::Absolute, "Absolute (pot/fader)")
                .changed()
            {
                learn.state.lock().set_encoding(target_id, e);
            }
            if ui
                .radio_value(
                    &mut e,
                    CcEncoding::BinaryOffset,
                    "Relative · binary offset (Arturia/BCR)",
                )
                .changed()
            {
                learn.state.lock().set_encoding(target_id, e);
            }
            if ui
                .radio_value(
                    &mut e,
                    CcEncoding::Centered,
                    "Relative · centred (Pioneer/X-Touch)",
                )
                .changed()
            {
                learn.state.lock().set_encoding(target_id, e);
            }
        }
        if current_binding.is_some() && ui.button("Forget MIDI").clicked() {
            // Drop the binding from the persisted state, then mirror
            // that change into the audio-thread NoteBlockMap so the
            // unblocked pad/key triggers the kick again.
            let removed = learn.state.lock().forget(target_id);
            if let Some((ch, MidiSource::NoteOn(note))) = removed {
                learn.note_block_map.unblock(ch, note);
            }
            *learn.armed.lock() = None;
            ui.close_menu();
        }
    });
}

/// Look up the shared `MidiLearnCtx` so widgets that don't go through
/// `param_knob` (the BPM display, sequencer play button) can attach
/// their own context menu using the public helper above.
pub fn fetch_midi_learn_ctx(ctx: &egui::Context) -> Option<MidiLearnCtx> {
    midi_learn_ctx(ctx)
}

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
/// Photorealistic hex-socket bolt, top-down view.
/// `rotation` is the Allen socket angle in radians — vary per screw so none
/// look perfectly aligned (real hardware never is).
pub fn draw_hex_screw(painter: &egui::Painter, cx: f32, cy: f32, radius: f32, rotation: f32) {
    use std::f32::consts::TAU;
    let center = egui::pos2(cx, cy);

    // Background blot: covers the baked chassis circle underneath.
    painter.circle_filled(
        center,
        radius + 1.5,
        egui::Color32::from_rgb(0x10, 0x10, 0x12),
    );

    // Drop shadow below the bolt head.
    painter.circle_filled(
        center + egui::vec2(0.5, 0.8),
        radius + 0.5,
        egui::Color32::from_rgba_premultiplied(0, 0, 0, 60),
    );

    // Outer rim — dark ring (socket body edge).
    painter.circle_filled(center, radius, egui::Color32::from_rgb(0x22, 0x22, 0x24));

    // Bolt cap body — stacked circles to fake a radial gradient.
    let cap_r = radius * 0.84;
    painter.circle_filled(center, cap_r, egui::Color32::from_rgb(0x40, 0x40, 0x44));
    // Subtle upper-left brightening.
    painter.circle_filled(
        center + egui::vec2(-cap_r * 0.15, -cap_r * 0.15),
        cap_r * 0.70,
        egui::Color32::from_rgba_premultiplied(0x65, 0x66, 0x6c, 0x38),
    );
    // Subtle lower-right darkening.
    painter.circle_filled(
        center + egui::vec2(cap_r * 0.08, cap_r * 0.08),
        cap_r * 0.80,
        egui::Color32::from_rgba_premultiplied(0x00, 0x00, 0x00, 0x18),
    );

    // Hex socket — recessed into the cap.
    let socket_r = cap_r * 0.55;
    let hex6 = |r: f32| -> Vec<egui::Pos2> {
        (0..6)
            .map(|i| {
                let a = (i as f32 / 6.0) * TAU + rotation;
                center + egui::vec2(a.cos(), a.sin()) * r
            })
            .collect()
    };
    // Outer socket edge.
    painter.add(egui::Shape::convex_polygon(
        hex6(socket_r),
        egui::Color32::from_rgb(0x20, 0x20, 0x23),
        egui::Stroke::NONE,
    ));
    // Socket floor — darkest.
    painter.add(egui::Shape::convex_polygon(
        hex6(socket_r * 0.76),
        egui::Color32::from_rgb(0x0c, 0x0c, 0x0e),
        egui::Stroke::NONE,
    ));
    // Soft shadow on upper-left inner wall. The chassis surface above
    // the recess blocks direct light, so this hex face sits in shadow.
    // Matches the upper-left light-from-above convention used by the
    // knobs and the cap body (the cap-top brightening above lights the
    // OUTER convex top from upper-left; the recess INTERIOR receives
    // the inverse — shadow on the wall facing the light source).
    let sh_a = rotation + TAU * (4.0 / 6.0);
    let sh_pts = vec![
        center + egui::vec2(sh_a.cos(), sh_a.sin()) * socket_r,
        center + egui::vec2((sh_a + TAU / 6.0).cos(), (sh_a + TAU / 6.0).sin()) * socket_r,
        center + egui::vec2((sh_a + TAU / 6.0).cos(), (sh_a + TAU / 6.0).sin()) * socket_r * 0.76,
        center + egui::vec2(sh_a.cos(), sh_a.sin()) * socket_r * 0.76,
    ];
    painter.add(egui::Shape::convex_polygon(
        sh_pts,
        egui::Color32::from_rgba_premultiplied(0x00, 0x00, 0x00, 0x40),
        egui::Stroke::NONE,
    ));

    // Very subtle specular on cap — matte finish, not shiny.
    let spec = center + egui::vec2(-cap_r * 0.28, -cap_r * 0.28);
    painter.circle_filled(
        spec,
        cap_r * 0.16,
        egui::Color32::from_rgba_premultiplied(0xff, 0xff, 0xff, 0x16),
    );
}

/// Legacy alias kept so the procedural fallback still compiles.
pub fn draw_screw(painter: &egui::Painter, cx: f32, cy: f32, radius: f32) {
    draw_hex_screw(painter, cx, cy, radius, 0.3);
}

/// Set once at editor startup when the baked chassis texture loads
/// successfully. When true, `draw_groove` no-ops because the grooves are
/// part of the bake. False = procedural fallback.
pub static CHASSIS_BAKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Set once at editor startup when the baked screws overlay
/// (`assets/screws.png`) loads successfully and is bigger than the 1×1
/// placeholder. When true, `draw_chrome` paints `screws.png` on top of the
/// clean chassis bake instead of falling through to procedural screw
/// circles. The placeholder check (>1×1) lets us ship a stub PNG so
/// `include_bytes!` compiles before the first real bake.
pub static SCREWS_BAKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Set once at editor startup when the baked knob-cap texture
/// (`assets/knob_cap.png`) loads successfully. When true, `knob::knob_inner`
/// blits the baked neutral cap with `core_color` as a tint instead of the
/// procedural layered-circle core paint. Falls back to procedural when
/// the bake is missing/decoded fails.
pub static KNOB_CAP_BAKED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Pull the lazy-loaded knob-cap texture out of egui's per-context data.
/// `None` until the editor decoded the PNG and stashed it.
pub fn knob_cap_handle(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    ctx.data(|d| d.get_temp::<egui::TextureHandle>(egui::Id::new("niner_knob_cap_handle")))
}

/// Set to `true` once the display reflection PNG has been uploaded as a
/// texture. Same gating idea as `CHASSIS_BAKED`: tells the main-display
/// painter to skip the procedural top-sheen/specular and rely on the
/// baked Cycles overlay instead.
pub static DISPLAY_BAKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Pull the lazy-loaded display-reflection texture handle out of egui's
/// per-context data. Returns `None` until the editor has decoded the PNG
/// and stashed the handle (see `editor.rs`). Callers paint via
/// `paint_display_reflection`.
pub fn display_reflection_handle(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    ctx.data(|d| {
        d.get_temp::<egui::TextureHandle>(egui::Id::new("niner_display_reflection_handle"))
    })
}

/// Paint the baked display-reflection PNG over a `lit` rect. The overlay's
/// luminance is mapped to alpha at bake time so dark regions are
/// transparent and only the highlight bands actually paint over the lit
/// content. Stretches the same texture to fit any display size — works
/// for the master display, the preset bar, and 7-seg LCDs.
pub fn paint_display_reflection(
    painter: &egui::Painter,
    lit: egui::Rect,
    handle: &egui::TextureHandle,
) {
    painter.image(
        handle.id(),
        lit,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        egui::Color32::WHITE,
    );
}

/// Horizontal panel groove — used to separate rows of knobs. Skipped when
/// `CHASSIS_BAKED` is set (the bake includes real beveled groove cuts).
pub fn draw_groove(painter: &egui::Painter, left: f32, right: f32, y: f32) {
    if CHASSIS_BAKED.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
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
/// Asymmetric padding from the bezel rect to the lit content rect.
///
/// The lit area sits inside the bezel with more dark margin on left/top/
/// bottom than right — the dark frame "extends further" around the lit
/// content. The right margin matches the bezel `frame` thickness so the
/// existing 7-seg readout placement (right-aligned to `wf_width`) stays
/// flush with the right bezel edge as it always has.
#[derive(Copy, Clone, Debug)]
pub struct DisplayInsets {
    pub frame: f32,
    pub content_left: f32,
    pub content_top: f32,
    pub content_bottom: f32,
    pub content_right: f32,
}

impl DisplayInsets {
    pub const DEFAULT: Self = Self {
        frame: 4.0,
        content_left: 8.0,
        // Top/bottom tightened from 6 → 4 so an 11-pt DSEG7 readout fits
        // the lit area cleanly inside a CHROME_H (22-px) display, leaving
        // 14 px of usable height. Was 10 px before, which clipped
        // ascenders against the bezel.
        content_top: 4.0,
        content_bottom: 4.0,
        content_right: 4.0,
    };

    /// Compute the lit rect from a bezel-inside rect (`x, y, w, h` — the
    /// area the original `draw_inset_display` painted as `BG_DISPLAY`).
    pub fn lit_rect(&self, x: f32, y: f32, w: f32, h: f32) -> egui::Rect {
        egui::Rect::from_min_size(
            egui::pos2(x + self.content_left, y + self.content_top),
            egui::vec2(
                w - self.content_left - self.content_right,
                h - self.content_top - self.content_bottom,
            ),
        )
    }
}

/// Draw the dark inset display backdrop with default insets. The lit rect
/// (where scan-lines + red glow are painted) is asymmetrically inset from
/// the bezel — see `DisplayInsets::DEFAULT`. Use [`lit_rect_default`] when
/// placing content inside it.
pub fn draw_inset_display(painter: &egui::Painter, x: f32, y: f32, w: f32, h: f32) {
    draw_inset_display_with(painter, x, y, w, h, DisplayInsets::DEFAULT);
}

/// Lit rect for the default insets — convenience for content placement.
pub fn lit_rect_default(x: f32, y: f32, w: f32, h: f32) -> egui::Rect {
    DisplayInsets::DEFAULT.lit_rect(x, y, w, h)
}

/// Draw the dark inset display backdrop with explicit insets.
pub fn draw_inset_display_with(
    painter: &egui::Painter,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    insets: DisplayInsets,
) {
    // Outer bezel frame — skipped when the chassis is baked, since the bake
    // contains a real beveled depression at the same coords.
    if !CHASSIS_BAKED.load(std::sync::atomic::Ordering::Relaxed) {
        painter.rect_filled(
            egui::Rect::from_min_size(
                egui::pos2(x - insets.frame, y - insets.frame),
                egui::vec2(w + insets.frame * 2.0, h + insets.frame * 2.0),
            ),
            4.0,
            theme::BG_DISPLAY_FRAME,
        );
    }
    let lit = insets.lit_rect(x, y, w, h);
    // Display-glass corner radius — keep it tight so 7-segment text inside
    // doesn't read as clipped, but enough that the lit area doesn't feel
    // like a square cut-out.
    let lit_round = 3.0;
    // Inner lit area — uniform dark backdrop. Covers the hammertone
    // texture inside the baked depression so scan-lines and glow have a
    // clean surface to render against.
    painter.rect_filled(lit, lit_round, theme::BG_DISPLAY);
    // Scan-lines, confined to lit rect. Slight 2-px inset on each side so
    // they don't run into the rounded corners visibly.
    let mut sy = lit.top();
    while sy < lit.bottom() {
        painter.line_segment(
            [
                egui::pos2(lit.left() + 2.0, sy),
                egui::pos2(lit.right() - 2.0, sy),
            ],
            egui::Stroke::new(1.0, egui::Color32::from_rgba_premultiplied(0, 0, 0, 20)),
        );
        sy += 2.0;
    }
    // Red ambient glow, confined to lit rect.
    let glow_inset = lit.width() * 0.2;
    painter.rect_filled(
        egui::Rect::from_min_size(
            egui::pos2(lit.left() + glow_inset, lit.top() + lit.height() * 0.2),
            egui::vec2(lit.width() - glow_inset * 2.0, lit.height() * 0.6),
        ),
        0.0,
        theme::RED_AMBIENT,
    );
    draw_display_glass_procedural(painter, lit, lit_round);
}

/// Procedural glass-reflection sheen. 32-step quadratic top gradient + a
/// 1-px specular line along the inside top edge. Used by every display
/// inset that has no baked Cycles overlay (preset bar, 7-seg). The main
/// display in `panels::MasterRow` switches to the baked reflection PNG
/// once `DISPLAY_BAKED` flips true and calls `draw_inset_display_no_glass`
/// instead of the wrapper that includes this.
pub fn draw_display_glass_procedural(painter: &egui::Painter, lit: egui::Rect, lit_round: f32) {
    let sheen_h = lit.height() * 0.35;
    const SHEEN_STEPS: u32 = 32;
    for i in 0..SHEEN_STEPS {
        let t = i as f32 / SHEEN_STEPS as f32;
        let falloff = (1.0 - t) * (1.0 - t);
        let alpha = (falloff * 24.0) as u8;
        let h = (sheen_h * (1.0 - t)).max(1.0);
        painter.rect_filled(
            egui::Rect::from_min_size(lit.min, egui::vec2(lit.width(), h)),
            lit_round,
            egui::Color32::from_rgba_premultiplied(alpha, alpha, alpha, alpha),
        );
    }
    painter.line_segment(
        [
            egui::pos2(lit.left() + 4.0, lit.top() + 1.5),
            egui::pos2(lit.right() - 4.0, lit.top() + 1.5),
        ],
        egui::Stroke::new(0.5, theme::DISPLAY_SPECULAR),
    );
}

/// Same as `draw_inset_display_with` but skips the procedural glass overlay.
/// Used by the main display, which paints a Cycles-baked reflection PNG
/// over the lit content after the bars/waveform are drawn.
pub fn draw_inset_display_no_glass(
    painter: &egui::Painter,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    insets: DisplayInsets,
) {
    if !CHASSIS_BAKED.load(std::sync::atomic::Ordering::Relaxed) {
        painter.rect_filled(
            egui::Rect::from_min_size(
                egui::pos2(x - insets.frame, y - insets.frame),
                egui::vec2(w + insets.frame * 2.0, h + insets.frame * 2.0),
            ),
            4.0,
            theme::BG_DISPLAY_FRAME,
        );
    }
    let lit = insets.lit_rect(x, y, w, h);
    let lit_round = 3.0;
    painter.rect_filled(lit, lit_round, theme::BG_DISPLAY);
    let mut sy = lit.top();
    while sy < lit.bottom() {
        painter.line_segment(
            [
                egui::pos2(lit.left() + 2.0, sy),
                egui::pos2(lit.right() - 2.0, sy),
            ],
            egui::Stroke::new(1.0, egui::Color32::from_rgba_premultiplied(0, 0, 0, 20)),
        );
        sy += 2.0;
    }
    let glow_inset = lit.width() * 0.2;
    painter.rect_filled(
        egui::Rect::from_min_size(
            egui::pos2(lit.left() + glow_inset, lit.top() + lit.height() * 0.2),
            egui::vec2(lit.width() - glow_inset * 2.0, lit.height() * 0.6),
        ),
        0.0,
        theme::RED_AMBIENT,
    );
}

/// Small arrow button used in the preset bar (`◂` / `▸`). `press_amount`
/// is 0.0..1.0 so the caller can animate the press via
/// `ctx.animate_bool_with_time`.
pub fn preset_arrow_btn(
    painter: &egui::Painter,
    rect: egui::Rect,
    glyph: &str,
    color: egui::Color32,
    press_amount: f32,
    rounding: f32,
) {
    draw_button_3d(painter, rect, press_amount, rounding);
    let text_offset = press_amount.clamp(0.0, 1.0) * BTN_PRESS_TRAVEL;
    painter.text(
        rect.center() + egui::vec2(0.0, text_offset),
        egui::Align2::CENTER_CENTER,
        glyph,
        egui::FontId::new(12.0, egui::FontFamily::Monospace),
        color,
    );
}

/// Maximum cap travel when fully pressed, in pixels. Animation rides this
/// value so a click reads as the cap dropping ~1.5 px into its well.
pub const BTN_PRESS_TRAVEL: f32 = 1.5;

/// Roland-TR-909-style tactile button. Paints, bottom-up:
///
/// 1. Recessed well — a darker rectangular "socket" 1 px outside `rect` so
///    a visible chassis gap surrounds the cap on all four sides. The well's
///    top edge gets an extra-dark line for the depth illusion (real caps
///    overhang their cutouts and cast a hard shadow into them).
/// 2. Drop shadow under the cap — alpha lerped to 0 as press_amount → 1, so
///    it fades out smoothly while the cap sinks.
/// 3. Cap body, translated downward by `press_amount * BTN_PRESS_TRAVEL`.
///    Multi-stop vertical gradient (light spec at top → midtone → deep
///    self-shadow at base) via stacked rounded `rect_filled`. Pressed-state
///    flip is interpolated by mixing each stop with its inverted partner.
/// 4. Edge stroke at the rim.
/// 5. Top specular ribbon — alpha tracks `1 - press_amount`.
/// 6. Bottom shadow ledge — concave-cap profile.
///
/// `press_amount` is 0.0 (released) … 1.0 (fully pressed). For a smooth
/// click animation, callers should pass `ctx.animate_bool_with_time(id,
/// pressed, 0.06)` rather than a raw bool — the cap then settles into the
/// well over ~60 ms instead of jumping. Callers should also offset the
/// text label by `+ press_amount * BTN_PRESS_TRAVEL` Y so the glyph rides
/// the cap.
pub fn draw_button_3d(painter: &egui::Painter, rect: egui::Rect, press_amount: f32, rounding: f32) {
    let press = press_amount.clamp(0.0, 1.0);

    // 1) Recessed well — chassis cutout the cap sits in. Painted before
    //    everything else so a 1.5-px gap shows around the cap on all sides
    //    (TR-909 caps overhang their cutouts ~0.5–1 mm; at this canvas DPI
    //    that's ~1.5 px). Outer ring is the deep cutout colour; an inset
    //    secondary fill lifts the inner edge slightly so the cap isn't
    //    floating on a flat black background.
    let well_outer = rect.expand(1.5);
    painter.rect_filled(well_outer, rounding + 1.5, theme::BTN_WELL);
    // Hard top-edge shadow inside the well — sells the overhanging cap
    // illusion. Two stacked half-pixel lines for emphasis.
    painter.line_segment(
        [
            egui::pos2(well_outer.left() + 1.0, well_outer.top() + 0.5),
            egui::pos2(well_outer.right() - 1.0, well_outer.top() + 0.5),
        ],
        egui::Stroke::new(0.6, theme::BTN_WELL_TOP_SHADOW),
    );
    // Left-edge shadow inside the well — light comes from upper-left so
    // the left wall of the cutout is slightly more in shadow than the
    // right (matches the chassis hammertone bake's lighting direction).
    painter.line_segment(
        [
            egui::pos2(well_outer.left() + 0.5, well_outer.top() + 1.0),
            egui::pos2(well_outer.left() + 0.5, well_outer.bottom() - 1.0),
        ],
        egui::Stroke::new(0.5, theme::BTN_WELL_TOP_SHADOW),
    );

    // 2) Drop shadow under the cap — fades out as the cap sinks.
    let shadow_alpha = ((1.0 - press) * 0x70 as f32) as u8;
    if shadow_alpha > 0 {
        let shadow_rect = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 0.5, rect.bottom() - 0.5),
            egui::pos2(rect.right() + 1.5, rect.bottom() + 2.0),
        );
        painter.rect_filled(
            shadow_rect,
            rounding,
            egui::Color32::from_rgba_premultiplied(0, 0, 0, shadow_alpha),
        );
    }

    // 3) Cap body — translated by press_amount * travel so the cap visibly
    //    sinks. Gradient stops are blended between idle and pressed
    //    arrangements by `press` so mid-animation frames don't snap.
    let cap_rect = rect.translate(egui::vec2(0.0, press * BTN_PRESS_TRAVEL));
    let idle_stops: [egui::Color32; 5] = [
        theme::BTN_BOTTOM_DEEP,
        theme::BTN_BOTTOM,
        theme::BTN_MID,
        theme::BTN_TOP,
        theme::BTN_HIGHLIGHT_TOP,
    ];
    let pressed_stops: [egui::Color32; 5] = [
        theme::BTN_HIGHLIGHT_TOP,
        theme::BTN_TOP,
        theme::BTN_MID,
        theme::BTN_BOTTOM,
        theme::BTN_BOTTOM_DEEP,
    ];
    let stop_positions: [f32; 5] = [0.00, 0.25, 0.55, 0.85, 1.00];
    let blended: [egui::Color32; 5] =
        std::array::from_fn(|i| lerp_color32(idle_stops[i], pressed_stops[i], press));

    let layers = 12;
    for i in 0..=layers {
        let t = i as f32 / layers as f32;
        let mut col = blended[blended.len() - 1];
        for w in 0..stop_positions.len() - 1 {
            let (t0, c0) = (stop_positions[w], blended[w]);
            let (t1, c1) = (stop_positions[w + 1], blended[w + 1]);
            if t >= t0 && t <= t1 {
                let span = (t1 - t0).max(1e-6);
                let u = ((t - t0) / span).clamp(0.0, 1.0);
                col = lerp_color32(c0, c1, u);
                break;
            }
        }
        let h = cap_rect.height() * (1.0 - t);
        if h < 0.5 {
            continue;
        }
        painter.rect_filled(
            egui::Rect::from_min_size(cap_rect.min, egui::vec2(cap_rect.width(), h)),
            rounding,
            col,
        );
    }

    // 4) Edge stroke.
    painter.rect_stroke(
        cap_rect,
        rounding,
        egui::Stroke::new(0.5, theme::BTN_EDGE),
        egui::StrokeKind::Inside,
    );

    // 5) Top specular ribbon — alpha fades as cap depresses.
    let sheen_alpha = ((1.0 - press) * 0x44 as f32) as u8;
    if sheen_alpha > 0 && cap_rect.height() >= 8.0 {
        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(cap_rect.left() + 1.5, cap_rect.top() + 1.0),
                egui::pos2(cap_rect.right() - 1.5, cap_rect.top() + 2.0),
            ),
            rounding * 0.5,
            egui::Color32::from_rgba_premultiplied(0xff, 0xff, 0xff, sheen_alpha),
        );
    }

    // 6) Bottom shadow ledge — concave-cap profile.
    if cap_rect.height() >= 8.0 {
        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(cap_rect.left() + 1.5, cap_rect.bottom() - 2.0),
                egui::pos2(cap_rect.right() - 1.5, cap_rect.bottom() - 1.0),
            ),
            rounding * 0.5,
            theme::BTN_BOT_LEDGE,
        );
    }
}

fn lerp_color32(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let lerp_u8 = |x: u8, y: u8| -> u8 {
        let v = x as f32 + (y as f32 - x as f32) * t;
        v.clamp(0.0, 255.0) as u8
    };
    egui::Color32::from_rgba_premultiplied(
        lerp_u8(a.r(), b.r()),
        lerp_u8(a.g(), b.g()),
        lerp_u8(a.b(), b.b()),
        lerp_u8(a.a(), b.a()),
    )
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
    // Read the user-set target, NOT the smoothed current value: smoothed
    // params (e.g. master decay's Linear(20ms)) are mid-ramp during the
    // same frame the user is dragging, so feeding `param.value()` back
    // into the knob makes the indicator lurch toward the smoother state
    // instead of staying where the user dragged it. `unmodulated_plain_value()`
    // is what the user actually dialled in.
    let mut val = param.unmodulated_plain_value();
    let knob_resp = knob::knob(
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
    );
    if knob_resp.changed {
        setter.begin_set_parameter(param);
        setter.set_parameter(param, val);
        setter.end_set_parameter(param);
    }
    if let Some(resp) = knob_resp.response.as_ref() {
        let ptr = param as *const FloatParam as usize;
        attach_midi_learn_menu(resp, ptr);
    }
    knob_resp.changed
}

/// Compact-layout variant of `param_knob` for dense clusters (e.g. the
/// stacked sub-rows in the v0.6.0 SAT/CLIP cluster). Identical
/// param-binding behaviour; visually the knob renders with tighter
/// surrounding padding and the label sits flush against the knob box.
///
/// `label` is the abbreviation rendered under the knob (≤ 4 chars
/// recommended so it fits on one line at 9.5 pt mono in the compact
/// column). `tooltip` is the long-form description shown on hover.
#[allow(clippy::too_many_arguments)]
pub fn param_knob_compact(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    id: &str,
    label: &str,
    tooltip: &str,
    param: &FloatParam,
    min: f32,
    max: f32,
    default: f32,
    format_value: impl Fn(f32) -> String,
    diameter: f32,
    core_color: egui::Color32,
) -> bool {
    // See `param_knob` above for why this isn't `param.value()`.
    let mut val = param.unmodulated_plain_value();
    let knob_resp = knob::knob_compact(
        ui,
        egui::Id::new(id),
        &mut val,
        min,
        max,
        default,
        label,
        tooltip,
        format_value,
        diameter,
        core_color,
    );
    if knob_resp.changed {
        setter.begin_set_parameter(param);
        setter.set_parameter(param, val);
        setter.end_set_parameter(param);
    }
    if let Some(resp) = knob_resp.response.as_ref() {
        let ptr = param as *const FloatParam as usize;
        attach_midi_learn_menu(resp, ptr);
    }
    knob_resp.changed
}
