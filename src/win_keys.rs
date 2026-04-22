//! Windows-only keyboard-shortcut fallback for the standalone path.
//!
//! nih-plug's standalone wrapper opens an outer baseview window whose
//! `WindowHandler::on_event` returns `EventStatus::Ignored` for everything,
//! including `Event::Keyboard`. Windows routes `WM_KEYDOWN` to that outer
//! window (it has focus) — so the egui *child* window never sees keyboard
//! events, even with the RustAudio/baseview#212 hook in place. (The hook
//! just re-dispatches messages into the same ignore-everything handler.)
//!
//! We don't want to fork and patch nih-plug, so instead the editor polls
//! a tiny set of keys (T = trigger, Space = play/stop) via
//! `GetAsyncKeyState` each frame and OR's the result into egui's normal
//! `key_pressed` detection. On plugin hosts and Linux/macOS the egui path
//! works; this helper is a *fallback* that only fires when (a) we're on
//! Windows, and (b) our GUI thread owns the foreground window.
//!
//! The foreground-thread gate is important: `GetAsyncKeyState` reports
//! global keyboard state. Without the gate, a T press in another app
//! (browser, chat) would trigger a kick while Slammer is in the
//! background.

use std::sync::atomic::{AtomicBool, Ordering};

use winapi::um::processthreadsapi::GetCurrentThreadId;
use winapi::um::winuser::{GetAsyncKeyState, GetForegroundWindow, GetWindowThreadProcessId};

pub const VK_T: i32 = 0x54;
pub const VK_SPACE: i32 = 0x20;

/// Returns true only on the frame the given virtual-key transitions from
/// up to down, and only when the foreground window belongs to this thread.
///
/// `prev_down` is caller-owned state (a `static AtomicBool`) so the helper
/// stays a free function with no singleton.
pub fn just_pressed(vk: i32, prev_down: &AtomicBool) -> bool {
    if !foreground_is_ours() {
        prev_down.store(false, Ordering::Relaxed);
        return false;
    }
    let is_down = unsafe { GetAsyncKeyState(vk) as u16 & 0x8000 != 0 };
    let was_down = prev_down.swap(is_down, Ordering::Relaxed);
    is_down && !was_down
}

fn foreground_is_ours() -> bool {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_null() {
        return false;
    }
    let mut pid = 0u32;
    let fg_tid = unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    fg_tid == unsafe { GetCurrentThreadId() }
}
