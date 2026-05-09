//! Editor shell: creates the egui editor, drains audio→GUI telemetry, and
//! composes the header / master row / knob panels / footer / preset bar.
//!
//! All drawing primitives and row layouts live in sibling modules
//! (`widgets`, `seven_seg`, `panels`, `preset_bar`); this file is
//! deliberately kept small so the overall flow is easy to follow.

use nih_plug::prelude::*;
use nih_plug_egui::egui;
use nih_plug_egui::{create_egui_editor, EguiState};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::Arc;

use crate::export::{self, ExportOutcome};
use crate::midi_map::{
    decode_relative_delta, detect_cc_encoding, sentinel, CcEncoding, MidiInputEvent, MidiSource,
    NoteBlockMap,
};
use crate::params::NinerParams;
use crate::presets::PresetManager;
use crate::sequencer::Sequencer;
use crate::ui::panels::{self, CONTENT_LEFT, KNOB_SPACING};
use crate::ui::preset_bar::PresetBar;
use crate::ui::theme;
use crate::ui::widgets::{self, MidiLearnCtx};
use crate::util::messages::UiToDsp;
use crate::util::telemetry::{MeterShared, SpectrumShared, TelemetryConsumer};

use crate::dsp::spectrum::{BINS as SPECTRUM_BINS, DB_CEIL, DB_FLOOR};

/// Diagnostic: log the first N keyboard events egui delivers, then go
/// quiet. If a user reports "keyboard shortcuts don't work" the log will
/// show whether any key events arrive at egui at all — the common Windows
/// failure mode is that baseview/winit silently drops them.
static KEY_EVENT_LOG_COUNT: AtomicUsize = AtomicUsize::new(0);
const KEY_EVENT_LOG_MAX: usize = 32;

/// Rolling ring of recent audio peaks for the OUTPUT waveform display.
///
/// Fixed-capacity ring buffer with an explicit head cursor. `push` is O(1)
/// — replaces the original `Vec::remove(0)` which was O(n) on every drain
/// once the buffer was full (a full 200-element memmove per audio
/// telemetry frame). Memory layout: `buf` is allocated once at editor
/// construction and never resized; `head` is the next write slot, `full`
/// flips true on the first wrap.
struct WaveformDisplay {
    buf: Box<[f32]>,
    head: usize,
    full: bool,
}

impl WaveformDisplay {
    fn new(max_points: usize) -> Self {
        Self {
            buf: vec![0.0f32; max_points].into_boxed_slice(),
            head: 0,
            full: false,
        }
    }

    fn push(&mut self, peak: f32) {
        // Empty buffer (max_points = 0) is a degenerate config — guard so
        // `self.buf[0]` is never indexed past the slice.
        if self.buf.is_empty() {
            return;
        }
        self.buf[self.head] = peak;
        self.head += 1;
        if self.head >= self.buf.len() {
            self.head = 0;
            self.full = true;
        }
    }

    /// Two ordered slices — concatenate for an oldest-first walk. While
    /// the ring is still filling, `older` is empty and `newer` is the
    /// live prefix.
    fn slices(&self) -> (&[f32], &[f32]) {
        if self.full {
            let (newer, older) = self.buf.split_at(self.head);
            (older, newer)
        } else {
            (&[], &self.buf[..self.head])
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        if self.full {
            self.buf.len()
        } else {
            self.head
        }
    }
}

/// GUI-side spectrum state: freshest dB-per-band snapshot plus a slowly
/// decaying peak-hold line. Decay is driven by frame time, not audio time,
/// so the dots feel consistent regardless of buffer size.
struct SpectrumDisplay {
    bins: [f32; SPECTRUM_BINS],
    peak_hold: [f32; SPECTRUM_BINS],
}

impl SpectrumDisplay {
    fn new() -> Self {
        Self {
            bins: [DB_FLOOR; SPECTRUM_BINS],
            peak_hold: [DB_FLOOR; SPECTRUM_BINS],
        }
    }

