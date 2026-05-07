//! In-plugin MIDI Learn — maps incoming MIDI CCs *and* note-on events to
//! plugin parameters, independent of the host's mapping system.
//!
//! Why this exists: not every host (or every controller setup) makes
//! parameter mapping easy. Bitwig has built-in controller scripts but they
//! assume specific encoder layouts; Renoise's mapping path is awkward; the
//! standalone has no host at all. A right-click → MIDI Learn affordance on
//! every knob lets the user bind whatever controller they have, no host
//! configuration required.
//!
//! The Arturia BeatStep encoder is the motivating case: stock factory layout
//! sends note events for some controls and CCs for others, and users
//! reconfigure freely via Arturia MIDI Control Center. Capturing both event
//! types means MIDI Learn just works regardless of how the controller was
//! programmed.
//!
//! Architecture:
//!
//! * **Audio thread** does the bare minimum — drops the MIDI event into a
//!   single SPSC ring (`MidiInputEvent`) without locking anything. Before
//!   triggering the engine on a `NoteOn`, it consults a lock-free
//!   [`NoteBlockMap`] (one bit per `(channel, note)`) and skips the trigger
//!   when that note has already been bound to a knob — so a learned encoder
//!   doesn't fire a kick on every detent.
//! * **GUI thread** drains the ring once per frame, consults the
//!   user-editable [`MidiMapState`] (also acts as the LEARN target), and
//!   pushes the resulting parameter changes back through `ParamSetter` so
//!   the host sees them as automation writes. It also keeps the
//!   `NoteBlockMap` in sync with the active note bindings.
//!
//! `MidiMapState` is persisted via nih-plug's `#[persist]` mechanism on
//! [`crate::params::NinerParams`] so bindings survive DAW project save/load
//! and standalone session restarts.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

/// Channel sentinel meaning "any channel". Stored as 16 (MIDI channels are
/// 0-15) so it round-trips through serde without a custom enum.
pub const OMNI: u8 = 16;

/// One MIDI input event delivered from the audio thread to the GUI thread
/// via an `rtrb` SPSC queue. Carries enough data for both the MIDI Learn
/// capture path and the apply-to-bound-param path.
#[derive(Clone, Copy, Debug)]
pub enum MidiInputEvent {
    /// Continuous controller change.
    Cc {
        /// 0..=15.
        channel: u8,
        /// 0..=127.
        cc: u8,
        /// Already normalized to 0.0..=1.0 (the raw 0..=127 byte divided by 127).
        value: f32,
    },
    /// Note-on. The audio thread only enqueues note-ons it has *not*
    /// triggered the engine for (i.e. ones that may be bound to a knob);
    /// see `NoteBlockMap::is_blocked`.
    NoteOn {
        channel: u8,
        note: u8,
        /// Velocity normalized to 0.0..=1.0.
        velocity: f32,
    },
}

/// Discriminator for what kind of MIDI event a binding listens for.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MidiSource {
    /// Continuous controller (encoder/slider/pedal).
    Cc(u8),
    /// Note-on (pad, key, encoder-as-note). Treated as a continuous control:
    /// param value = velocity / 127. Releases (note-off) are ignored — they
    /// would just snap the knob back to zero, which is rarely what you want.
    NoteOn(u8),
}

/// How a CC binding interprets incoming values.
///
/// Most absolute pots / sliders / mod wheels send the raw 0..=127 value as
/// the desired parameter setting (`Absolute`). Endless rotary encoders
/// instead send signed deltas, in one of two common encodings — picked
/// per controller, often configurable in the controller's own editor app.
/// Auto-detected at bind time from the captured value's signature (1/127
/// vs 63/65) and overridable from the right-click menu.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum CcEncoding {
    /// `0..=127` maps directly to the parameter's normalized value.
    /// What every fader/pot does.
    #[default]
    Absolute,
    /// Sign-magnitude / "binary offset" relative encoder
    /// (Arturia BeatStep MK1 SEQ mode, BCR2000 default, FaderFox).
    /// `0x01` = +1, `0x7F` = -1, `0x3F` = +63, `0x41` = -63.
    BinaryOffset,
    /// 2's-complement-around-64 / "centred" relative encoder
    /// (Pioneer DDJ rotaries, Behringer X-Touch in some modes).
    /// `0x40` = no-op, `0x41` = +1, `0x3F` = -1, `0x4F` = +15.
    Centered,
}

