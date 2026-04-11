//! Header-integrated preset bar: prev/next arrows, LED preset name display
//! with dropdown, SAVE/DEL buttons, and transient status message.
//!
//! The `PresetBar` struct owns all of its mutable UI state and a cached
//! preset list. The cache is refreshed lazily — once on construction, then
//! only after a successful `save`/`delete` — so the filesystem is not scanned
//! on every egui frame.

use nih_plug::prelude::*;
use nih_plug_egui::egui;
use parking_lot::Mutex;
use std::sync::Arc;

use crate::params::{ParamSnapshot, SlammerParams};
use crate::presets::{PresetEntry, PresetManager};
use crate::sequencer::Sequencer;
use crate::ui::theme;
use crate::ui::widgets::{draw_inset_display, preset_arrow_btn};

/// Mutable UI state for the preset bar.
struct PresetBarState {
    /// Currently loaded preset name.
    selected_name: String,
    /// Index of current preset in the full list (for arrow cycling).
    selected_index: usize,
    /// Whether the LED display is in edit mode (SAVE clicked).
    editing: bool,
    /// Text being edited in save mode.
    edit_buffer: String,
    /// Whether the preset dropdown is open.
    dropdown_open: bool,
    /// Status message shown briefly after save/delete/error.
    status_msg: String,
    status_timer: f32,
}

impl Default for PresetBarState {
    fn default() -> Self {
        Self {
            selected_name: "Init".into(),
            selected_index: 0,
            editing: false,
            edit_buffer: String::new(),
            dropdown_open: false,
            status_msg: String::new(),
            status_timer: 0.0,
        }
    }
}

/// Preset bar widget: owns its own state and a cached preset list.
pub struct PresetBar {
    state: PresetBarState,
    cached: Vec<PresetEntry>,
}

impl PresetBar {
    pub fn new(pm: &Arc<Mutex<PresetManager>>) -> Self {
        let cached = {
            let mut mgr = pm.lock();
            mgr.refresh();
            mgr.list_all()
        };
        Self {
            state: PresetBarState::default(),
            cached,
        }
    }

    fn reload(&mut self, pm: &Arc<Mutex<PresetManager>>) {
        let mut mgr = pm.lock();
        mgr.refresh();
        self.cached = mgr.list_all();
    }

    /// Point the selection at an entry by name (used when restoring the
    /// last-used preset on launch). Silently no-ops if the name isn't in
    /// the cache.
    pub fn select_by_name(&mut self, name: &str) {
        if let Some((idx, entry)) = self
            .cached
            .iter()
            .enumerate()
            .find(|(_, e)| e.name == name)
        {
            self.state.selected_index = idx;
            self.state.selected_name = entry.name.clone();
        }
    }

