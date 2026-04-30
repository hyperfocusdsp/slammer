//! Layout overrides — bulk + per-element offsets applied to the chrome.
//!
//! ## Two compile modes
//!
//! * **Default (release)** — the editor UI is excluded. Layouts come from
//!   `assets/baked_layout.json` via `include_bytes!`, parsed once at
//!   editor construction. `instrument()` / `instrument_text()` apply the
//!   offsets but never paint drag handles, never register an interact
//!   surface. Production binaries pay only one JSON parse + an `Arc<Data>`
//!   read per element per frame.
//!
//! * **`--features layout_editor`** — the full hand-tuning panel compiles
//!   in. Activation is then gated three more ways:
//!   (1) env var `NINER_LAYOUT_EDITOR=1` flips `LAYOUT_EDITOR_ON` at startup,
//!   (2) Alt+L toggles it at runtime, and
//!   (3) without either, the atomic stays false and `instrument()` matches
//!   the default-build behavior — pixel-identical.
//!   "Save layout" writes to `<niner-data>/layout_overrides.json`, which
//!   is overlaid on top of the baked JSON at the next startup. Run
//!   `cargo xtask lock-layout` to bake the saved JSON into the asset.
//!
//! ## Snapping (editor-only)
//!
//! Drag motion is snapped to a configurable pixel grid (default 2 px).
//! Holding Shift during a drag bypasses the grid for sub-pixel nudges.
//! Holding Ctrl multiplies the grid ×4 for coarser placement. Right-click
//! an element to reset its offset to zero.

use nih_plug_egui::egui;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
#[cfg(feature = "layout_editor")]
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(feature = "layout_editor")]
use crate::util::paths::niner_data_dir;

/// Layouts shipped in the release binary. Captured at build time from
/// `assets/baked_layout.json` (mirror of the dev-tuned
/// `<niner-data>/layout_overrides.json`). Run `cargo xtask lock-layout`
/// to refresh.
const BAKED_LAYOUT_JSON: &[u8] = include_bytes!("../../assets/baked_layout.json");

/// Master switch — only present when the editor is compiled in. Initialized
/// from `NINER_LAYOUT_EDITOR` at startup; toggled by Alt+L thereafter.
#[cfg(feature = "layout_editor")]
pub static LAYOUT_EDITOR_ON: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "layout_editor")]
#[inline]
fn is_editor_on() -> bool {
    LAYOUT_EDITOR_ON.load(Ordering::Relaxed)
}

#[cfg(not(feature = "layout_editor"))]
#[inline]
fn is_editor_on() -> bool {
    false
}

/// Bulk multipliers/offsets that apply uniformly to a class of widgets.
/// All defaults are identity (1.0 / 0.0) except snap, which defaults
/// on at a 2-px grid.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BulkOverrides {
    /// Multiplies CHROME_H / CHROME_SQ at runtime — uniform scaler for
    /// every chrome cap height.
    pub chrome_height_scale: f32,
    /// Adds to the corner radius of chrome buttons (pre-clamped to >=0).
    pub chrome_rounding_delta: f32,
    /// Multiplies every label font size (row labels SUB/TOP/MID/SAT/EQ,
    /// knob labels, etc).
    pub label_font_scale: f32,
    /// Adds horizontal padding (px) around row labels.
    pub label_padding_delta: f32,
    /// When true, drag motion snaps to `snap_grid` pixels. Hold Shift
    /// during a drag to bypass; hold Ctrl to multiply by ×4.
    #[serde(default = "default_true")]
    pub snap_enabled: bool,
    /// Snap grid in pixels. 1.0 ≈ off (every pixel), 2.0 default,
    /// 4 / 8 for coarse placement.
    #[serde(default = "default_snap_grid")]
    pub snap_grid: f32,
}

fn default_true() -> bool {
    true
}
fn default_snap_grid() -> f32 {
    2.0
}

impl Default for BulkOverrides {
    fn default() -> Self {
        Self {
            chrome_height_scale: 1.0,
            chrome_rounding_delta: 0.0,
            label_font_scale: 1.0,
            label_padding_delta: 0.0,
            snap_enabled: true,
            snap_grid: 2.0,
        }
    }
}

/// Per-element override. `pos_offset` is added to the element's anchor;
/// `size_scale` and `font_scale` multiply.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct OverrideEntry {
    pub pos_offset_x: f32,
    pub pos_offset_y: f32,
    pub size_scale: f32,
    pub font_scale: f32,
}

impl Default for OverrideEntry {
    fn default() -> Self {
        Self {
            pos_offset_x: 0.0,
            pos_offset_y: 0.0,
            size_scale: 1.0,
            font_scale: 1.0,
        }
    }
}

/// Full snapshot — bulk + per-element entries. Persisted as JSON.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LayoutOverrides {
    pub bulk: BulkOverrides,
    #[serde(default)]
    pub entries: HashMap<String, OverrideEntry>,
}