/// One persisted binding — a single (channel, source) → param mapping.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MidiBinding {
    /// 0..=15 for a specific channel, [`OMNI`] for any channel.
    pub channel: u8,
    /// Whether this binding listens for a CC or a note-on.
    pub source: MidiSource,
    /// Param's `#[id = "..."]` string from the `Params` derive, or one of
    /// the [`sentinel`] strings (`__niner_tempo`, `__niner_seq_play`) for
    /// non-`Param` targets like the standalone tempo knob.
    pub param_id: String,
    /// How to interpret incoming CC values for this binding. Auto-detected
    /// at bind time and overridable from the right-click menu. Ignored for
    /// `MidiSource::NoteOn` (notes always map velocity → value).
    #[serde(default)]
    pub encoding: CcEncoding,
}

/// Sentinel `param_id` strings for non-`Param` MIDI Learn targets
/// (standalone tempo knob, sequencer play toggle, etc.). The apply path
/// in `editor.rs` short-circuits these before doing the normal
/// `id_to_ptr` lookup. Persisted by string so backwards-compat is free.
pub mod sentinel {
    /// Bind to the standalone sequencer tempo (no-op in DAW-host-synced mode).
    pub const TEMPO: &str = "__niner_tempo";
    /// Toggle the standalone sequencer's running flag (rising-edge only).
    pub const SEQ_PLAY: &str = "__niner_seq_play";
}

/// Decode a normalized CC value (`0.0..=1.0`) into a signed step count
/// using the encoding the binding was learned under. Returns 0 for the
/// "no movement" sentinel of each scheme so callers can short-circuit.
///
/// `Absolute` encoding never goes through this function (the caller uses
/// `value` directly), so we just return 0 for it as a defensive no-op.
pub fn decode_relative_delta(value_normalized: f32, encoding: CcEncoding) -> i32 {
    let raw = (value_normalized * 127.0).round() as i32;
    match encoding {
        CcEncoding::Absolute => 0,
        // Sign-magnitude: 0x01..0x3F = +1..+63, 0x41..0x7F = -63..-1.
        // 0x00 and 0x40 are both "no movement" by convention.
        CcEncoding::BinaryOffset => {
            if raw == 0 || raw == 64 {
                0
            } else if raw < 64 {
                raw
            } else {
                raw - 128
            }
        }
        // 2's-complement-around-64: 0x40 = 0, 0x41 = +1, 0x3F = -1.
        CcEncoding::Centered => raw - 64,
    }
}

/// Auto-classify a CC encoding from the first value captured at bind
/// time. Single-step values are the only reliable signal:
/// - `1` or `127` → `BinaryOffset` (sign-magnitude relative)
/// - `63` or `65` → `Centered` (2's-complement around 64)
/// - anything else → `Absolute`
///
/// The user can override from the right-click menu if the heuristic
/// guesses wrong (e.g. an absolute pot whose first move happened to land
/// on `1` at the bottom of its travel).
pub fn detect_cc_encoding(value_normalized: f32) -> CcEncoding {
    let raw = (value_normalized * 127.0).round() as i32;
    match raw {
        1 | 127 => CcEncoding::BinaryOffset,
        63 | 65 => CcEncoding::Centered,
        _ => CcEncoding::Absolute,
    }
}

