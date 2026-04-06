//! Audio-thread → GUI waveform telemetry via an rtrb ring buffer.
//!
//! The audio thread pushes peak values each `process()` call; the GUI drains
//! them to render the waveform display. The producer tracks dropped pushes
//! so the UI / logs can surface buffer starvation.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

const RING_SIZE: usize = 4096;

/// Lock-free meter state shared between the audio and GUI threads.
///
/// Currently carries the per-buffer maximum gain-reduction (dB, as a positive
/// number) reported by the master-bus compressor. Stored as the bit pattern of
/// a non-negative `f32` so we can round-trip it through an `AtomicU32` without
/// any synchronisation heavier than a relaxed store/load.
pub struct MeterShared {
    gr_db_bits: AtomicU32,
}

impl MeterShared {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            gr_db_bits: AtomicU32::new(0),
        })
    }

    /// Audio thread: publish the max GR seen during the current buffer.
    #[inline]
    pub fn store_gr_db(&self, gr_db: f32) {
        // `gr_db` is always a non-negative f32, so the bit pattern is a plain
        // integer — no NaN-handling required on the GUI side.
        self.gr_db_bits.store(gr_db.to_bits(), Ordering::Relaxed);
    }

    /// GUI thread: read the latest published GR in dB.
    #[inline]
    pub fn load_gr_db(&self) -> f32 {
        f32::from_bits(self.gr_db_bits.load(Ordering::Relaxed))
    }
}

pub struct TelemetryProducer {
    tx: rtrb::Producer<f32>,
    dropped: Arc<AtomicU64>,
}

pub struct TelemetryConsumer {
    rx: rtrb::Consumer<f32>,
    dropped: Arc<AtomicU64>,
}

pub fn channel() -> (TelemetryProducer, TelemetryConsumer) {
    let (tx, rx) = rtrb::RingBuffer::new(RING_SIZE);
    let dropped = Arc::new(AtomicU64::new(0));
    (
        TelemetryProducer {
            tx,
            dropped: Arc::clone(&dropped),
        },
        TelemetryConsumer { rx, dropped },
    )
}

impl TelemetryProducer {
    /// Push a peak sample value. If the buffer is full, the sample is dropped
    /// and the dropped-counter is incremented (not fatal — just telemetry
    /// starvation in the GUI waveform display).
    pub fn push(&mut self, peak: f32) {
        if self.tx.push(peak).is_err() {
            // Relaxed is fine: this is a monotonic counter read by the GUI
            // occasionally for diagnostics; strict ordering isn't required.
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

impl TelemetryConsumer {
    /// Drain up to `max` samples into `buf` (appends). Returns the count read.
    pub fn drain_into(&mut self, buf: &mut Vec<f32>, max: usize) -> usize {
        let mut count = 0;
        while count < max {
            match self.rx.pop() {
                Ok(v) => {
                    buf.push(v);
                    count += 1;
                }
                Err(_) => break,
            }
        }
        count
    }

    /// Total samples dropped by the producer (monotonic counter).
    #[allow(dead_code)]
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drop_counter_increments_when_full() {
        let (mut tx, rx) = channel();
        for _ in 0..(RING_SIZE + 100) {
            tx.push(0.5);
        }
        assert_eq!(rx.dropped(), 100);
    }

    #[test]
    fn drain_reads_pushed_samples() {
        let (mut tx, mut rx) = channel();
        for i in 0..10 {
            tx.push(i as f32);
        }
        let mut buf = Vec::new();
        let n = rx.drain_into(&mut buf, 100);
        assert_eq!(n, 10);
        assert_eq!(buf.len(), 10);
        assert_eq!(buf[0], 0.0);
        assert_eq!(buf[9], 9.0);
    }
}