const DATA_KEY: &str = "niner_layout_overrides";

#[cfg(feature = "layout_editor")]
const REGISTRY_KEY: &str = "niner_layout_registry";
#[cfg(feature = "layout_editor")]
const SELECTION_KEY: &str = "niner_layout_selection";

/// Per-frame keyset of every instrumented element. Cleared at the start
/// of each frame (lazy — when the editor panel renders) so the panel's
/// "registered elements" list reflects what actually painted this frame.
#[cfg(feature = "layout_editor")]
#[derive(Clone, Debug, Default)]
struct ElementRegistry {
    /// Frame index at which this registry was last reset. Stale entries
    /// are dropped on the first call of a new frame.
    frame: u64,
    keys: Vec<String>,
    /// Last known visible rect per key, for the registry list's "find" /
    /// reset-to-origin button.
    rects: HashMap<String, egui::Rect>,
}

#[cfg(feature = "layout_editor")]
#[derive(Clone, Debug, Default)]
struct Selection {
    keys: Vec<String>,
}

#[cfg(feature = "layout_editor")]
fn get_selection(ctx: &egui::Context) -> Selection {
    ctx.data(|d| {
        d.get_temp::<Selection>(egui::Id::new(SELECTION_KEY))
            .unwrap_or_default()
    })
}

#[cfg(feature = "layout_editor")]
fn set_selection(ctx: &egui::Context, sel: Selection) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(SELECTION_KEY), sel));
}

#[cfg(feature = "layout_editor")]
fn is_selected(ctx: &egui::Context, key: &str) -> bool {
    get_selection(ctx).keys.iter().any(|k| k == key)
}

#[cfg(feature = "layout_editor")]
fn store_path() -> std::path::PathBuf {
    niner_data_dir().join("layout_overrides.json")
}

/// Seed the egui temp data store with the effective layout. Always parses
/// the baked JSON (compiled into the binary via `include_bytes!`); when
/// the editor feature is enabled and a `<niner-data>/layout_overrides.json`
/// file exists, that file overrides the baked baseline so iterative
/// tweaks during a tuning session don't require a rebuild.
pub fn init(ctx: &egui::Context) {
    let baked = parse_baked();
    #[cfg(feature = "layout_editor")]
    let effective = load_from_disk().unwrap_or(baked);
    #[cfg(not(feature = "layout_editor"))]
    let effective = baked;

    ctx.data_mut(|d| d.insert_temp(egui::Id::new(DATA_KEY), effective));

    #[cfg(feature = "layout_editor")]
    {
        let on = std::env::var("NINER_LAYOUT_EDITOR")
            .map(|v| v != "0" && !v.is_empty())
            .unwrap_or(false);
        LAYOUT_EDITOR_ON.store(on, Ordering::Relaxed);
        if on {
            nih_plug::nih_log!(
                "[layout_editor] active — Alt+L toggles, save writes to {}",
                store_path().display()
            );
        }
    }
}

fn parse_baked() -> LayoutOverrides {
    serde_json::from_slice::<LayoutOverrides>(BAKED_LAYOUT_JSON).unwrap_or_default()
}

/// Pull the current snapshot out of egui's per-context data store.
/// Always reads from data — the saved JSON is preloaded by `init()` so
/// the offsets become the plugin's layout regardless of whether the
/// editor UI is currently visible. Production builds where the user
/// never saved a layout get `Default::default()` (identity).
pub fn snapshot(ctx: &egui::Context) -> LayoutOverrides {
    ctx.data(|d| {
        d.get_temp::<LayoutOverrides>(egui::Id::new(DATA_KEY))
            .unwrap_or_default()
    })
}

/// Bulk overrides only — common case for chrome-height / font-scale lookups.
pub fn bulk(ctx: &egui::Context) -> BulkOverrides {
    snapshot(ctx).bulk
}

/// Effective chrome cap height: `CHROME_H * bulk.chrome_height_scale`.
/// Use everywhere a button height is computed so the bulk slider sweeps
/// the whole chrome family in one move.
pub fn chrome_height(ctx: &egui::Context) -> f32 {
    crate::ui::panels::CHROME_H * bulk(ctx).chrome_height_scale
}

/// Effective square-cap dimension. Mirrors `chrome_height` for square
/// buttons (preset arrows, SAT-row LCD arrows).
pub fn chrome_sq(ctx: &egui::Context) -> f32 {
    crate::ui::panels::CHROME_SQ * bulk(ctx).chrome_height_scale
}

/// Multiplier for label font sizes — apply to row labels, knob labels,
/// chrome glyphs.
pub fn label_font_scale(ctx: &egui::Context) -> f32 {
    bulk(ctx).label_font_scale
}

/// Effective corner-rounding for chrome buttons: `(base + delta).max(0.0)`.
/// Wrap any call to `draw_button_3d`'s `rounding` arg so the bulk slider
/// affects every cap uniformly.
pub fn chrome_rounding(ctx: &egui::Context, base: f32) -> f32 {
    (base + bulk(ctx).chrome_rounding_delta).max(0.0)
}