    /// Layout + event-handling for the preset bar inside the header strip.
    ///
    /// `header_center_y` is the vertical center of the header band;
    /// `origin_x` is the left edge where the bar should begin laying out.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        ui: &mut egui::Ui,
        setter: &ParamSetter,
        params: &SlammerParams,
        sequencer: &Sequencer,
        preset_manager: &Arc<Mutex<PresetManager>>,
        origin_x: f32,
        header_center_y: f32,
        dt_seconds: f32,
    ) {
        let display_w = 130.0;
        let display_h = 16.0;
        let arrow_size = 16.0;
        let btn_w = 36.0;
        let btn_h = 20.0;
        let preset_y = header_center_y - display_h * 0.5;

        // Divider line
        {
            let painter = ui.painter();
            painter.line_segment(
                [
                    egui::pos2(origin_x - 8.0, preset_y),
                    egui::pos2(origin_x - 8.0, preset_y + display_h),
                ],
                egui::Stroke::new(1.0, egui::Color32::from_rgb(0x28, 0x28, 0x28)),
            );
        }

        let selected_name = self.state.selected_name.clone();
        let is_editing = self.state.editing;
        let dropdown_open = self.state.dropdown_open;
        let is_factory = self
            .cached
            .iter()
            .any(|e| e.is_factory && e.name == selected_name);

        // --- Left arrow ---
        let left_rect = egui::Rect::from_min_size(
            egui::pos2(origin_x, header_center_y - arrow_size * 0.5),
            egui::vec2(arrow_size, arrow_size),
        );
        if !is_editing {
            let left_resp = ui.interact(
                left_rect,
                egui::Id::new("preset_prev"),
                egui::Sense::click(),
            );
            {
                let painter = ui.painter();
                let color = if left_resp.hovered() {
                    theme::WHITE
                } else {
                    theme::TEXT_DIM
                };
                preset_arrow_btn(painter, left_rect, "\u{25C2}", color);
            }
            if left_resp.clicked() && !self.cached.is_empty() {
                let idx = if self.state.selected_index == 0 {
                    self.cached.len() - 1
                } else {
                    self.state.selected_index - 1
                };
                let entry = self.cached[idx].clone();
                self.state.selected_index = idx;
                self.state.selected_name = entry.name.clone();
                entry.params.apply(setter, params, sequencer);
                crate::presets::save_last_preset_name(&entry.name);
            }
        } else {
            preset_arrow_btn(ui.painter(), left_rect, "\u{25C2}", theme::TEXT_GHOST);
        }

        // --- LED display ---
        let led_rect = egui::Rect::from_min_size(
            egui::pos2(origin_x + arrow_size + 2.0, preset_y),
            egui::vec2(display_w, display_h),
        );

        if is_editing {
            self.render_edit_mode(ui, setter, params, sequencer, preset_manager, led_rect);
        } else {
            self.render_led_display(ui, setter, params, led_rect, display_w, &selected_name);
            if dropdown_open {
                self.render_dropdown(
                    ui,
                    setter,
                    params,
                    sequencer,
                    led_rect,
                    display_w,
                    &selected_name,
                );
            }
        }

        // --- Right arrow ---
        let right_rect = egui::Rect::from_min_size(
            egui::pos2(led_rect.right() + 2.0, header_center_y - arrow_size * 0.5),
            egui::vec2(arrow_size, arrow_size),
        );
        if !is_editing {
            let right_resp = ui.interact(
                right_rect,
                egui::Id::new("preset_next"),
                egui::Sense::click(),
            );
            {
                let painter = ui.painter();
                let color = if right_resp.hovered() {
                    theme::WHITE
                } else {
                    theme::TEXT_DIM
                };
                preset_arrow_btn(painter, right_rect, "\u{25B8}", color);
            }
            if right_resp.clicked() && !self.cached.is_empty() {
                let idx = if self.state.selected_index >= self.cached.len() - 1 {
                    0
                } else {
                    self.state.selected_index + 1
                };
                let entry = self.cached[idx].clone();
                self.state.selected_index = idx;
                self.state.selected_name = entry.name.clone();
                entry.params.apply(setter, params, sequencer);
                crate::presets::save_last_preset_name(&entry.name);
            }
        } else {
            preset_arrow_btn(ui.painter(), right_rect, "\u{25B8}", theme::TEXT_GHOST);
        }

        // --- SAVE button ---
        let save_x = right_rect.right() + 6.0;
        let save_rect = egui::Rect::from_min_size(
            egui::pos2(save_x, header_center_y - btn_h * 0.5),
            egui::vec2(btn_w, btn_h),
        );
        let save_resp = ui.interact(
            save_rect,
            egui::Id::new("preset_save"),
            egui::Sense::click(),
        );
        draw_3d_button(
            ui.painter(),
            save_rect,
            "SAVE",
            save_resp.is_pointer_button_down_on() || is_editing,
        );
        if save_resp.clicked() {
            if self.state.editing {
                self.commit_save(preset_manager, params, sequencer);
            } else {
                self.state.editing = true;
                self.state.edit_buffer = self.state.selected_name.clone();
                self.state.dropdown_open = false;
            }
        }

        // --- DEL button ---
        let del_x = save_rect.right() + 4.0;
        let del_rect = egui::Rect::from_min_size(
            egui::pos2(del_x, header_center_y - btn_h * 0.5),
            egui::vec2(btn_w, btn_h),
        );
        // Every preset — factory or user — is now deletable. Factory
        // deletions are persisted as a hidden-names list by
        // `PresetManager::delete`; "Init" stays listed because it's
        // defined in the factory set and will reappear if the hidden
        // entry is cleared (e.g. by saving under that name).
        let _ = is_factory;
        let can_delete = !self.cached.is_empty();
        let del_resp = ui.interact(del_rect, egui::Id::new("preset_del"), egui::Sense::click());
        draw_3d_button(
            ui.painter(),
            del_rect,
            "DEL",
            del_resp.is_pointer_button_down_on() && can_delete,
        );
        if del_resp.clicked() && can_delete {
            let result = preset_manager.lock().delete(&selected_name);
            match result {
                Ok(()) => {
                    self.state.status_msg = format!("Deleted: {selected_name}");
                    self.state.status_timer = 2.0;
                    self.reload(preset_manager);
                    // Snap the selection to whatever's now at index 0
                    // (typically "Init", unless that was the one deleted).
                    if let Some(first) = self.cached.first().cloned() {
                        self.state.selected_index = 0;
                        self.state.selected_name = first.name;
                    } else {
                        self.state.selected_name.clear();
                    }
                }
                Err(e) => {
                    self.state.status_msg = e;
                    self.state.status_timer = 4.0;
                    self.reload(preset_manager);
                }
            }
        }

        // --- Keyboard shortcuts (when not editing) ---
        if !is_editing && !self.cached.is_empty() {
            let up = ui.input(|i| i.key_pressed(egui::Key::ArrowUp));
            let down = ui.input(|i| i.key_pressed(egui::Key::ArrowDown));
            if up || down {
                let idx = if up {
                    if self.state.selected_index == 0 {
                        self.cached.len() - 1
                    } else {
                        self.state.selected_index - 1
                    }
                } else if self.state.selected_index >= self.cached.len() - 1 {
                    0
                } else {
                    self.state.selected_index + 1
                };
                let entry = self.cached[idx].clone();
                self.state.selected_index = idx;
                self.state.selected_name = entry.name.clone();
                entry.params.apply(setter, params, sequencer);
                crate::presets::save_last_preset_name(&entry.name);
            }
        }

        // --- Status message (temporary) ---
        if self.state.status_timer > 0.0 {
            self.state.status_timer -= dt_seconds;
            let msg = self.state.status_msg.clone();
            ui.painter().text(
                egui::pos2(del_rect.right() + 8.0, header_center_y),
                egui::Align2::LEFT_CENTER,
                &msg,
                egui::FontId::new(6.0, egui::FontFamily::Monospace),
                theme::RED_LED,
            );
        }
    }

    fn render_edit_mode(
        &mut self,
        ui: &mut egui::Ui,
        _setter: &ParamSetter,
        params: &SlammerParams,
        sequencer: &Sequencer,
        preset_manager: &Arc<Mutex<PresetManager>>,
        led_rect: egui::Rect,
    ) {
        {
            let painter = ui.painter();
            painter.rect_filled(led_rect, 0.0, theme::BG_DISPLAY);
            painter.rect_stroke(
                led_rect,
                0.0,
                egui::Stroke::new(2.0, theme::RED_LED),
                egui::StrokeKind::Outside,
            );
        }
        let mut edit_buf = self.state.edit_buffer.clone();
        let input_rect = led_rect.shrink(2.0);
        let te_resp = ui
            .allocate_new_ui(egui::UiBuilder::new().max_rect(input_rect), |ui| {
                let te = egui::TextEdit::singleline(&mut edit_buf)
                    .font(egui::FontId::new(
                        11.0,
                        egui::FontFamily::Name(theme::FONT_DIGITAL.into()),
                    ))
                    .text_color(theme::RED_LED)
                    .frame(false)
                    .desired_width(input_rect.width());
                let resp = ui.add(te);
                if !resp.has_focus() {
                    resp.request_focus();
                }
                resp
            })
            .inner;

        self.state.edit_buffer = edit_buf.clone();

        // Enter commits via the same `commit_save` path used by the SAVE
        // button so both entry points behave identically. We check the
        // Enter key unconditionally (not gated on `lost_focus()`) because
        // the focus-request step above re-steals focus in the same frame
        // the TextEdit tried to release it, which made the old
        // `lost_focus() && Enter` check fire inconsistently.
        let _ = te_resp;
        let (enter_pressed, esc_pressed) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::Enter),
                i.key_pressed(egui::Key::Escape),
            )
        });
        if enter_pressed {
            self.commit_save(preset_manager, params, sequencer);
        } else if esc_pressed {
            self.state.editing = false;
        }
    }

    fn render_led_display(
        &mut self,
        ui: &mut egui::Ui,
        _setter: &ParamSetter,
        _params: &SlammerParams,
        led_rect: egui::Rect,
        display_w: f32,
        selected_name: &str,
    ) {
        {
            let painter = ui.painter();
            draw_inset_display(
                painter,
                led_rect.left(),
                led_rect.top(),
                display_w,
                led_rect.height(),
            );
            painter.text(
                egui::pos2(led_rect.center().x, led_rect.center().y),
                egui::Align2::CENTER_CENTER,
                selected_name,
                egui::FontId::new(11.0, egui::FontFamily::Name(theme::FONT_DIGITAL.into())),
                theme::RED_LED,
            );
        }
        let led_resp = ui.interact(led_rect, egui::Id::new("preset_led"), egui::Sense::click());
        if led_resp.clicked() {
            self.state.dropdown_open = !self.state.dropdown_open;
        }
    }

    fn render_dropdown(
        &mut self,
        ui: &mut egui::Ui,
        setter: &ParamSetter,
        params: &SlammerParams,
        sequencer: &Sequencer,
        led_rect: egui::Rect,
        display_w: f32,
        selected_name: &str,
    ) {
        let dd_width = display_w.max(160.0);
        let dd_max_h = 200.0;
        let dd_item_h = 18.0;
        let dd_h = (self.cached.len() as f32 * dd_item_h + 6.0).min(dd_max_h);
        let dd_rect = egui::Rect::from_min_size(
            egui::pos2(led_rect.left(), led_rect.bottom() + 2.0),
            egui::vec2(dd_width, dd_h),
        );

        let fg_painter = ui.painter().clone().with_layer_id(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("preset_dropdown_layer"),
        ));
        fg_painter.rect_filled(dd_rect, 2.0, egui::Color32::from_rgb(0x12, 0x12, 0x12));
        fg_painter.rect_stroke(
            dd_rect,
            2.0,
            egui::Stroke::new(1.0, egui::Color32::from_rgb(0x33, 0x33, 0x33)),
            egui::StrokeKind::Outside,
        );

        let factory_count = self.cached.iter().filter(|e| e.is_factory).count();
        let mut item_y = dd_rect.top() + 3.0;
        let mut clicked: Option<(usize, PresetEntry)> = None;

        for (idx, entry) in self.cached.iter().enumerate() {
            if !entry.is_factory && idx == factory_count {
                fg_painter.line_segment(
                    [
                        egui::pos2(dd_rect.left() + 6.0, item_y),
                        egui::pos2(dd_rect.right() - 6.0, item_y),
                    ],
                    egui::Stroke::new(1.0, egui::Color32::from_rgb(0x33, 0x33, 0x33)),
                );
                item_y += 3.0;
            }

            let item_rect = egui::Rect::from_min_size(
                egui::pos2(dd_rect.left(), item_y),
                egui::vec2(dd_width, dd_item_h),
            );

            let label = if entry.is_factory {
                entry.name.clone()
            } else {
                format!("* {}", entry.name)
            };

            let item_resp = ui.interact(
                item_rect,
                egui::Id::new(format!("dd_{idx}")),
                egui::Sense::click(),
            );
            let is_sel = entry.name == selected_name;
            let color = if is_sel || item_resp.hovered() {
                theme::RED_LED
            } else if entry.is_factory {
                egui::Color32::from_rgb(0x88, 0x88, 0x88)
            } else {
                egui::Color32::from_rgb(0xaa, 0xaa, 0xaa)
            };

            if item_resp.hovered() {
                fg_painter.rect_filled(item_rect, 0.0, egui::Color32::from_rgb(0x1e, 0x1e, 0x1e));
            }

            fg_painter.text(
                egui::pos2(item_rect.left() + 10.0, item_rect.center().y),
                egui::Align2::LEFT_CENTER,
                &label,
                egui::FontId::new(11.0, egui::FontFamily::Name(theme::FONT_DIGITAL.into())),
                color,
            );

            if item_resp.clicked() {
                clicked = Some((idx, entry.clone()));
            }

            item_y += dd_item_h;
        }

        if let Some((idx, entry)) = clicked {
            self.state.selected_name = entry.name.clone();
            self.state.selected_index = idx;
            self.state.dropdown_open = false;
            entry.params.apply(setter, params, sequencer);
            crate::presets::save_last_preset_name(&entry.name);
        }

        // Close dropdown if clicking outside the dropdown rect
        if ui.input(|i| i.pointer.any_click()) {
            if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                if !dd_rect.contains(pos) && !led_rect.contains(pos) {
                    self.state.dropdown_open = false;
                }
            }
        }
    }

    fn commit_save(
        &mut self,
        preset_manager: &Arc<Mutex<PresetManager>>,
        params: &SlammerParams,
        sequencer: &Sequencer,
    ) {
        let name = self.state.edit_buffer.trim().to_owned();
        if !name.is_empty() {
            let snapshot = ParamSnapshot::capture(params, sequencer);
            let result = preset_manager.lock().save(&name, snapshot);
            match result {
                Ok(()) => {
                    self.state.selected_name = name.clone();
                    self.state.status_msg = format!("Saved: {name}");
                    self.state.status_timer = 3.0;
                    crate::presets::save_last_preset_name(&name);
                }
                Err(e) => {
                    self.state.status_msg = e;
                    self.state.status_timer = 4.0;
                }
            }
            self.reload(preset_manager);
        }
        self.state.editing = false;
    }
}

fn draw_3d_button(painter: &egui::Painter, rect: egui::Rect, label: &str, pressed: bool) {
    let top_color = if pressed {
        theme::BTN_DARK
    } else {
        theme::BTN_LIGHT
    };
    let bot_color = if pressed {
        theme::BTN_LIGHT
    } else {
        theme::BTN_DARK
    };
    painter.rect_filled(rect, 2.0, bot_color);
    painter.rect_filled(
        egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), rect.height() * 0.45)),
        2.0,
        top_color,
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::new(11.0, egui::FontFamily::Monospace),
        theme::WHITE,
    );
}
