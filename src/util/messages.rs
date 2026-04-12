//! Lock-free UI → audio message queue.
//!
//! The editor runs on its own thread and must not block the audio thread
//! when it wants to tell the engine to do something (trigger a test kick,
//! reset a voice, etc.). A single-producer / single-consumer `rtrb` ring
//! buffer gives us that: the UI pushes `UiToDsp` messages, `process()`
//! drains them at the top of the block with `try_pop()` and never blocks.
//!
//! Today the only message is a test trigger from the on-screen button and
//! the `T` keyboard shortcut, but this infrastructure leaves room to add
//! more (manual voice reset, engine reseed, etc.) without reintroducing
//! another ad-hoc `AtomicBool`.

use rtrb::{Consumer, Producer, RingBuffer};

/// Capacity of the UI → DSP queue.
///
/// A handful of slots is plenty: even frenetic keyboard-bashing generates
/// orders of magnitude fewer events per block than samples, and any
/// overflow is dropped silently (the user just doesn't hear that one
/// trigger).
const QUEUE_CAPACITY: usize = 32;

/// Commands the UI thread can send to the audio thread.
#[derive(Clone, Copy, Debug)]
pub enum UiToDsp {
    /// Fire the engine as if a NoteOn arrived.
    Trigger,
}

/// Allocate a fresh UI → DSP channel. Call exactly once per plugin instance
/// (in the `Plugin` constructor).
pub fn channel() -> (Producer<UiToDsp>, Consumer<UiToDsp>) {
    RingBuffer::new(QUEUE_CAPACITY)
}