/// Construct a monospace `FontId` scaled by `label_font_scale`. Wrap any
/// `egui::FontId::new(size, FontFamily::Monospace)` for row/knob/chrome
/// labels with this helper so the bulk slider sweeps every label.
pub fn label_font(ctx: &egui::Context, base_size: f32) -> egui::FontId {
    egui::FontId::new(
        (base_size * label_font_scale(ctx)).max(1.0),
        egui::FontFamily::Monospace,
    )
}

/// The per-frame snap step. Tracks the user's `snap_grid` setting only
/// — modifier keys do NOT change this. Earlier versions overrode the
/// grid with Shift/Ctrl, but that meant pressing a modifier (e.g. for
/// multi-select) re-snapped every offset to a different grid and
/// every element jumped by ±1–2 px every time you held Shift. Modifiers
/// now affect ONLY click-time semantics (Shift+click = multi-select)
/// and arrow-key step (Shift+arrow = 10 px).
fn effective_grid(ctx: &egui::Context) -> f32 {
    let b = bulk(ctx);
    if !b.snap_enabled {
        return 1.0;
    }
    b.snap_grid.max(1.0)
}

fn snap(value: f32, grid: f32) -> f32 {
    if grid <= 1.0 {
        value.round()
    } else {
        (value / grid).round() * grid
    }
}

#[cfg(feature = "layout_editor")]
fn frame_index(ctx: &egui::Context) -> u64 {
    // egui doesn't expose a monotonic frame counter via stable API, so we
    // bump our own each time `register` runs from a new repaint cycle.
    // Caller-side: any `register` call refreshes the frame on the first
    // hit per repaint by comparing input.time keyed on the frame's
    // wall-clock dt. Simpler proxy: bucket on `i.time` quantised to ms.
    ctx.input(|i| (i.time * 1000.0) as u64)
}

#[cfg(feature = "layout_editor")]
fn registry_refresh(ctx: &egui::Context) -> ElementRegistry {
    let frame = frame_index(ctx);
    ctx.data_mut(|d| {
        let id = egui::Id::new(REGISTRY_KEY);
        let mut reg: ElementRegistry = d.get_temp(id).unwrap_or_default();
        if reg.frame != frame {
            reg.frame = frame;
            reg.keys.clear();
            // Don't clear rects: they're stable across frames and the
            // registry list reads them between drag events. Stale rects
            // get overwritten when their owning element paints again.
        }
        d.insert_temp(id, reg.clone());
        reg
    })
}

#[cfg(feature = "layout_editor")]
fn registry_record(ctx: &egui::Context, key: &str, rect: egui::Rect) {
    let _ = registry_refresh(ctx);
    ctx.data_mut(|d| {
        let id = egui::Id::new(REGISTRY_KEY);
        let mut reg: ElementRegistry = d.get_temp(id).unwrap_or_default();
        if !reg.keys.iter().any(|k| k == key) {
            reg.keys.push(key.to_string());
        }
        reg.rects.insert(key.to_string(), rect);
        d.insert_temp(id, reg);
    });
}

#[cfg(feature = "layout_editor")]
fn registry_keys(ctx: &egui::Context) -> Vec<String> {
    ctx.data(|d| {
        d.get_temp::<ElementRegistry>(egui::Id::new(REGISTRY_KEY))
            .map(|r| r.keys.clone())
            .unwrap_or_default()
    })
}

#[cfg(feature = "layout_editor")]
fn registry_rect(ctx: &egui::Context, key: &str) -> Option<egui::Rect> {
    ctx.data(|d| {
        d.get_temp::<ElementRegistry>(egui::Id::new(REGISTRY_KEY))
            .and_then(|r| r.rects.get(key).copied())
    })
}