    /// Drain atomic bin state from the audio thread, then apply peak-hold
    /// decay with a ~500 ms one-pole. `dt` is the frame time (seconds).
    fn update(&mut self, shared: &SpectrumShared, dt: f32) {
        // Peak-hold "tau" — seconds for the hold dot to decay by 1/e.
        // At 500 ms the dot lingers long enough to read a transient.
        let tau = 0.5f32;
        let decay_per_frame = (-dt / tau).exp();
        // Decay toward the floor so silent bands don't leave frozen dots at
        // mid-height indefinitely.
        for i in 0..SPECTRUM_BINS {
            let v = shared.load_bin(i).clamp(DB_FLOOR, DB_CEIL);
            self.bins[i] = v;
            let prior = self.peak_hold[i];
            // Decay the existing hold toward the floor, then max against the
            // current reading. This way a fresh transient instantly lights
            // the dot, but a decaying tail just lets it fall at tau=500 ms.
            let decayed = DB_FLOOR + (prior - DB_FLOOR) * decay_per_frame;
            self.peak_hold[i] = decayed.max(v);
        }
    }
}

// Factory function — each argument is genuinely distinct editor-owned state
// plumbed in from `Plugin::editor()`. Bundling them into a struct would just
// rename the same 8 fields, not reduce surface area.
#[allow(clippy::too_many_arguments)]
pub fn create(
    editor_state: Arc<EguiState>,
    params: Arc<NinerParams>,
    telemetry_rx: Option<TelemetryConsumer>,
    ui_tx: Option<rtrb::Producer<UiToDsp>>,
    midi_event_rx: Option<rtrb::Consumer<MidiInputEvent>>,
    note_block_map: Arc<NoteBlockMap>,
    midi_activity: Arc<AtomicU64>,
    preset_manager: Arc<Mutex<PresetManager>>,
    sequencer: Arc<Sequencer>,
    meter: Arc<MeterShared>,
    spectrum: Arc<SpectrumShared>,
) -> Option<Box<dyn Editor>> {
    let telemetry = Arc::new(Mutex::new(telemetry_rx));
    let waveform = Arc::new(Mutex::new(WaveformDisplay::new(200)));
    let spectrum_display = Arc::new(Mutex::new(SpectrumDisplay::new()));
    let preset_bar = Arc::new(Mutex::new(PresetBar::new(&preset_manager)));
    let ui_tx = Arc::new(Mutex::new(ui_tx));
    let seq_ui_state = Arc::new(Mutex::new(panels::SequencerUiState::default()));
    // Remembered export dir + format, loaded lazily from disk on first build.
    // The one-shot bounce button lives in the SAT/EQ row and fires through
    // this state so the next export opens at the same directory.
    let export_state = Arc::new(Mutex::new(export::load_export_state()));
    // Bounce runs on a worker thread — calling `rfd::FileDialog::save_file()`
    // from inside the egui paint closure pumps a nested Win32 message loop
    // while OpenGL is mid-frame, which crashed the app on Windows. The worker
    // owns its own thread context, and the receiver here lets the UI thread
    // drain the outcome once the thread finishes.
    let bounce_inflight: Arc<Mutex<Option<mpsc::Receiver<ExportOutcome>>>> =
        Arc::new(Mutex::new(None));
    let editor_state_clone = Arc::clone(&editor_state);
    // Visually smoothed GR meter value — instant attack, slow release, held
    // across frames so the bar doesn't flicker between audio buffers.
    let gr_display = Arc::new(Mutex::new(0.0f32));

    // Restore-last-preset state: read the name once here, apply on the first
    // frame where we've confirmed we're running standalone (not a DAW — the
    // host owns state restoration there).
    let pending_restore: Arc<Mutex<Option<String>>> =
        Arc::new(Mutex::new(crate::presets::load_last_preset_name()));

    // Header logo texture, lazily uploaded on first paint.
    let logo_texture: Arc<Mutex<Option<egui::TextureHandle>>> = Arc::new(Mutex::new(None));
    // Header "9" model-badge texture, drawn left of the wordmark.
    let nine_badge_texture: Arc<Mutex<Option<egui::TextureHandle>>> = Arc::new(Mutex::new(None));
    // Footer "manufacturer mark" (Hyperfocus DSP wordmark) — same lazy
    // upload pattern as the header logo, separate handle so each can be
    // sized independently if needed.
    let hf_logo_texture: Arc<Mutex<Option<egui::TextureHandle>>> = Arc::new(Mutex::new(None));
    // Photoreal baked chassis (Cycles render baked offline at
    // tools/blender/). Same lazy-upload pattern as the logos; replaces the
    // procedural BG_PANEL fill in panels::draw_chrome when present, falls
    // back to procedural on decode failure.
    let chassis_texture: Arc<Mutex<Option<egui::TextureHandle>>> = Arc::new(Mutex::new(None));
    // Optional separate screws overlay. When `assets/screws.png` is the
    // 1×1 transparent placeholder the runtime falls back to either the
    // screws baked into chassis.png (current production state) or
    // procedural screws when CHASSIS_BAKED is false. When the user
    // re-bakes a clean plate (`screws.enabled=false`) and ships a real
    // screws.png from `--only-screws`, this layer paints on top.
    let screws_texture: Arc<Mutex<Option<egui::TextureHandle>>> = Arc::new(Mutex::new(None));
    // Photoreal baked knob cap (Cycles render of a single neutral plastic
    // dome under the chassis studio rig). Runtime tints with `core_color`
    // via painter.image color multiply, so one bake serves all section
    // colours.
    let knob_cap_texture: Arc<Mutex<Option<egui::TextureHandle>>> = Arc::new(Mutex::new(None));
    // Cycles-baked display reflection overlay
    // (`assets/display_reflection.png`). Same lazy-upload pattern as the
    // chassis bake; replaces the procedural top sheen + 1-px specular line
    // on the OUTPUT display once `DISPLAY_BAKED` flips true.
    let display_reflection_texture: Arc<Mutex<Option<egui::TextureHandle>>> =
        Arc::new(Mutex::new(None));
    // Identity of the egui::Context the cached TextureHandles above were
    // uploaded against. Bitwig destroys the plugin window's GL context on
    // close and creates a new one on reopen; the renderer's texture cache
    // goes with it, but our Arc<Mutex<Option<…>>>'s outlive the closure for
    // the whole Editor object. Without this, every painter.image after a
    // reopen targets a freed TextureId — observed as a flood of "Failed to
    // find texture Managed(N)" warnings in ~/.BitwigStudio/log/engine.log
    // and a plain-black UI with no logos / chassis / knob caps.
    let cached_ctx: Arc<Mutex<Option<egui::Context>>> = Arc::new(Mutex::new(None));
    let dice_locks = Arc::new(std::sync::atomic::AtomicU8::new(0));

    // ── MIDI Learn plumbing ──
    // Build the param-id ↔ ParamPtr lookups once. `param_map()` walks the
    // `#[derive(Params)]` reflection metadata; the result is stable for the
    // lifetime of `params`, so we freeze both directions here.
    //
    // `id_to_ptr` is consumed by the GUI when applying an incoming CC: given
    // a binding's stored param_id string, find the underlying ParamPtr and
    // route the value through `setter.raw_context.raw_set_parameter_normalized`.
    //
    // `ptr_to_id` is the reverse: every `param_knob` call site already passes
    // `&FloatParam`, and at paint time we look up that pointer to identify
    // the param for the right-click menu — so no call site changes are needed
    // to opt into MIDI Learn.
    let (id_to_ptr, ptr_to_id): (
        Arc<HashMap<String, ParamPtr>>,
        Arc<HashMap<usize, String>>,
    ) = {
        let mut id_to_ptr = HashMap::new();
        let mut ptr_to_id = HashMap::new();
        for (id, ptr, _name) in params.param_map() {
            let raw = match ptr {
                ParamPtr::FloatParam(p) => p as usize,
                ParamPtr::IntParam(p) => p as usize,
                ParamPtr::BoolParam(p) => p as usize,
                ParamPtr::EnumParam(p) => p as usize,
            };
            ptr_to_id.insert(raw, id.clone());
            id_to_ptr.insert(id, ptr);
        }
        (Arc::new(id_to_ptr), Arc::new(ptr_to_id))
    };
    let midi_learn_state = Arc::clone(&params.midi_learn);
    let learn_armed: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let midi_event_rx = Arc::new(Mutex::new(midi_event_rx));
    // Sync the audio-thread NoteOn block bitmap with whatever's in the
    // persisted state on first frame. Without this, a session that already
    // has note bindings would still trigger the kick on every learned pad
    // until the user re-binds.
    note_block_map.rebuild_from(&midi_learn_state.lock());
    // MIDI activity indicator state — `last_count` snapshots the audio
    // thread's per-event counter; `last_change` records when we last
    // observed the counter advance, so we can decay the LED over ~200 ms
    // back to its dim resting colour. Held in `Arc<Mutex<>>` so the
    // captured `move` closure can mutate them across frames.
    let midi_activity_seen: Arc<Mutex<(u64, std::time::Instant)>> = Arc::new(Mutex::new((
        0,
        std::time::Instant::now()
            .checked_sub(std::time::Duration::from_secs(60))
            .unwrap_or_else(std::time::Instant::now),
    )));

    create_egui_editor(
        editor_state,
        (),
        |ctx, _| {
            theme::setup_fonts(ctx);
            theme::setup_style(ctx);
            // Optional dev-only layout editor — gated on the
            // `NINER_LAYOUT_EDITOR` env var so production runs are never
            // affected. F12 toggles the panel after init.
            crate::ui::layout_overrides::init(ctx);
        },
        move |ctx, setter, _state| {
            // Scaling is handled outside this callback: baseview applies the
            // window scale factor (standalone via `--dpi-scale`, DAW via
            // `Editor::set_scale_factor`), and egui's `pixels_per_point`
            // follows. We do NOT call `ctx.set_pixels_per_point()` here —
            // that fights baseview and double-scales the layout.
            let _ = &editor_state_clone;

            // Invalidate texture caches when the egui::Context changes. The
            // `Arc<Mutex<Option<TextureHandle>>>`s captured above outlive any
            // single GUI window; on Bitwig editor reopen the new context's
            // texture cache is empty, but `tex.is_none()` would still be
            // false and the lazy-upload blocks would skip. Clearing here
            // lets them re-fire against the new context on this frame.
            {
                let mut cached = cached_ctx.lock();
                if cached.as_ref() != Some(ctx) {
                    *cached = Some(ctx.clone());
                    *logo_texture.lock() = None;
                    *nine_badge_texture.lock() = None;
                    *hf_logo_texture.lock() = None;
                    *chassis_texture.lock() = None;
                    *screws_texture.lock() = None;
                    *knob_cap_texture.lock() = None;
                    *display_reflection_texture.lock() = None;
                    use std::sync::atomic::Ordering;
                    crate::ui::widgets::CHASSIS_BAKED.store(false, Ordering::Relaxed);
                    crate::ui::widgets::SCREWS_BAKED.store(false, Ordering::Relaxed);
                    crate::ui::widgets::KNOB_CAP_BAKED.store(false, Ordering::Relaxed);
                    crate::ui::widgets::DISPLAY_BAKED.store(false, Ordering::Relaxed);
                }
            }

            // Drain audio-thread telemetry into the waveform ring.
            drain_telemetry(&telemetry, &waveform);

            // Publish the MIDI Learn context so `param_knob` (and any other
            // widget that wants to expose a Learn menu) can pick it up via
            // `egui::Context::data` without us threading three more args
            // through every call site. Re-published every frame because
            // `insert_temp` doesn't survive the egui frame swap on every
            // backend.
            widgets::install_midi_learn_ctx(
                ctx,
                MidiLearnCtx {
                    state: Arc::clone(&midi_learn_state),
                    armed: Arc::clone(&learn_armed),
                    ptr_to_id: Arc::clone(&ptr_to_id),
                    note_block_map: Arc::clone(&note_block_map),
                },
            );

            // MIDI activity indicator: read the audio thread's monotonic
            // counter, freshen the "last seen" timestamp if it changed,
            // then publish a 0..1 intensity (1 = lit, 0 = dim) into
            // `ctx.data` so the tempo widget can render the dot. Decay
            // window 200 ms — a single CC event flashes the dot for two
            // or three frames, which reads as a definite blink without
            // looking jittery. We re-publish even when the counter
            // hasn't moved so the consumer can rely on the value being
            // current every frame.
            let intensity = {
                let now = midi_activity.load(Ordering::Relaxed);
                let mut guard = midi_activity_seen.lock();
                if now != guard.0 {
                    guard.0 = now;
                    guard.1 = std::time::Instant::now();
                }
                let elapsed = guard.1.elapsed().as_secs_f32();
                (1.0 - elapsed / 0.2).clamp(0.0, 1.0)
            };
            ctx.data_mut(|d| {
                d.insert_temp::<f32>(egui::Id::new("niner_midi_activity"), intensity);
            });

            // Drain the audio thread's MIDI event queue. Each event is
            // either a CC or a (pre-filtered) NoteOn — see plugin.rs for
            // the audio-side logic that decides which NoteOns reach this
            // queue. For each event we either:
            //   1) Capture it as the binding for the currently-armed
            //      param (LEARN mode), or
            //   2) Look up an existing binding and write the value back
            //      to the bound param via `ParamSetter`'s raw context
            //      (so the host sees it as automation). Unbound events
            //      are silently dropped.
            //
            // We go through `setter.raw_context` directly so we can drive
            // a `ParamPtr` (which `param_map()` hands back) instead of a
            // typed `&FloatParam` — there's no compile-time variant for
            // "generic param by id".
            {
                let mut rx_guard = midi_event_rx.lock();
                if let Some(rx) = rx_guard.as_mut() {
                    while let Ok(event) = rx.pop() {
                        let (channel, source, value) = match event {
                            MidiInputEvent::Cc { channel, cc, value } => {
                                (channel, MidiSource::Cc(cc), value)
                            }
                            MidiInputEvent::NoteOn {
                                channel,
                                note,
                                velocity,
                            } => (channel, MidiSource::NoteOn(note), velocity),
                        };

                        // 1) LEARN: consume the armed param if any.
                        let mut armed_guard = learn_armed.lock();
                        if let Some(armed_id) = armed_guard.take() {
                            // Auto-detect the encoding from the first
                            // captured CC value. Notes are always absolute
                            // (velocity → value), so the heuristic only runs
                            // for `MidiSource::Cc`.
                            //
                            // Important: a hard `Absolute` default looks
                            // attractive but breaks relative encoders
                            // (BeatStep MK1 in default mode, BCR2000,
                            // FaderFox) — turning the encoder produces a
                            // raw CC of 1 or 127, which Absolute would
                            // interpret as ~0 or ~1.0 (param flicks to the
                            // extremes). The heuristic catches that pattern
                            // (1|127 → BinaryOffset, 63|65 → Centered) and
                            // is correct for the BeatStep workflow we use.
                            // Edge case: an absolute pot whose first move
                            // happens to land on raw 1/63/65/127 will be
                            // mis-classified — fix via right-click → Encoding
                            // picker on that specific knob.
                            let encoding = match source {
                                MidiSource::Cc(_) => detect_cc_encoding(value),
                                MidiSource::NoteOn(_) => CcEncoding::Absolute,
                            };
                            let old = {
                                let mut state = midi_learn_state.lock();
                                let prev = state.binding_for_param(&armed_id);
                                state.bind(channel, source, &armed_id, encoding);
                                prev
                            };
                            // Keep the audio-thread NoteBlockMap in sync.
                            // Clear any prior block for this param (in case
                            // the user is rebinding from a note to a CC, or
                            // to a different note), and add the new one if
                            // applicable.
                            if let Some((old_ch, MidiSource::NoteOn(old_note), _)) = old {
                                note_block_map.unblock(old_ch, old_note);
                            }
                            if let MidiSource::NoteOn(note) = source {
                                note_block_map.block(channel, note);
                            }
                            // armed_guard is now None (we used `take`).
                            continue;
                        }
                        drop(armed_guard);

                        // 2) APPLY: look up the binding and write through.
                        let bound = midi_learn_state
                            .lock()
                            .lookup(channel, source)
                            .map(|(id, enc)| (id.to_string(), enc));
                        let Some((param_id, encoding)) = bound else {
                            continue;
                        };

                        // 2a) Sentinel targets — non-`Param` bindings like
                        // the standalone tempo or sequencer-play toggle.
                        match param_id.as_str() {
                            sentinel::TEMPO => {
                                // Skip when the host owns the transport.
                                if sequencer.is_host_synced() {
                                    continue;
                                }
                                // BPM range matches `Sequencer::set_bpm`'s
                                // internal clamp. Relative encoders nudge
                                // by 1 BPM per detent (1/8 with shift).
                                let new_bpm = match encoding {
                                    CcEncoding::Absolute => 40.0 + value * (240.0 - 40.0),
                                    CcEncoding::BinaryOffset
                                    | CcEncoding::Centered => {
                                        let delta = decode_relative_delta(value, encoding);
                                        let fine = ctx.input(|i| i.modifiers.shift);
                                        let step = if fine { 0.125 } else { 1.0 };
                                        (sequencer.bpm() + delta as f32 * step)
                                            .clamp(40.0, 240.0)
                                    }
                                };
                                sequencer.set_bpm(new_bpm);
                                continue;
                            }
                            sentinel::SEQ_PLAY => {
                                // Toggle on rising edge. Notes use
                                // velocity > 0; absolute CC uses value
                                // > 0.5; relative CCs ignore "no-op"
                                // values and trigger on any non-zero
                                // delta.
                                let trigger = match encoding {
                                    CcEncoding::Absolute => value > 0.5,
                                    CcEncoding::BinaryOffset
                                    | CcEncoding::Centered => {
                                        decode_relative_delta(value, encoding) != 0
                                    }
                                };
                                if trigger {
                                    sequencer.toggle_running();
                                }
                                continue;
                            }
                            _ => {}
                        }

                        // 2b) Normal `Param` target.
                        if let Some(&ptr) = id_to_ptr.get(&param_id) {
                            // For relative encoders, decode the value as
                            // a signed step delta and accumulate onto
                            // the param's current normalized value.
                            // Step granularity = 1/64 of full range per
                            // detent normally, /8 finer when the host's
                            // keyboard shift modifier is held.
                            //
                            // SAFETY: `setter.raw_context` is the live
                            // `dyn GuiContext` for this editor instance,
                            // and `ptr` came from this editor's own
                            // `params.param_map()`. Both live as long
                            // as the editor frame.
                            let new_value = match encoding {
                                CcEncoding::Absolute => value,
                                CcEncoding::BinaryOffset | CcEncoding::Centered => {
                                    let delta = decode_relative_delta(value, encoding);
                                    let fine = ctx.input(|i| i.modifiers.shift);
                                    let steps_per_range = if fine { 512.0 } else { 64.0 };
                                    let delta_norm = delta as f32 / steps_per_range;
                                    let current =
                                        unsafe { ptr.unmodulated_normalized_value() };
                                    (current + delta_norm).clamp(0.0, 1.0)
                                }
                            };
                            unsafe {
                                setter.raw_context.raw_begin_set_parameter(ptr);
                                setter
                                    .raw_context
                                    .raw_set_parameter_normalized(ptr, new_value);
                                setter.raw_context.raw_end_set_parameter(ptr);
                            }
                        }
                    }
                }
            }

            // Restore last-used preset once the audio thread has confirmed
            // we're standalone. Skipped entirely in DAW mode — the host
            // restores parameter state from the project file itself, and we
            // don't want to clobber it.
            if sequencer
                .transport_probed
                .load(std::sync::atomic::Ordering::Relaxed)
            {
                let mut pending = pending_restore.lock();
                if let Some(name) = pending.take() {
                    if !sequencer.is_host_synced() {
                        let mgr = preset_manager.lock();
                        if let Some(entry) = mgr.list_all().into_iter().find(|e| e.name == name) {
                            entry.params.apply(setter, &params);
                            // Reflect the selection in the preset bar UI.
                            let mut bar = preset_bar.lock();
                            bar.select_by_name(&entry.name);
                        }
                    }
                }
            }

            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    let panel_rect = ui.max_rect();

                    // Chassis texture (lazy upload — replaces the procedural
                    // BG fill in draw_chrome when present).
                    {
                        let mut tex = chassis_texture.lock();
                        if tex.is_none() {
                            let bytes = include_bytes!("../../assets/chassis.png");
                            if let Ok(img) = image::load_from_memory(bytes) {
                                let rgba = img.to_rgba8();
                                let (w, h) = rgba.dimensions();
                                let pixels = rgba.into_raw();
                                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                                    [w as usize, h as usize],
                                    &pixels,
                                );
                                *tex = Some(ctx.load_texture(
                                    "niner_chassis",
                                    color_image,
                                    egui::TextureOptions::LINEAR,
                                ));
                                // Tell draw_groove and any other gated
                                // chrome painters to skip — the bake
                                // includes real beveled grooves.
                                crate::ui::widgets::CHASSIS_BAKED
                                    .store(true, std::sync::atomic::Ordering::Relaxed);
                            }
                        }
                    }

                    // Screws overlay (lazy upload). The 1×1 placeholder
                    // ships in-tree so include_bytes! compiles; only flip
                    // SCREWS_BAKED if the loaded image is the real bake.
                    {
                        let mut tex = screws_texture.lock();
                        if tex.is_none() {
                            let bytes = include_bytes!("../../assets/screws.png");
                            if let Ok(img) = image::load_from_memory(bytes) {
                                let rgba = img.to_rgba8();
                                let (w, h) = rgba.dimensions();
                                let pixels = rgba.into_raw();
                                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                                    [w as usize, h as usize],
                                    &pixels,
                                );
                                *tex = Some(ctx.load_texture(
                                    "niner_screws",
                                    color_image,
                                    egui::TextureOptions::LINEAR,
                                ));
                                if w > 1 && h > 1 {
                                    crate::ui::widgets::SCREWS_BAKED
                                        .store(true, std::sync::atomic::Ordering::Relaxed);
                                }
                            }
                        }
                    }

                    // Knob cap (lazy upload). Same pattern — stash a clone
                    // in ctx.data so knob.rs can pick it up without
                    // threading a handle through every callsite.
                    {
                        let mut tex = knob_cap_texture.lock();
                        if tex.is_none() {
                            let bytes = include_bytes!("../../assets/knob_cap.png");
                            if let Ok(img) = image::load_from_memory(bytes) {
                                let rgba = img.to_rgba8();
                                let (w, h) = rgba.dimensions();
                                let pixels = rgba.into_raw();
                                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                                    [w as usize, h as usize],
                                    &pixels,
                                );
                                let handle = ctx.load_texture(
                                    "niner_knob_cap",
                                    color_image,
                                    egui::TextureOptions::LINEAR,
                                );
                                ctx.data_mut(|d| {
                                    d.insert_temp(
                                        egui::Id::new("niner_knob_cap_handle"),
                                        handle.clone(),
                                    );
                                });
                                *tex = Some(handle);
                                crate::ui::widgets::KNOB_CAP_BAKED
                                    .store(true, std::sync::atomic::Ordering::Relaxed);
                            }
                        }
                    }