/// Persisted MIDI Learn state. Stored on `NinerParams` via `#[persist]`.
///
/// `Vec` rather than `BTreeMap` keeps serde JSON output simple (no custom
/// key serialization) and avoids ordering surprises on round-trip.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MidiMapState {
    pub bindings: Vec<MidiBinding>,
}

impl MidiMapState {
    /// Find the binding for (channel, source), preferring an exact channel
    /// match over an `OMNI` fallback. Returns `(param_id, encoding)` so
    /// the caller can decide how to interpret the value.
    pub fn lookup(&self, channel: u8, source: MidiSource) -> Option<(&str, CcEncoding)> {
        let mut omni_match: Option<&MidiBinding> = None;
        for b in &self.bindings {
            if b.source != source {
                continue;
            }
            if b.channel == channel {
                return Some((b.param_id.as_str(), b.encoding));
            }
            if b.channel == OMNI && omni_match.is_none() {
                omni_match = Some(b);
            }
        }
        omni_match.map(|b| (b.param_id.as_str(), b.encoding))
    }

    /// Look up the binding currently in place for `param_id`. Returns the
    /// full `(channel, source, encoding)` triple so the right-click menu
    /// can show all three.
    pub fn binding_for_param(&self, param_id: &str) -> Option<(u8, MidiSource, CcEncoding)> {
        self.bindings
            .iter()
            .find(|b| b.param_id == param_id)
            .map(|b| (b.channel, b.source, b.encoding))
    }

    /// Bind `(channel, source) → param_id`, replacing any prior binding
    /// either for that exact source or for the named param. Each param has
    /// at most one binding; each (channel, source) hits at most one param.
    pub fn bind(
        &mut self,
        channel: u8,
        source: MidiSource,
        param_id: &str,
        encoding: CcEncoding,
    ) {
        self.bindings.retain(|b| {
            !((b.channel == channel && b.source == source) || b.param_id == param_id)
        });
        self.bindings.push(MidiBinding {
            channel,
            source,
            param_id: param_id.to_string(),
            encoding,
        });
    }

    /// Replace the encoding mode on the binding for `param_id`. Returns
    /// the new value, or `None` if the param has no binding. Used by the
    /// right-click menu's encoding picker when the auto-detect heuristic
    /// gets it wrong.
    pub fn set_encoding(&mut self, param_id: &str, encoding: CcEncoding) -> Option<CcEncoding> {
        for b in &mut self.bindings {
            if b.param_id == param_id {
                b.encoding = encoding;
                return Some(b.encoding);
            }
        }
        None
    }

    /// Drop any binding for `param_id`. Returns the previously-bound
    /// (channel, source) if anything was removed, so the caller can update
    /// out-of-band runtime state (e.g. `NoteBlockMap`) accordingly.
    pub fn forget(&mut self, param_id: &str) -> Option<(u8, MidiSource)> {
        if let Some(idx) = self.bindings.iter().position(|b| b.param_id == param_id) {
            let removed = self.bindings.swap_remove(idx);
            Some((removed.channel, removed.source))
        } else {
            None
        }
    }

    /// Iterator over every (channel, note) currently bound — used to
    /// reconstruct [`NoteBlockMap`] from a freshly deserialised
    /// `MidiMapState`.
    pub fn bound_notes(&self) -> impl Iterator<Item = (u8, u8)> + '_ {
        self.bindings.iter().filter_map(|b| match b.source {
            MidiSource::NoteOn(note) => Some((b.channel, note)),
            MidiSource::Cc(_) => None,
        })
    }
}

/// Lock-free `(channel, note) → bound?` lookup, shared between the audio
/// thread (read-only) and the GUI thread (read/write).
///
/// 16 channels × 128 notes = 2048 bits, packed into 32 `AtomicU64`s. Both
/// sides use relaxed ordering — bindings are user-driven (knob-rate),
/// not hot data, so we don't need acquire/release fences.
///
/// `OMNI` (channel 16) is represented by setting *all* channel slots for
/// the given note: a single bit-test in the audio thread covers both the
/// "exact channel" and "any channel" cases without branching on `OMNI`.
pub struct NoteBlockMap {
    bits: [AtomicU64; 32],
}