/// Wrap a chrome element's base rect with the editor's drag instrumentation.
/// When `LAYOUT_EDITOR_ON` is false this is a pure pass-through and returns
/// `base_rect` unchanged — production builds pay zero overhead.
///
/// When the editor is on:
///   - The recorded raw `pos_offset_x/y` for this `key` is applied to
///     the rect, snapped to the active grid for the visual.
///   - A `Sense::click_and_drag()` interaction is registered. Click
///     selects (Shift toggles), drag of a selected element drags the
///     whole selection, drag of an unselected element replaces selection
///     with this one and drags it. Right-click resets offsets.
///   - On hover, a 1-px outline + corner handles paint on top.
///   - When selected, the outline brightens (solid amber) so the user
///     can see what arrow-key nudge will affect.
pub fn instrument(ui: &mut egui::Ui, key: &'static str, base_rect: egui::Rect) -> egui::Rect {
    let ctx = ui.ctx().clone();
    let ov = override_for(&ctx, key);
    // Apply the saved offset (snapped to the user's grid) regardless of
    // editor state — the layout the dev saved IS the layout users see.
    let grid = effective_grid(&ctx);
    let visual_rect = base_rect.translate(egui::vec2(
        snap(ov.pos_offset_x, grid),
        snap(ov.pos_offset_y, grid),
    ));

    // When the editor is off we're done: no interact, no registry, no
    // overlay paint. Production builds pay only an `Arc<Data>` read +
    // a translate() per element.
    if !is_editor_on() {
        return visual_rect;
    }

    #[cfg(feature = "layout_editor")]
    {
        // Record this element so the panel's registry list can find it.
        registry_record(&ctx, key, visual_rect);

        // Interact on a foreground layer so the editor wins click + drag
        // priority over the underlying widget (otherwise clicking PLAY,
        // TEST, etc. would fire the button instead of selecting the
        // element).
        let resp = interact_on_overlay(
            ui,
            ("layout_edit", key),
            visual_rect,
            egui::Sense::click_and_drag(),
        );

        apply_click_select(&ctx, key, &resp);
        apply_drag(&ctx, key, &resp);
        apply_right_click_reset(&ctx, key, &resp);

        let selected = is_selected(&ctx, key);
        if resp.hovered() || resp.dragged() || selected {
            paint_drag_overlay(&ctx, visual_rect, resp.dragged(), selected);
        }
    }
    let _ = ui; // suppress unused warning when feature off
    visual_rect
}

/// Spawn a child Ui in the layout-editor overlay layer and run a single
/// `interact()` on it. Clicks land here instead of the underlying
/// widget because Foreground-order layers sit above the central panel.
#[cfg(feature = "layout_editor")]
fn interact_on_overlay(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash,
    rect: egui::Rect,
    sense: egui::Sense,
) -> egui::Response {
    let layer = egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("layout_editor_overlay"),
    );
    let child = ui.new_child(egui::UiBuilder::new().layer_id(layer).max_rect(rect));
    child.interact(rect, egui::Id::new(id_salt), sense)
}

/// Position-only variant for `painter.text(pos, ...)` anchors and other
/// no-rect draws. Returns the (possibly-shifted) anchor pos and registers
/// a small drag rect of `approx_size` centred on the visible anchor so
/// the developer can grab the text label and drag it directly.
///
/// The returned `Pos2` is `base_pos + snapped(offset)` — pass it straight
/// into `painter.text(returned_pos, align, ...)`.
///
/// `align` should match the alignment used by the caller's `painter.text`
/// call so the drag rect lines up with the rendered glyphs.
pub fn instrument_text(
    ui: &mut egui::Ui,
    key: &'static str,
    base_pos: egui::Pos2,
    approx_size: egui::Vec2,
    align: egui::Align2,
) -> egui::Pos2 {
    let ctx = ui.ctx().clone();
    let ov = override_for(&ctx, key);
    // Apply the saved offset (snapped) regardless of editor state so the
    // dev-saved layout is what the user sees with the editor off.
    let grid = effective_grid(&ctx);
    let visual_pos =
        base_pos + egui::vec2(snap(ov.pos_offset_x, grid), snap(ov.pos_offset_y, grid));

    if !is_editor_on() {
        return visual_pos;
    }

    #[cfg(feature = "layout_editor")]
    {
        // Build the drag rect that surrounds the rendered text. align
        // tells us where `visual_pos` sits relative to the glyph box.
        let (ax, ay) = (align.x(), align.y());
        let dx = match ax {
            egui::Align::Min => 0.0,
            egui::Align::Center => -approx_size.x * 0.5,
            egui::Align::Max => -approx_size.x,
        };
        let dy = match ay {
            egui::Align::Min => 0.0,
            egui::Align::Center => -approx_size.y * 0.5,
            egui::Align::Max => -approx_size.y,
        };
        let visual_rect = egui::Rect::from_min_size(visual_pos + egui::vec2(dx, dy), approx_size);

        registry_record(&ctx, key, visual_rect);

        let resp = interact_on_overlay(
            ui,
            ("layout_edit", key),
            visual_rect,
            egui::Sense::click_and_drag(),
        );
        apply_click_select(&ctx, key, &resp);
        apply_drag(&ctx, key, &resp);
        apply_right_click_reset(&ctx, key, &resp);
        let selected = is_selected(&ctx, key);
        if resp.hovered() || resp.dragged() || selected {
            paint_drag_overlay(&ctx, visual_rect, resp.dragged(), selected);
        }
    }
    #[cfg(not(feature = "layout_editor"))]
    {
        let _ = (ui, approx_size, align);
    }
    visual_pos
}