                    // Display reflection (lazy upload — replaces the
                    // procedural sheen/specular on the OUTPUT display).
                    {
                        let mut tex = display_reflection_texture.lock();
                        if tex.is_none() {
                            let bytes = include_bytes!("../../assets/display_reflection.png");
                            if let Ok(img) = image::load_from_memory(bytes) {
                                let rgba = img.to_rgba8();
                                let (w, h) = rgba.dimensions();
                                let pixels = rgba.into_raw();
                                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                                    [w as usize, h as usize],
                                    &pixels,
                                );
                                let handle = ctx.load_texture(
                                    "niner_display_reflection",
                                    color_image,
                                    egui::TextureOptions::LINEAR,
                                );
                                // Stash a clone in ctx.data so the preset
                                // bar and 7-seg displays can pick it up
                                // without threading a new parameter through
                                // every call site.
                                ctx.data_mut(|d| {
                                    d.insert_temp(
                                        egui::Id::new("niner_display_reflection_handle"),
                                        handle.clone(),
                                    );
                                });
                                *tex = Some(handle);
                                crate::ui::widgets::DISPLAY_BAKED
                                    .store(true, std::sync::atomic::Ordering::Relaxed);
                            }
                        }
                    }

                    // ===== Panel chrome =====
                    let header_center_y = panels::draw_chrome(
                        ui,
                        panel_rect,
                        chassis_texture.lock().as_ref(),
                        screws_texture.lock().as_ref(),
                    );

                    // Header lockup: "9" model-badge + NINER wordmark.
                    // Both lazy-uploaded textures, drawn as separate elements
                    // so each can be tuned independently in the layout editor
                    // (`header.niner_9` and `header.niner_logo`). Aspect
                    // ratios are sourced from the trimmed PNGs in
                    // `assets/niner_9.png` and `assets/niner_logo.png` —
                    // re-run `rsvg-convert | magick -trim` and update these
                    // ratios when the wordmark/badge SVGs change.
                    let lockup_h = 20.0;
                    let nine_w = lockup_h * (64.0 / 80.0); // ~16.0 px
                    let wordmark_w = lockup_h * (342.0 / 80.0); // ~85.5 px
                    let lockup_gap = 5.0;

                    // "9" badge — sits at CONTENT_LEFT, left of the wordmark.
                    {
                        let mut tex = nine_badge_texture.lock();
                        if tex.is_none() {
                            let bytes = include_bytes!("../../assets/niner_9.png");
                            if let Ok(img) = image::load_from_memory(bytes) {
                                let rgba = img.to_rgba8();
                                let (w, h) = rgba.dimensions();
                                let pixels = rgba.into_raw();
                                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                                    [w as usize, h as usize],
                                    &pixels,
                                );
                                *tex = Some(ctx.load_texture(
                                    "niner_9",
                                    color_image,
                                    egui::TextureOptions::LINEAR,
                                ));
                            }
                        }
                        if let Some(t) = tex.as_ref() {
                            let base_rect = egui::Rect::from_min_size(
                                egui::pos2(
                                    panel_rect.left() + CONTENT_LEFT,
                                    header_center_y - lockup_h * 0.5,
                                ),
                                egui::vec2(nine_w, lockup_h),
                            );
                            let rect = crate::ui::layout_overrides::instrument(
                                ui,
                                "header.niner_9",
                                base_rect,
                            );
                            ui.painter().image(
                                t.id(),
                                rect,
                                egui::Rect::from_min_max(
                                    egui::pos2(0.0, 0.0),
                                    egui::pos2(1.0, 1.0),
                                ),
                                // Identity tint — render the PNG's actual
                                // oxide red, don't multiply by `theme::WHITE`
                                // (which is now brand bone, not pure white).
                                egui::Color32::WHITE,
                            );
                        }
                    }

                    // NINER wordmark — sits to the right of the "9" badge.
                    {
                        let mut tex = logo_texture.lock();
                        if tex.is_none() {
                            let bytes = include_bytes!("../../assets/niner_logo.png");
                            if let Ok(img) = image::load_from_memory(bytes) {
                                let rgba = img.to_rgba8();
                                let (w, h) = rgba.dimensions();
                                let pixels = rgba.into_raw();
                                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                                    [w as usize, h as usize],
                                    &pixels,
                                );
                                *tex = Some(ctx.load_texture(
                                    "niner_logo",
                                    color_image,
                                    egui::TextureOptions::LINEAR,
                                ));
                            }
                        }
                        if let Some(t) = tex.as_ref() {
                            let base_logo_rect = egui::Rect::from_min_size(
                                egui::pos2(
                                    panel_rect.left() + CONTENT_LEFT + nine_w + lockup_gap,
                                    header_center_y - lockup_h * 0.5,
                                ),
                                egui::vec2(wordmark_w, lockup_h),
                            );
                            let logo_rect = crate::ui::layout_overrides::instrument(
                                ui,
                                "header.niner_logo",
                                base_logo_rect,
                            );
                            ui.painter().image(
                                t.id(),
                                logo_rect,
                                egui::Rect::from_min_max(
                                    egui::pos2(0.0, 0.0),
                                    egui::pos2(1.0, 1.0),
                                ),
                                // Identity tint — render the PNG's actual
                                // bone, don't multiply by `theme::WHITE`.
                                egui::Color32::WHITE,
                            );
                        }
                    }

                    // UI scale badge — discreet click-to-cycle, lives in the
                    // header to the left of "KICK SYNTHESIZER" so the footer
                    // chrome stays clean. Mirrors the SquelchBox `band1.rs`
                    // pattern; the new value is mirrored to a sidecar file so
                    // `niner-launch` can forward it as `--dpi-scale` on the
                    // next standalone launch (DAWs honour `#[persist]` directly).
                    {
                        let scale = *params.ui_scale.lock();
                        let scale_text = if (scale - scale.round()).abs() < 0.05 {
                            format!("UI {:.0}×", scale)
                        } else {
                            format!("UI {:.1}×", scale)
                        };
                        // UI 1× badge sits between TEST and KICK SYNTHESIZER,
                        // anchored to TEST's rendered right edge so it follows
                        // when TEST is repositioned in the layout editor.
                        let test_right = panel_rect.left()
                            + CONTENT_LEFT
                            + 111.0
                            + 40.0
                            + crate::ui::layout_overrides::offset_for(ctx, "header.test_btn").x;
                        let badge_w = 50.0;
                        let badge_h = 14.0;
                        let base_badge_pos = egui::pos2(test_right + 8.0, header_center_y);
                        let badge_pos = crate::ui::layout_overrides::instrument_text(
                            ui,
                            "header.ui_scale",
                            base_badge_pos,
                            egui::vec2(badge_w, badge_h),
                            egui::Align2::LEFT_CENTER,
                        );
                        let hit = egui::Rect::from_min_size(
                            egui::pos2(badge_pos.x, badge_pos.y - badge_h * 0.5),
                            egui::vec2(badge_w, badge_h),
                        );
                        let resp = ui
                            .interact(hit, egui::Id::new("ui_scale_btn"), egui::Sense::click())
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text(
                                "UI scale — click to cycle (1× / 1.5× / 2×).\n\
                                 Reopen the plugin (or restart niner) to apply.",
                            );
                        let color = if resp.hovered() {
                            theme::WHITE
                        } else {
                            theme::TEXT_DIM
                        };
                        ui.painter().text(
                            badge_pos,
                            egui::Align2::LEFT_CENTER,
                            &scale_text,
                            egui::FontId::new(8.0, egui::FontFamily::Monospace),
                            color,
                        );
                        if resp.clicked() {
                            let mut lock = params.ui_scale.lock();
                            let next = match *lock {
                                v if v < 1.25 => 1.5,
                                v if v < 1.75 => 2.0,
                                _ => 1.0,
                            };
                            *lock = next;
                            crate::util::paths::save_ui_scale(next);
                            tracing::info!(
                                "[ui_scale] cycled → {next}× (saved; reopen plugin to apply)"
                            );
                        }
                    }

                    // Diagnostic: log the first few key events so we can
                    // tell whether keys are reaching egui at all on Windows.
                    // Bounded so a long session doesn't spam the log.
                    if KEY_EVENT_LOG_COUNT.load(Ordering::Relaxed) < KEY_EVENT_LOG_MAX {
                        ctx.input(|i| {
                            for event in &i.events {
                                if matches!(event, egui::Event::Key { .. }) {
                                    let n = KEY_EVENT_LOG_COUNT.fetch_add(1, Ordering::Relaxed);
                                    if n < KEY_EVENT_LOG_MAX {
                                        tracing::info!(
                                            "[keyboard] event #{}: {:?} (focus={})",
                                            n,
                                            event,
                                            i.focused
                                        );
                                    }
                                }
                            }
                        });
                    }

                    // Skip global shortcuts when a TextEdit wants the keys —
                    // otherwise typing "T" in the preset-name field would
                    // also fire a test kick.
                    let typing = ctx.wants_keyboard_input();

                    // ===== Test trigger (button + keyboard 'T') =====
                    // On Windows standalone, egui never sees key events (the
                    // outer nih-plug WindowHandler returns EventStatus::Ignored
                    // for Event::Keyboard, so baseview->egui-baseview never
                    // translates anything). We OR in a GetAsyncKeyState poll,
                    // gated by our own `typing` guard and a foreground-thread
                    // check inside `win_keys`, so the fallback only fires
                    // when Niner is the focused app.
                    let button_fired = panels::test_button(ui, panel_rect, header_center_y);
                    let t_egui = !typing && ui.input(|i| i.key_pressed(egui::Key::T));
                    #[cfg(target_os = "windows")]
                    let t_win32 = !typing && {
                        use std::sync::atomic::AtomicBool;
                        static PREV: AtomicBool = AtomicBool::new(false);
                        crate::win_keys::just_pressed(crate::win_keys::VK_T, &PREV)
                    };
                    #[cfg(not(target_os = "windows"))]
                    let t_win32 = false;
                    let key_fired = t_egui || t_win32;
                    if button_fired || key_fired {
                        if key_fired {
                            tracing::info!(
                                "[keyboard] T shortcut fired (egui={}, win32={})",
                                t_egui,
                                t_win32
                            );
                        }
                        if let Some(tx) = ui_tx.lock().as_mut() {
                            // Dropped triggers are intentional: the ring is
                            // small, and the user won't notice one missed
                            // test-kick. No panic, no log spam.
                            let _ = tx.push(UiToDsp::Trigger);
                        }
                    }

                    // Spacebar toggles the standalone sequencer. Gated off in
                    // DAW mode so the host's own transport owns Space.
                    // Same Windows-standalone fallback as T above.
                    let space_egui = !typing && ui.input(|i| i.key_pressed(egui::Key::Space));
                    #[cfg(target_os = "windows")]
                    let space_win32 = !typing && {
                        use std::sync::atomic::AtomicBool;
                        static PREV: AtomicBool = AtomicBool::new(false);
                        crate::win_keys::just_pressed(crate::win_keys::VK_SPACE, &PREV)
                    };
                    #[cfg(not(target_os = "windows"))]
                    let space_win32 = false;
                    if (space_egui || space_win32) && !sequencer.is_host_synced() {
                        tracing::info!(
                            "[keyboard] Space shortcut fired (egui={}, win32={})",
                            space_egui,
                            space_win32
                        );
                        sequencer.toggle_running();
                    }

                    // Alt+L toggles the dev-only layout editor (no-op
                    // when `NINER_LAYOUT_EDITOR` was unset at startup
                    // *and* the user hasn't already turned it on this
                    // session). Compiled out unless `--features
                    // layout_editor` is set.
                    #[cfg(feature = "layout_editor")]
                    {
                        crate::ui::layout_overrides::handle_toggle(ctx, typing);

                        // Arrow keys nudge selected elements when the
                        // editor is on. Runs BEFORE preset_bar / tempo
                        // so consumed events don't fire prev/next or
                        // BPM ±10. No-op when selection is empty.
                        crate::ui::layout_overrides::handle_arrow_nudge(ctx, typing);

                        // Ctrl+Z / Ctrl+Y (also Ctrl+Shift+Z) for layout
                        // undo/redo. Internally gated on is_editor_on so
                        // the keys remain free for plugin shortcuts when
                        // the editor is off.
                        crate::ui::layout_overrides::handle_undo_redo(ctx, typing);
                    }

                    // ===== Header preset bar =====
                    {
                        let mut bar = preset_bar.lock();
                        let dt = ctx.input(|i| i.unstable_dt);
                        let preset_origin_x = panel_rect.left() + CONTENT_LEFT + 167.0;
                        bar.render(
                            ui,
                            setter,
                            &params,
                            &preset_manager,
                            preset_origin_x,
                            header_center_y,
                            dt,
                        );
                    }

                    // ===== Groove below header =====
                    let groove_y = panel_rect.top() + 36.0;
                    {
                        let painter = ui.painter();
                        crate::ui::widgets::draw_groove(
                            painter,
                            panel_rect.left() + CONTENT_LEFT - 4.0,
                            panel_rect.right() - CONTENT_LEFT + 4.0,
                            groove_y,
                        );
                    }

                    // ===== Master row (OUTPUT + master knobs + comp strip) =====
                    let master_y = groove_y + 6.0;
                    let wf_left = panel_rect.left() + CONTENT_LEFT;
                    let wf_width = 7.0 * KNOB_SPACING - 16.0;
                    let wf_height = 56.0;

                    // Pull latest GR from the audio thread and apply a one-pole
                    // visual smoother: instant attack, ~180 ms release. `dt`
                    // from egui is already the frame time.
                    let dt = ctx.input(|i| i.unstable_dt).max(1e-4);
                    let gr_live = meter.load_gr_db();
                    let gr_smoothed = {
                        let mut g = gr_display.lock();
                        if gr_live >= *g {
                            *g = gr_live;
                        } else {
                            let release_tau = 0.18; // seconds
                            let a = (-dt / release_tau).exp();
                            *g = *g * a + gr_live * (1.0 - a);
                        }
                        *g
                    };

                    // Drain atomic spectrum bins + decay peak-hold once per
                    // frame, before any MasterRow draws read the values.
                    {
                        let mut sd = spectrum_display.lock();
                        sd.update(&spectrum, dt);
                    }

                    {
                        let wf = waveform.lock();
                        let sd = spectrum_display.lock();
                        let mode = panels::display_mode(ctx);
                        let display_reflection_lock = display_reflection_texture.lock();
                        let (wf_older, wf_newer) = wf.slices();
                        let master_row = panels::MasterRow {
                            master_y,
                            wf_left,
                            wf_width,
                            wf_height,
                            waveform_peaks_older: wf_older,
                            waveform_peaks_newer: wf_newer,
                            spectrum_bins: &sd.bins,
                            spectrum_peak_hold: &sd.peak_hold,
                            display_mode: mode,
                            gr_db: gr_smoothed,
                            display_reflection: display_reflection_lock.as_ref(),
                        };
                        master_row.draw(ui, setter, &params, panel_rect);
                    }

                    // BPM readout — anchored to the lower-left corner of
                    // the master display's lit area so it reads as part of
                    // the screen, like a real piece of hardware shows
                    // tempo on its main LCD. Follows the display when the
                    // user drags it via the layout editor; can also be
                    // dragged independently via its own "master.bpm" key.
                    {
                        let lit = crate::ui::widgets::lit_rect_default(
                            wf_left, master_y, wf_width, wf_height,
                        );
                        let bpm_pos = crate::ui::layout_overrides::instrument_text(
                            ui,
                            "master.bpm",
                            egui::pos2(lit.left() + 2.0, lit.bottom() - 12.0),
                            egui::vec2(80.0, 12.0),
                            egui::Align2::LEFT_TOP,
                        );
                        let mut seq_ui = seq_ui_state.lock();
                        panels::draw_tempo_widget(
                            ui,
                            bpm_pos,
                            &sequencer,
                            sequencer.is_host_synced(),
                            &mut seq_ui.tempo_edit,
                        );
                    }

                    // ===== Three knob rows =====
                    let master_bottom_y = master_y + wf_height;
                    let sub_top_bottom_y =
                        panels::draw_sub_top_row(ui, setter, &params, panel_rect, master_bottom_y);
                    let mid_bottom_y =
                        panels::draw_mid_row(ui, setter, &params, panel_rect, sub_top_bottom_y);
                    let sat_eq_result =
                        panels::draw_sat_eq_row(ui, setter, &params, panel_rect, mid_bottom_y);
                    let sat_eq_bottom_y = sat_eq_result.next_y;

                    // ===== Filter (SAT/EQ right column) =====
                    // Anchor the small FILT/RES cluster to the EQ knob
                    // baseline so its 18 px knobs stay vertically centred
                    // against the 32 px EQ knobs regardless of SAT cluster
                    // height. Mirrors how the CLAP small cluster aligns
                    // against the big MID knobs in the row above.
                    {
                        let filter_top = sat_eq_result.eq_knob_y + (panels::KNOB_SIZE - 18.0) * 0.5;
                        panels::draw_filter_cluster(ui, setter, &params, panel_rect, filter_top);
                    }

                    // ===== DICE + BOUNCE (sequencer right column) =====
                    {
                        // DICE sits directly under the FILTER cluster on the
                        // right column. Anchored to `eq_knob_y` rather than
                        // `sat_eq_bottom_y` so the SAT-cluster row extension
                        // doesn't push DICE into the BOUNCE button.
                        // Filter cluster top = eq_knob_y + 7, height = ~66 px
                        // (knob row + 32 px gap + caption), so its bottom sits
                        // at eq_knob_y + 73. DICE lands 5 px below that.
                        let dice_top = sat_eq_result.eq_knob_y + 78.0;
                        let dice_clicked =
                            panels::draw_dice_row(ui, panel_rect, dice_top, &dice_locks);
                        if dice_clicked {
                            let locked = dice_locks.load(std::sync::atomic::Ordering::Relaxed);
                            crate::ui::randomize::randomize(setter, &params, locked);
                        }
                        // BOUNCE shares the sequencer row vertically with PLAY +
                        // the 16 step pads. Mirror the pad_top formula in
                        // panels::draw_sequencer_row so they stay locked.
                        let bounce_top = sat_eq_bottom_y + 4.0 + 14.0 + 6.0;
                        let bounce_row = panels::draw_bounce_button(ui, panel_rect, bounce_top);
                        let bounce_clicked = bounce_row.bounce_clicked;
                        if bounce_row.clear_clicked {
                            sequencer.clear_pattern();
                        }

                        // Drain any completed bounce from the worker thread
                        // first so the next click isn't blocked by a stale
                        // receiver.
                        {
                            let mut slot = bounce_inflight.lock();
                            let drained = if let Some(rx) = slot.as_ref() {
                                match rx.try_recv() {
                                    Ok(outcome) => {
                                        match outcome {
                                            ExportOutcome::Written(path) => {
                                                tracing::info!("bounce written: {}", path.display())
                                            }
                                            ExportOutcome::Cancelled => {}
                                            ExportOutcome::UnsupportedExtension(ext) => {
                                                tracing::warn!(
                                                    "bounce: unsupported extension .{}",
                                                    ext
                                                );
                                            }
                                            ExportOutcome::Failed(msg) => {
                                                tracing::error!("bounce failed: {}", msg);
                                            }
                                        }
                                        true
                                    }
                                    Err(mpsc::TryRecvError::Empty) => false,
                                    Err(mpsc::TryRecvError::Disconnected) => {
                                        tracing::error!(
                                            "bounce worker disconnected without result"
                                        );
                                        true
                                    }
                                }
                            } else {
                                false
                            };
                            if drained {
                                *slot = None;
                            }
                        }

                        if bounce_clicked {
                            let mut slot = bounce_inflight.lock();
                            if slot.is_some() {
                                tracing::info!("bounce: worker still running, ignoring click");
                            } else {
                                let (tx, rx) = mpsc::channel();
                                let export_state_worker = Arc::clone(&export_state);
                                let params_worker = Arc::clone(&params);
                                let spawn_result = std::thread::Builder::new()
                                    .name("niner-bounce".into())
                                    .spawn(move || {
                                        let outcome = {
                                            let mut state = export_state_worker.lock();
                                            export::export_one_shot(&mut state, &params_worker)
                                        };
                                        let _ = tx.send(outcome);
                                    });
                                match spawn_result {
                                    Ok(_handle) => *slot = Some(rx),
                                    Err(e) => {
                                        tracing::error!("bounce: failed to spawn worker: {}", e);
                                    }
                                }
                            }
                        }
                    }

                    // ===== Step sequencer =====
                    {
                        let mut seq_ui = seq_ui_state.lock();
                        panels::draw_sequencer_row(
                            ui,
                            setter,
                            &params,
                            panel_rect,
                            sat_eq_bottom_y,
                            &sequencer,
                            &mut seq_ui,
                        );
                    }

                    // ===== Footer =====
                    panels::draw_footer(ui, panel_rect);

                    // Footer manufacturer mark — full Hyperfocus DSP wordmark
                    // (with small-caps DSP suffix and ring-as-O) left-aligned
                    // in the footer strip. Sourced from `wordmark-master.svg`,
                    // not the no-DSP `wordmark-only.svg` derivative.
                    {
                        let mut tex = hf_logo_texture.lock();
                        if tex.is_none() {
                            let bytes = include_bytes!("../../assets/hyperfocus_dsp_logo.png");
                            if let Ok(img) = image::load_from_memory(bytes) {
                                let rgba = img.to_rgba8();
                                let (w, h) = rgba.dimensions();
                                let pixels = rgba.into_raw();
                                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                                    [w as usize, h as usize],
                                    &pixels,
                                );
                                *tex = Some(ctx.load_texture(
                                    "hyperfocus_dsp_logo",
                                    color_image,
                                    egui::TextureOptions::LINEAR,
                                ));
                            }
                        }
                        if let Some(t) = tex.as_ref() {
                            // Source rendered at 142×24 (rsvg-convert -h 24);
                            // 8 px tall is a 3× downscale that LINEAR handles
                            // cleanly without aliasing, and fits inside the
                            // 10-px slot between the footer groove (y=422)
                            // and the bottom edge band (y=432) without
                            // crossing either, mirroring how the header
                            // strip clears the top edge band.
                            let logo_h = 8.0;
                            let [tex_w, tex_h] = t.size();
                            let logo_w = logo_h * (tex_w as f32 / tex_h as f32);
                            let strip_y = panel_rect.bottom() - 21.0;
                            let base_logo_rect = egui::Rect::from_min_size(
                                egui::pos2(panel_rect.left() + CONTENT_LEFT, strip_y),
                                egui::vec2(logo_w, logo_h),
                            );
                            let logo_rect = crate::ui::layout_overrides::instrument(
                                ui,
                                "footer.hyperfocus_logo",
                                base_logo_rect,
                            );
                            ui.painter().image(
                                t.id(),
                                logo_rect,
                                egui::Rect::from_min_max(
                                    egui::pos2(0.0, 0.0),
                                    egui::pos2(1.0, 1.0),
                                ),
                                // Identity tint — render the HF wordmark at
                                // its true bone color (its source PNG is
                                // already brand bone), don't double-tint.
                                egui::Color32::WHITE,
                            );
                            let logo_resp = ui.interact(
                                logo_rect,
                                egui::Id::new("hyperfocus_brand"),
                                egui::Sense::hover(),
                            );
                            if logo_resp.hovered() {
                                logo_resp.on_hover_text("Made by Hyperfocus DSP");
                            }
                        }
                    }

                    // Footer "feedback" link — sits opposite the wordmark,
                    // opens the system mail client with a pre-filled mailto
                    // so reports land with version + OS + arch attached.
                    {
                        let feedback_w = 50.0;
                        let feedback_h = 8.0;
                        let strip_y = panel_rect.bottom() - 21.0;
                        let base_pos = egui::pos2(
                            panel_rect.right() - CONTENT_LEFT,
                            strip_y + feedback_h * 0.5,
                        );
                        let feedback_pos = crate::ui::layout_overrides::instrument_text(
                            ui,
                            "footer.feedback_link",
                            base_pos,
                            egui::vec2(feedback_w, feedback_h),
                            egui::Align2::RIGHT_CENTER,
                        );
                        let hit = egui::Rect::from_min_size(
                            egui::pos2(
                                feedback_pos.x - feedback_w,
                                feedback_pos.y - feedback_h * 0.5,
                            ),
                            egui::vec2(feedback_w, feedback_h),
                        );
                        let resp = ui
                            .interact(
                                hit,
                                egui::Id::new("footer_feedback"),
                                egui::Sense::click(),
                            )
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text("Send feedback (opens your mail client)");
                        let color = if resp.hovered() {
                            theme::WHITE
                        } else {
                            theme::TEXT_DIM
                        };
                        ui.painter().text(
                            feedback_pos,
                            egui::Align2::RIGHT_CENTER,
                            "feedback",
                            egui::FontId::new(8.0, egui::FontFamily::Monospace),
                            color,
                        );
                        if resp.clicked() {
                            crate::util::feedback::open_feedback();
                        }
                    }

                    // Final cursor override — must run AFTER every knob-panel
                    // draw so the dropdown's PointingHand wins last-write
                    // against the knob widget's ResizeVertical. See
                    // `PresetBar::apply_late_cursor` for the rationale.
                    {
                        let bar = preset_bar.lock();
                        bar.apply_late_cursor(ui);
                    }
                });

            // Dev-only layout editor — paints the bulk-adjust window on
            // top of the central panel. Compiled out unless `--features
            // layout_editor` is set.
            #[cfg(feature = "layout_editor")]
            crate::ui::layout_overrides::render_panel(ctx);
        },
    )
}