impl Default for NoteBlockMap {
    fn default() -> Self {
        // `AtomicU64` is not `Copy`, so `[AtomicU64::new(0); 32]` won't
        // compile. Build the array explicitly.
        Self {
            bits: std::array::from_fn(|_| AtomicU64::new(0)),
        }
    }
}

impl NoteBlockMap {
    pub fn new() -> Self {
        Self::default()
    }

    fn slot(channel: u8, note: u8) -> Option<(usize, u64)> {
        if channel >= 16 || note >= 128 {
            return None;
        }
        let idx = (channel as usize) * 128 + (note as usize);
        Some((idx / 64, 1u64 << (idx % 64)))
    }

    /// Audio-thread fast path: is this `(channel, note)` claimed by a
    /// MIDI Learn binding? Single relaxed atomic load + bit test.
    pub fn is_blocked(&self, channel: u8, note: u8) -> bool {
        match Self::slot(channel, note) {
            Some((word, mask)) => self.bits[word].load(Ordering::Relaxed) & mask != 0,
            None => false,
        }
    }

    /// GUI-thread call: mark a `(channel, note)` as bound. `channel == OMNI`
    /// blocks the note across all 16 channels.
    pub fn block(&self, channel: u8, note: u8) {
        if channel == OMNI {
            for ch in 0..16u8 {
                self.set_bit(ch, note, true);
            }
        } else {
            self.set_bit(channel, note, true);
        }
    }

    /// GUI-thread call: clear a `(channel, note)` block.
    pub fn unblock(&self, channel: u8, note: u8) {
        if channel == OMNI {
            for ch in 0..16u8 {
                self.set_bit(ch, note, false);
            }
        } else {
            self.set_bit(channel, note, false);
        }
    }

    fn set_bit(&self, channel: u8, note: u8, on: bool) {
        if let Some((word, mask)) = Self::slot(channel, note) {
            if on {
                self.bits[word].fetch_or(mask, Ordering::Relaxed);
            } else {
                self.bits[word].fetch_and(!mask, Ordering::Relaxed);
            }
        }
    }