/// Pure offset accessor — returns `(dx, dy)` for `key`, snapped to the
/// active grid so dependent painters (e.g. BPM tracking the OUTPUT
/// display) move pixel-aligned. Applied unconditionally so the
/// dev-saved layout is what the user sees with the editor off.
pub fn offset_for(ctx: &egui::Context, key: &str) -> egui::Vec2 {
    let ov = snapshot(ctx).entries.get(key).copied().unwrap_or_default();
    let grid = effective_grid(ctx);
    egui::vec2(snap(ov.pos_offset_x, grid), snap(ov.pos_offset_y, grid))
}

#[cfg(feature = "layout_editor")]
fn apply_drag(ctx: &egui::Context, key: &str, resp: &egui::Response) {
    // On drag-start: if this element isn't already selected, replace the
    // selection with this one (or add to it if Shift is held). That way
    // grabbing an unselected element "picks it up" the way a typical
    // graphics editor does, while a click on an empty drag still allows
    // multi-select via Shift+click first.
    if resp.drag_started() {
        let shift = ctx.input(|i| i.modifiers.shift);
        let mut sel = get_selection(ctx);
        let already = sel.keys.iter().any(|k| k == key);
        if !already {
            if shift {
                sel.keys.push(key.to_string());
            } else {
                sel.keys = vec![key.to_string()];
            }
            set_selection(ctx, sel);
        }
    }

    if !resp.dragged() {
        return;
    }
    let delta = resp.drag_delta();
    if delta.length_sq() <= 0.0 {
        return;
    }

    // Accumulate RAW delta — sub-grid motion would otherwise be eaten by
    // round-snap on every frame. The visual snaps at draw time via
    // instrument()'s `snap(ov.pos_offset, grid)` translate.
    let mut snap_state = snapshot(ctx);
    let targets: Vec<String> = {
        let sel = get_selection(ctx);
        if sel.keys.iter().any(|k| k == key) {
            sel.keys.clone()
        } else {
            vec![key.to_string()]
        }
    };
    for target in &targets {
        let entry = snap_state.entries.entry(target.clone()).or_default();
        entry.pos_offset_x += delta.x;
        entry.pos_offset_y += delta.y;
    }
    write_snapshot(ctx, snap_state);
}

/// Click-without-drag selects. Shift toggles into existing selection,
/// plain click replaces. Called before `apply_drag` so the drag handler
/// can read the (possibly just-mutated) selection.
#[cfg(feature = "layout_editor")]
fn apply_click_select(ctx: &egui::Context, key: &str, resp: &egui::Response) {
    if !resp.clicked() {
        return;
    }
    let shift = ctx.input(|i| i.modifiers.shift);
    let mut sel = get_selection(ctx);
    if shift {
        if let Some(pos) = sel.keys.iter().position(|k| k == key) {
            sel.keys.remove(pos);
        } else {
            sel.keys.push(key.to_string());
        }
    } else {
        sel.keys = vec![key.to_string()];
    }
    set_selection(ctx, sel);
}

#[cfg(feature = "layout_editor")]
fn apply_right_click_reset(ctx: &egui::Context, key: &str, resp: &egui::Response) {
    if !resp.secondary_clicked() {
        return;
    }
    // If the right-clicked element is part of the current selection,
    // reset every selected element. Otherwise reset just this one.
    let mut snap_state = snapshot(ctx);
    let sel = get_selection(ctx);
    let targets: Vec<String> = if sel.keys.iter().any(|k| k == key) {
        sel.keys.clone()
    } else {
        vec![key.to_string()]
    };
    let mut any = false;
    for target in &targets {
        if snap_state.entries.remove(target).is_some() {
            any = true;
        }
    }
    if any {
        write_snapshot(ctx, snap_state);
        nih_plug::nih_log!(
            "[layout_editor] reset offset for {} element(s)",
            targets.len()
        );
    }
}

/// Per-element override by stable string key. Returns identity values
/// only when no entry has been recorded for `key`. Applied
/// unconditionally so the dev-saved layout persists when the editor UI
/// is off.
pub fn override_for(ctx: &egui::Context, key: &str) -> OverrideEntry {
    snapshot(ctx).entries.get(key).copied().unwrap_or_default()
}