fn drain_telemetry(
    telemetry: &Mutex<Option<TelemetryConsumer>>,
    waveform: &Mutex<WaveformDisplay>,
) {
    let mut tel = telemetry.lock();
    let mut wf = waveform.lock();
    if let Some(rx) = tel.as_mut() {
        let mut temp = Vec::new();
        rx.drain_into(&mut temp, 128);
        for &p in &temp {
            wf.push(p);
        }
    }
}

#[cfg(test)]
mod waveform_tests {
    use super::WaveformDisplay;

    #[test]
    fn ring_wraparound_yields_oldest_first() {
        // Capacity 5; push 7 values so the ring wraps. The oldest two are
        // overwritten — the iter must yield 3, 4, 5, 6, 7 in order.
        let mut wf = WaveformDisplay::new(5);
        for v in 1..=7 {
            wf.push(v as f32);
        }
        let (older, newer) = wf.slices();
        let collected: Vec<f32> = older.iter().chain(newer.iter()).copied().collect();
        assert_eq!(collected, vec![3.0, 4.0, 5.0, 6.0, 7.0]);
        assert_eq!(wf.len(), 5);
    }

    #[test]
    fn before_wrap_older_is_empty() {
        let mut wf = WaveformDisplay::new(10);
        wf.push(1.0);
        wf.push(2.0);
        wf.push(3.0);
        let (older, newer) = wf.slices();
        assert!(older.is_empty());
        assert_eq!(newer, &[1.0, 2.0, 3.0][..]);
        assert_eq!(wf.len(), 3);
    }

    #[test]
    fn empty_capacity_does_not_panic() {
        // Edge case: 0-cap is degenerate but must not panic.
        let mut wf = WaveformDisplay::new(0);
        wf.push(1.0);
        assert_eq!(wf.len(), 0);
        let (a, b) = wf.slices();
        assert!(a.is_empty() && b.is_empty());
    }
}