    /// Replace the entire block set with the notes currently bound in
    /// `state`. Used at startup to restore from the persisted
    /// `MidiMapState` and after bulk operations.
    pub fn rebuild_from(&self, state: &MidiMapState) {
        for w in &self.bits {
            w.store(0, Ordering::Relaxed);
        }
        for (channel, note) in state.bound_notes() {
            self.block(channel, note);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_prefers_exact_channel_over_omni() {
        let mut s = MidiMapState::default();
        s.bind(OMNI, MidiSource::Cc(74), "decay", CcEncoding::Absolute);
        s.bind(0, MidiSource::Cc(74), "drift", CcEncoding::Absolute);
        assert_eq!(
            s.lookup(0, MidiSource::Cc(74)),
            Some(("drift", CcEncoding::Absolute))
        );
        assert_eq!(
            s.lookup(5, MidiSource::Cc(74)),
            Some(("decay", CcEncoding::Absolute))
        ); // omni fallback
        assert_eq!(s.lookup(0, MidiSource::Cc(99)), None);
    }

    #[test]
    fn cc_and_noteon_are_distinct_sources() {
        let mut s = MidiMapState::default();
        s.bind(0, MidiSource::Cc(60), "decay", CcEncoding::Absolute);
        s.bind(0, MidiSource::NoteOn(60), "drift", CcEncoding::Absolute);
        // Same channel and same number, different kind — both must coexist.
        assert_eq!(s.bindings.len(), 2);
        assert_eq!(
            s.lookup(0, MidiSource::Cc(60)),
            Some(("decay", CcEncoding::Absolute))
        );
        assert_eq!(
            s.lookup(0, MidiSource::NoteOn(60)),
            Some(("drift", CcEncoding::Absolute))
        );
    }

    #[test]
    fn bind_replaces_existing_binding_for_same_param() {
        let mut s = MidiMapState::default();
        s.bind(0, MidiSource::Cc(74), "decay", CcEncoding::Absolute);
        s.bind(0, MidiSource::Cc(75), "decay", CcEncoding::BinaryOffset);
        assert_eq!(s.bindings.len(), 1);
        assert_eq!(s.bindings[0].source, MidiSource::Cc(75));
        assert_eq!(s.bindings[0].encoding, CcEncoding::BinaryOffset);
    }

    #[test]
    fn bind_replaces_existing_binding_for_same_source() {
        let mut s = MidiMapState::default();
        s.bind(0, MidiSource::Cc(74), "decay", CcEncoding::Absolute);
        s.bind(0, MidiSource::Cc(74), "drift", CcEncoding::Absolute);
        assert_eq!(s.bindings.len(), 1);
        assert_eq!(s.bindings[0].param_id, "drift");
    }

    #[test]
    fn forget_returns_removed_source() {
        let mut s = MidiMapState::default();
        s.bind(0, MidiSource::NoteOn(36), "decay", CcEncoding::Absolute);
        s.bind(0, MidiSource::Cc(75), "drift", CcEncoding::Absolute);
        assert_eq!(s.forget("decay"), Some((0, MidiSource::NoteOn(36))));
        assert_eq!(s.forget("decay"), None); // already gone
        assert_eq!(s.bindings.len(), 1);
        assert_eq!(s.bindings[0].param_id, "drift");
    }

    #[test]
    fn binding_for_param_round_trip() {
        let mut s = MidiMapState::default();
        s.bind(7, MidiSource::Cc(42), "decay", CcEncoding::Centered);
        assert_eq!(
            s.binding_for_param("decay"),
            Some((7, MidiSource::Cc(42), CcEncoding::Centered))
        );
        assert_eq!(s.binding_for_param("nope"), None);
    }

    #[test]
    fn set_encoding_replaces_value() {
        let mut s = MidiMapState::default();
        s.bind(0, MidiSource::Cc(74), "decay", CcEncoding::Absolute);
        assert_eq!(
            s.set_encoding("decay", CcEncoding::BinaryOffset),
            Some(CcEncoding::BinaryOffset)
        );
        assert_eq!(s.bindings[0].encoding, CcEncoding::BinaryOffset);
        assert_eq!(s.set_encoding("nope", CcEncoding::Absolute), None);
    }

    #[test]
    fn decode_binary_offset() {
        // 0x01 = +1, 0x7F = -1, 0x3F = +63, 0x41 = -63, 0x40 / 0x00 = no-op.
        let bo = CcEncoding::BinaryOffset;
        assert_eq!(decode_relative_delta(1.0 / 127.0, bo), 1);
        assert_eq!(decode_relative_delta(127.0 / 127.0, bo), -1);
        assert_eq!(decode_relative_delta(63.0 / 127.0, bo), 63);
        assert_eq!(decode_relative_delta(65.0 / 127.0, bo), -63);
        assert_eq!(decode_relative_delta(64.0 / 127.0, bo), 0);
        assert_eq!(decode_relative_delta(0.0, bo), 0);
    }

    #[test]
    fn decode_centered() {
        // 0x40 = 0, 0x41 = +1, 0x3F = -1, 0x4F = +15, 0x31 = -15.
        let c = CcEncoding::Centered;
        assert_eq!(decode_relative_delta(64.0 / 127.0, c), 0);
        assert_eq!(decode_relative_delta(65.0 / 127.0, c), 1);
        assert_eq!(decode_relative_delta(63.0 / 127.0, c), -1);
        assert_eq!(decode_relative_delta(79.0 / 127.0, c), 15);
        assert_eq!(decode_relative_delta(49.0 / 127.0, c), -15);
    }

    #[test]
    fn detect_cc_encoding_picks_right_kind() {
        // First-tick signatures.
        assert_eq!(
            detect_cc_encoding(1.0 / 127.0),
            CcEncoding::BinaryOffset
        );
        assert_eq!(
            detect_cc_encoding(127.0 / 127.0),
            CcEncoding::BinaryOffset
        );
        assert_eq!(detect_cc_encoding(63.0 / 127.0), CcEncoding::Centered);
        assert_eq!(detect_cc_encoding(65.0 / 127.0), CcEncoding::Centered);
        // Anything mid-range (e.g. an absolute pot at ~50%) -> absolute.
        assert_eq!(detect_cc_encoding(0.5), CcEncoding::Absolute);
        assert_eq!(detect_cc_encoding(0.0), CcEncoding::Absolute);
    }

    #[test]
    fn serde_round_trip_mixed_sources() {
        let mut s = MidiMapState::default();
        s.bind(0, MidiSource::Cc(74), "decay", CcEncoding::BinaryOffset);
        s.bind(OMNI, MidiSource::NoteOn(7), "master_vol", CcEncoding::Absolute);
        let json = serde_json::to_string(&s).unwrap();
        let back: MidiMapState = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn legacy_binding_without_encoding_field_loads_as_absolute() {
        // Persisted state from a build before `encoding` existed (or with
        // the old `relative: bool` shape) must deserialize without error
        // and default the new field. Unknown extra fields are ignored.
        let json =
            r#"{"bindings":[{"channel":0,"source":{"Cc":74},"param_id":"decay","relative":true}]}"#;
        let s: MidiMapState = serde_json::from_str(json).unwrap();
        assert_eq!(s.bindings.len(), 1);
        assert_eq!(s.bindings[0].encoding, CcEncoding::Absolute);
    }

    #[test]
    fn legacy_state_loads_with_empty_bindings() {
        // A pre-MIDI-learn build's persist field deserializes from `{}` —
        // serde(default) must yield an empty Vec, not an error.
        let s: MidiMapState = serde_json::from_str("{}").unwrap();
        assert!(s.bindings.is_empty());
    }

    #[test]
    fn note_block_map_round_trip() {
        let map = NoteBlockMap::new();
        assert!(!map.is_blocked(0, 36));
        map.block(0, 36);
        assert!(map.is_blocked(0, 36));
        assert!(!map.is_blocked(1, 36));
        map.unblock(0, 36);
        assert!(!map.is_blocked(0, 36));
    }

    #[test]
    fn note_block_map_omni_covers_all_channels() {
        let map = NoteBlockMap::new();
        map.block(OMNI, 60);
        for ch in 0..16u8 {
            assert!(map.is_blocked(ch, 60));
        }
        map.unblock(OMNI, 60);
        for ch in 0..16u8 {
            assert!(!map.is_blocked(ch, 60));
        }
    }

    #[test]
    fn note_block_map_rebuild_from_state() {
        let mut s = MidiMapState::default();
        s.bind(0, MidiSource::NoteOn(36), "decay", CcEncoding::Absolute);
        s.bind(3, MidiSource::Cc(74), "drift", CcEncoding::Absolute); // CC binding shouldn't block notes
        s.bind(OMNI, MidiSource::NoteOn(60), "master_vol", CcEncoding::Absolute);

        let map = NoteBlockMap::new();
        map.rebuild_from(&s);

        assert!(map.is_blocked(0, 36));
        assert!(!map.is_blocked(0, 74)); // CCs not in the note map
        for ch in 0..16u8 {
            assert!(map.is_blocked(ch, 60));
        }
    }

    #[test]
    fn note_block_map_ignores_out_of_range() {
        let map = NoteBlockMap::new();
        // No panic; just ignored.
        map.block(99, 200);
        assert!(!map.is_blocked(99, 200));
    }
}