/// Outline + 4 corner handles painted when an instrumented element is
/// hovered, dragged, or selected. Selected elements get a solid amber
/// outline + cyan handles so the user can tell at a glance which
/// elements arrow-key nudge will move. Active-drag uses a brighter
/// outline so they're distinguishable mid-gesture.
#[cfg(feature = "layout_editor")]
fn paint_drag_overlay(ctx: &egui::Context, rect: egui::Rect, active: bool, selected: bool) {
    // Paint into the overlay layer so the highlights sit on top of the
    // central panel content (and on top of the chassis bake's
    // anti-aliasing) rather than getting hidden behind it.
    let layer = egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("layout_editor_overlay"),
    );
    let painter = ctx.layer_painter(layer);
    let painter = &painter;
    let (outline, handle_color) = if active {
        (
            egui::Color32::from_rgb(0xff, 0xff, 0x80),
            egui::Color32::from_rgb(0xff, 0xff, 0xff),
        )
    } else if selected {
        (
            egui::Color32::from_rgb(0x40, 0xc0, 0xff),
            egui::Color32::from_rgb(0x80, 0xe0, 0xff),
        )
    } else {
        (
            egui::Color32::from_rgba_premultiplied(0xff, 0xc0, 0x40, 0xa0),
            egui::Color32::from_rgba_premultiplied(0xff, 0xc0, 0x40, 0xa0),
        )
    };
    let stroke_w = if selected { 1.5 } else { 1.0 };
    painter.rect_stroke(
        rect,
        0.0,
        egui::Stroke::new(stroke_w, outline),
        egui::StrokeKind::Outside,
    );
    let s = if selected { 5.0 } else { 4.0 };
    for (cx, cy) in [
        (rect.left(), rect.top()),
        (rect.right(), rect.top()),
        (rect.left(), rect.bottom()),
        (rect.right(), rect.bottom()),
    ] {
        let handle = egui::Rect::from_center_size(egui::pos2(cx, cy), egui::vec2(s, s));
        painter.rect_filled(handle, 0.0, handle_color);
    }
}

/// Arrow keys nudge every selected element. Plain arrow = 1 px,
/// Shift+arrow = 10 px (coarse). Escape clears the selection.
///
/// Call once per frame, BEFORE preset_bar / tempo / sequencer arrow-key
/// handlers. Drains ALL matching arrow-press events from the queue so
/// OS auto-repeat (which can deliver several events per frame at high
/// repeat rates) translates 1:1 into nudges instead of being throttled
/// to one-per-frame by `consume_key`.
///
/// `typing` should be `ctx.wants_keyboard_input()` so an active TextEdit
/// (preset rename) keeps its arrows.
#[cfg(feature = "layout_editor")]
pub fn handle_arrow_nudge(ctx: &egui::Context, typing: bool) {
    if !is_editor_on() || typing {
        return;
    }
    let sel = get_selection(ctx);
    if sel.keys.is_empty() {
        // Still allow Escape to clear (no-op, but harmless) and skip
        // arrow consumption so the rest of the UI keeps working.
        ctx.input_mut(|i| {
            i.events.retain(|e| {
                !matches!(
                    e,
                    egui::Event::Key {
                        key: egui::Key::Escape,
                        pressed: true,
                        ..
                    }
                )
            });
        });
        return;
    }

    // Drain all arrow + Escape events from the input queue in one pass,
    // counting matching presses (so OS auto-repeat events all register
    // as nudges) and removing them so they don't leak to preset_bar /
    // tempo / sequencer downstream.
    let (dx, dy, escape) = ctx.input_mut(|i| {
        let mut left_1 = 0i32;
        let mut right_1 = 0i32;
        let mut up_1 = 0i32;
        let mut down_1 = 0i32;
        let mut left_10 = 0i32;
        let mut right_10 = 0i32;
        let mut up_10 = 0i32;
        let mut down_10 = 0i32;
        let mut esc = false;
        i.events.retain(|e| match e {
            egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => {
                let shift = modifiers.shift;
                let plain = !modifiers.ctrl && !modifiers.alt && !modifiers.command;
                if !plain {
                    return true;
                }
                match key {
                    egui::Key::ArrowLeft if shift => {
                        left_10 += 1;
                        false
                    }
                    egui::Key::ArrowLeft => {
                        left_1 += 1;
                        false
                    }
                    egui::Key::ArrowRight if shift => {
                        right_10 += 1;
                        false
                    }
                    egui::Key::ArrowRight => {
                        right_1 += 1;
                        false
                    }
                    egui::Key::ArrowUp if shift => {
                        up_10 += 1;
                        false
                    }
                    egui::Key::ArrowUp => {
                        up_1 += 1;
                        false
                    }
                    egui::Key::ArrowDown if shift => {
                        down_10 += 1;
                        false
                    }
                    egui::Key::ArrowDown => {
                        down_1 += 1;
                        false
                    }
                    egui::Key::Escape if !shift => {
                        esc = true;
                        false
                    }
                    _ => true,
                }
            }
            _ => true,
        });
        let dx = (right_1 - left_1) as f32 * 1.0 + (right_10 - left_10) as f32 * 10.0;
        let dy = (down_1 - up_1) as f32 * 1.0 + (down_10 - up_10) as f32 * 10.0;
        (dx, dy, esc)
    });

    if escape {
        set_selection(ctx, Selection::default());
        return;
    }
    if dx == 0.0 && dy == 0.0 {
        return;
    }
    let mut snap_state = snapshot(ctx);
    for k in &sel.keys {
        let entry = snap_state.entries.entry(k.clone()).or_default();
        entry.pos_offset_x += dx;
        entry.pos_offset_y += dy;
    }
    write_snapshot(ctx, snap_state);
}

#[cfg(feature = "layout_editor")]
fn write_snapshot(ctx: &egui::Context, snap_state: LayoutOverrides) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(DATA_KEY), snap_state));
}

