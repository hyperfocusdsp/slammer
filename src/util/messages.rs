//! Lock-free UI → audio message queue, plus its DSP → UI counterpart for
//! MIDI Learn.
//!
//! The editor runs on its own thread and must not block the audio thread
//! when it wants to tell the engine to do something (trigger a test kick,
//! reset a voice, etc.). A single-producer / single-consumer `rtrb` ring
//! buffer gives us that: the UI pushes `UiToDsp` messages, `process()`
//! drains them at the top of the block with `try_pop()` and never blocks.
//!
//! The DSP → UI direction is symmetrical and used to forward incoming
//! MIDI events (CC and selectively NoteOn) from `process()` into the
//! editor's MIDI Learn / mapping pipeline (see [`crate::midi_map`]).

use crate::midi_map::MidiInputEvent;
use rtrb::{Consumer, Producer, RingBuffer};

/// Capacity of the UI → DSP queue.
///
/// A handful of slots is plenty: even frenetic keyboard-bashing generates
/// orders of magnitude fewer events per block than samples, and any
/// overflow is dropped silently (the user just doesn't hear that one
/// trigger).
const QUEUE_CAPACITY: usize = 32;

/// Capacity of the DSP → UI MIDI event queue. Sized for sustained streams
/// from a knob-twisty controller (BeatStep, BCR2000) — at typical 60 fps
/// editor refresh and ~50 events/sec/knob the queue never fills, but a
/// bigger buffer keeps us safe under load. Overflow drops the new event
/// silently; MIDI Learn isn't latency-sensitive, but losing the latest
/// value is the less surprising failure mode than displacing an in-flight
/// one.
const MIDI_EVENT_QUEUE_CAPACITY: usize = 256;

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

/// Allocate a fresh DSP → UI MIDI event channel. Call exactly once per
/// plugin instance.
pub fn midi_event_channel() -> (Producer<MidiInputEvent>, Consumer<MidiInputEvent>) {
    RingBuffer::new(MIDI_EVENT_QUEUE_CAPACITY)
}