#[cfg(feature = "layout_editor")]
fn load_from_disk() -> Option<LayoutOverrides> {
    let path = store_path();
    let text = std::fs::read_to_string(&path).ok()?;
    match serde_json::from_str::<LayoutOverrides>(&text) {
        Ok(v) => Some(v),
        Err(e) => {
            nih_plug::nih_log!(
                "[layout_editor] failed to parse {}: {} (using defaults)",
                path.display(),
                e
            );
            None
        }
    }
}

#[cfg(feature = "layout_editor")]
fn save_to_disk(snap_state: &LayoutOverrides) -> std::io::Result<()> {
    let path = store_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(snap_state)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(path, json)
}

/// Alt+L handler — flips the atomic. Call once per frame from the
/// editor's input pump. Ignored when a TextEdit owns the keyboard.
///
/// Why not F12: egui-baseview's `translate_virtual_key` only handles
/// arrow/escape/tab/character keys — F-keys never reach egui. Alt+L
/// goes through the character-key path so it works under nih-plug.
#[cfg(feature = "layout_editor")]
pub fn handle_toggle(ctx: &egui::Context, typing: bool) {
    if typing {
        return;
    }
    let l_pressed = ctx.input(|i| i.key_pressed(egui::Key::L));
    if l_pressed {
        ctx.input(|i| {
            nih_plug::nih_log!(
                "[layout_editor] L pressed: ctrl={} shift={} alt={} cmd={}",
                i.modifiers.ctrl,
                i.modifiers.shift,
                i.modifiers.alt,
                i.modifiers.command,
            );
        });
    }
    let toggled = ctx.input(|i| {
        i.key_pressed(egui::Key::L) && i.modifiers.alt && !i.modifiers.shift && !i.modifiers.ctrl
    });
    if toggled {
        let prev = LAYOUT_EDITOR_ON.load(Ordering::Relaxed);
        LAYOUT_EDITOR_ON.store(!prev, Ordering::Relaxed);
        // Seed the temp data on first toggle-on so sliders render with
        // sensible defaults even when NINER_LAYOUT_EDITOR was unset at
        // launch (init() only seeds when the env var is on).
        if !prev {
            ctx.data_mut(|d| {
                let id = egui::Id::new(DATA_KEY);
                if d.get_temp::<LayoutOverrides>(id).is_none() {
                    d.insert_temp(id, LayoutOverrides::default());
                }
            });
        }
        nih_plug::nih_log!("[layout_editor] toggled → {}", !prev);
    }
}

/// Render the bulk-adjust window if the editor is on. Call last in the
/// frame so the window paints on top.
#[cfg(feature = "layout_editor")]
pub fn render_panel(ctx: &egui::Context) {
    if !is_editor_on() {
        return;
    }
    let mut snap_state = snapshot(ctx);
    let mut dirty = false;
    let mut clear_key: Option<String> = None;
    let registered = registry_keys(ctx);

    // Order::Foreground (same layer as the per-element drag overlay) so
    // clicks on Save/Reset/sliders aren't stolen by the overlay below.
    // Within a single layer, ties are broken by registration order — the
    // Window registers in render_panel which runs LAST in the frame, so
    // it sits above the overlay's already-registered interacts.
    egui::Window::new("Layout editor")
        .resizable(true)
        .order(egui::Order::Foreground)
        .default_pos(egui::pos2(20.0, 60.0))
        .default_size(egui::vec2(280.0, 460.0))
        .show(ctx, |ui| {
            ui.label(
                egui::RichText::new(
                    "Drag = move. Click = select. Shift+click = multi-select. \
                     Arrow keys = nudge 1px (Shift+arrow = 10px). \
                     Esc = clear selection. Right-click = reset. \
                     Hold Shift while dragging = bypass snap, Ctrl = ×4 grid.",
                )
                .small()
                .color(egui::Color32::GRAY),
            );
            ui.separator();

            // Selection summary — surfaces what arrow-key nudge will move.
            let sel_now = get_selection(ctx);
            let sel_text = if sel_now.keys.is_empty() {
                egui::RichText::new("Selection: (none)")
                    .small()
                    .color(egui::Color32::DARK_GRAY)
            } else if sel_now.keys.len() == 1 {
                egui::RichText::new(format!("Selection: {}", sel_now.keys[0]))
                    .small()
                    .color(egui::Color32::from_rgb(0x40, 0xc0, 0xff))
            } else {
                egui::RichText::new(format!(
                    "Selection: {} elements ({}, …)",
                    sel_now.keys.len(),
                    sel_now.keys[0]
                ))
                .small()
                .color(egui::Color32::from_rgb(0x40, 0xc0, 0xff))
            };
            ui.horizontal(|ui| {
                ui.label(sel_text);
                if !sel_now.keys.is_empty() && ui.small_button("clear").clicked() {
                    set_selection(ctx, Selection::default());
                }
            });
            ui.separator();

            // Snap controls — top of panel so they're easy to flip.
            ui.horizontal(|ui| {
                dirty |= ui
                    .checkbox(&mut snap_state.bulk.snap_enabled, "Snap")
                    .changed();
                ui.label("grid:");
                let grid_resp = ui.add_enabled(
                    snap_state.bulk.snap_enabled,
                    egui::DragValue::new(&mut snap_state.bulk.snap_grid)
                        .range(1.0..=16.0)
                        .speed(0.1)
                        .suffix(" px"),
                );
                dirty |= grid_resp.changed();
                if ui.small_button("1").clicked() {
                    snap_state.bulk.snap_grid = 1.0;
                    dirty = true;
                }
                if ui.small_button("2").clicked() {
                    snap_state.bulk.snap_grid = 2.0;
                    dirty = true;
                }
                if ui.small_button("4").clicked() {
                    snap_state.bulk.snap_grid = 4.0;
                    dirty = true;
                }
                if ui.small_button("8").clicked() {
                    snap_state.bulk.snap_grid = 8.0;
                    dirty = true;
                }
            });
            ui.separator();

            ui.collapsing("Bulk overrides", |ui| {
                ui.label("Chrome height scale");
                dirty |= ui
                    .add(
                        egui::Slider::new(&mut snap_state.bulk.chrome_height_scale, 0.5..=1.5)
                            .step_by(0.01),
                    )
                    .changed();

                ui.label("Chrome rounding delta (px)");
                dirty |= ui
                    .add(
                        egui::Slider::new(&mut snap_state.bulk.chrome_rounding_delta, -3.0..=6.0)
                            .step_by(0.5),
                    )
                    .changed();

                ui.label("Label font scale");
                dirty |= ui
                    .add(
                        egui::Slider::new(&mut snap_state.bulk.label_font_scale, 0.5..=1.5)
                            .step_by(0.01),
                    )
                    .changed();

                ui.label("Label padding delta (px)");
                dirty |= ui
                    .add(
                        egui::Slider::new(&mut snap_state.bulk.label_padding_delta, -4.0..=8.0)
                            .step_by(0.5),
                    )
                    .changed();
            });

            ui.separator();
            ui.collapsing(
                format!(
                    "Per-element offsets ({} of {} active)",
                    snap_state.entries.len(),
                    registered.len()
                ),
                |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(220.0)
                        .show(ui, |ui| {
                            // Sort registered keys for stable display.
                            let mut sorted = registered.clone();
                            sorted.sort();
                            for key in &sorted {
                                let entry =
                                    snap_state.entries.get(key).copied().unwrap_or_default();
                                let active = entry.pos_offset_x != 0.0 || entry.pos_offset_y != 0.0;
                                let label = if active {
                                    format!(
                                        "{} ({:+.0}, {:+.0})",
                                        key, entry.pos_offset_x, entry.pos_offset_y
                                    )
                                } else {
                                    key.clone()
                                };
                                ui.horizontal(|ui| {
                                    let txt = if active {
                                        egui::RichText::new(label)
                                            .color(egui::Color32::from_rgb(0xff, 0xc0, 0x40))
                                    } else {
                                        egui::RichText::new(label).color(egui::Color32::DARK_GRAY)
                                    };
                                    ui.label(txt);
                                    if ui.small_button("⟲").clicked() {
                                        clear_key = Some(key.clone());
                                    }
                                });
                            }
                        });
                },
            );

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Save layout").clicked() {
                    match save_to_disk(&snap_state) {
                        Ok(()) => {
                            nih_plug::nih_log!("[layout_editor] saved → {}", store_path().display())
                        }
                        Err(e) => nih_plug::nih_log!(
                            "[layout_editor] save failed ({}): {}",
                            store_path().display(),
                            e
                        ),
                    }
                }
                if ui.button("Reset all").clicked() {
                    snap_state = LayoutOverrides::default();
                    dirty = true;
                }
            });

            // Hover hint: highlight the currently-hovered registered
            // element by reading its rect from the registry and
            // outlining it. Done via a label that lists the hover.
            if let Some(hovered) = ctx.pointer_hover_pos().and_then(|p| {
                let mut found: Option<(String, egui::Rect)> = None;
                let keys = registry_keys(ctx);
                for k in keys {
                    if let Some(r) = registry_rect(ctx, &k) {
                        if r.contains(p) {
                            found = Some((k, r));
                            break;
                        }
                    }
                }
                found
            }) {
                ui.label(
                    egui::RichText::new(format!("hover: {}", hovered.0))
                        .small()
                        .color(egui::Color32::LIGHT_GRAY),
                );
            }
        });

    if let Some(k) = clear_key {
        if snap_state.entries.remove(&k).is_some() {
            dirty = true;
        }
    }
    if dirty {
        write_snapshot(ctx, snap_state);
    }
}
