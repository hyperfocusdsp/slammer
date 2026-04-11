//! Stereo 16-bit PCM file writer for the one-shot bounce feature.
//!
//! Supports WAV (via the `hound` crate) and AIFF (hand-rolled — AIFF is just
//! a few IFF chunks of big-endian PCM, so a one-off encoder is cheaper than
//! pulling in another dependency).
//!
//! Writes go to a sibling `<path>.tmp` file first and are then renamed to
//! the final destination — if the process dies mid-write the user never
//! sees a half-file at the name they'll trust later.

use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

/// Fixed export sample rate, in Hz. Matches `render::EXPORT_SR`.
const SR_HZ: u32 = 44_100;

/// Channel count. Stereo.
const CHANNELS: u16 = 2;

/// Bit depth of the PCM data we write.
const BITS: u16 = 16;

/// Output container format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Wav,
    Aiff,
}

impl Format {
    /// Infer the format from a file extension (case-insensitive). Returns
    /// `None` for unknown extensions so the caller can surface a friendly
    /// error instead of silently picking a default.
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "wav" => Some(Self::Wav),
            "aif" | "aiff" => Some(Self::Aiff),
            _ => None,
        }
    }

    /// Canonical file extension for this format.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Wav => "wav",
            Self::Aiff => "aiff",
        }
    }

    /// Short human label for logs / UI. Not locale-aware.
    pub fn label(self) -> &'static str {
        match self {
            Self::Wav => "WAV",
            Self::Aiff => "AIFF",
        }
    }
}

/// Write a stereo f32 buffer to disk as 16-bit PCM in the requested format.
///
/// * Samples outside ±1.0 are clamped (they shouldn't occur — the render
///   layer peak-normalizes to -1 dBFS — but we clamp anyway for safety).
/// * The write is atomic: we go through `<path>.tmp` + rename.
///
/// `left` and `right` must have the same length.
pub fn write(path: &Path, format: Format, left: &[f32], right: &[f32]) -> io::Result<()> {
    if left.len() != right.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "channel length mismatch",
        ));
    }

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }

    let tmp = tmp_path_for(path);
    match format {
        Format::Wav => write_wav(&tmp, left, right)?,
        Format::Aiff => write_aiff(&tmp, left, right)?,
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

fn tmp_path_for(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".tmp");
    path.with_file_name(name)
}

/// Convert a single f32 sample to clamped 16-bit PCM.
#[inline]
fn to_i16(x: f32) -> i16 {
    let clamped = x.clamp(-1.0, 1.0);
    // 32767.0 (not 32768) so positive peaks don't wrap around on exactly-1.0.
    (clamped * 32767.0).round() as i16
}

// ---------------------------------------------------------------------------
// WAV via hound
// ---------------------------------------------------------------------------

fn write_wav(path: &Path, left: &[f32], right: &[f32]) -> io::Result<()> {
    let spec = hound::WavSpec {
        channels: CHANNELS,
        sample_rate: SR_HZ,
        bits_per_sample: BITS,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).map_err(hound_to_io)?;
    for (l, r) in left.iter().zip(right.iter()) {
        writer.write_sample(to_i16(*l)).map_err(hound_to_io)?;
        writer.write_sample(to_i16(*r)).map_err(hound_to_io)?;
    }
    writer.finalize().map_err(hound_to_io)?;
    Ok(())
}

fn hound_to_io(e: hound::Error) -> io::Error {
    match e {
        hound::Error::IoError(io_err) => io_err,
        other => io::Error::other(other.to_string()),
    }
}

// ---------------------------------------------------------------------------
// AIFF hand-rolled encoder
// ---------------------------------------------------------------------------
//
// AIFF is an Apple variant of IFF:
//
//   FORM <size:u32>  "AIFF"
//       COMM <18:u32>
//           numChannels       : i16
//           numSampleFrames   : u32
//           sampleSize        : i16    (bits)
//           sampleRate        : 80-bit IEEE 754 extended float
//       SSND <size:u32>
//           offset            : u32    (0)
//           blockSize         : u32    (0)
//           <samples...>              (big-endian, interleaved)
//
// Everything is big-endian. Chunk payloads are padded to even length.

fn write_aiff(path: &Path, left: &[f32], right: &[f32]) -> io::Result<()> {
    let num_frames = left.len() as u32;
    let num_samples_bytes = (num_frames as u64) * (CHANNELS as u64) * 2; // 16-bit PCM

    // COMM: 2 + 4 + 2 + 10 = 18 bytes.
    let comm_size: u32 = 18;
    // SSND header is 8 bytes (offset + blockSize) followed by the samples.
    let ssnd_size: u32 = 8 + num_samples_bytes as u32;

    // FORM payload size: "AIFF" fourcc + COMM chunk (header + payload) +
    // SSND chunk (header + payload). Samples are already even-length
    // (16-bit stereo) so no pad byte is needed.
    let form_size: u32 = 4 + (8 + comm_size) + (8 + ssnd_size);

    let file = File::create(path)?;
    let mut w = BufWriter::new(file);

    // FORM header
    w.write_all(b"FORM")?;
    w.write_all(&form_size.to_be_bytes())?;
    w.write_all(b"AIFF")?;

    // COMM chunk
    w.write_all(b"COMM")?;
    w.write_all(&comm_size.to_be_bytes())?;
    w.write_all(&(CHANNELS as i16).to_be_bytes())?;
    w.write_all(&num_frames.to_be_bytes())?;
    w.write_all(&(BITS as i16).to_be_bytes())?;
    // 80-bit IEEE 754 extended-precision representation of 44100.0. This is
    // a fixed constant — we don't support any other rates, so hard-coding it
    // is simpler and faster than implementing a generic f64 → f80 converter.
    // 44100 = 2^15 * 1.345703125 → exponent 16398, mantissa MSB set.
    w.write_all(&AIFF_SR_44100)?;

    // SSND chunk
    w.write_all(b"SSND")?;
    w.write_all(&ssnd_size.to_be_bytes())?;
    w.write_all(&0u32.to_be_bytes())?; // offset
    w.write_all(&0u32.to_be_bytes())?; // blockSize
    for (l, r) in left.iter().zip(right.iter()) {
        w.write_all(&to_i16(*l).to_be_bytes())?;
        w.write_all(&to_i16(*r).to_be_bytes())?;
    }

    w.flush()?;
    Ok(())
}

/// 80-bit IEEE 754 extended-precision encoding of 44100.0. Computed once
/// by hand so the AIFF encoder doesn't need a generic f64 → f80 routine.
///
/// Layout: 1 sign bit + 15 exponent bits + 64 mantissa bits (with the
/// explicit integer bit set — Intel/m68k extended, as used by AIFF).
///
/// For 44100.0:
///   biased exponent = 16398  (0x400E)
///   mantissa        = 0xAC44_0000_0000_0000 (integer bit + 44100/32768)
const AIFF_SR_44100: [u8; 10] = [
    0x40, 0x0E, // sign(0) + exponent 0x400E
    0xAC, 0x44, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // mantissa
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn tmp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("slammer_test_{name}"))
    }

    fn sine(n: usize, freq: f32) -> (Vec<f32>, Vec<f32>) {
        let mut l = Vec::with_capacity(n);
        let mut r = Vec::with_capacity(n);
        for i in 0..n {
            let s = (std::f32::consts::TAU * freq * i as f32 / SR_HZ as f32).sin() * 0.5;
            l.push(s);
            r.push(s);
        }
        (l, r)
    }

    #[test]
    fn from_extension_parses_common_cases() {
        assert_eq!(Format::from_extension("wav"), Some(Format::Wav));
        assert_eq!(Format::from_extension("WAV"), Some(Format::Wav));
        assert_eq!(Format::from_extension("aif"), Some(Format::Aiff));
        assert_eq!(Format::from_extension("AIFF"), Some(Format::Aiff));
        assert_eq!(Format::from_extension("flac"), None);
    }

    #[test]
    fn wav_round_trip_matches_peak_and_length() {
        let (l, r) = sine(1024, 440.0);
        let path = tmp("wav_round_trip.wav");
        write(&path, Format::Wav, &l, &r).unwrap();

        let mut reader = hound::WavReader::open(&path).unwrap();
        let spec = reader.spec();
        assert_eq!(spec.channels, CHANNELS);
        assert_eq!(spec.sample_rate, SR_HZ);
        assert_eq!(spec.bits_per_sample, BITS);

        let samples: Vec<i16> = reader.samples::<i16>().collect::<Result<_, _>>().unwrap();
        assert_eq!(samples.len(), l.len() * 2);

        let peak = samples.iter().map(|s| s.unsigned_abs()).max().unwrap();
        // 0.5 amplitude → ~16383 in 16-bit PCM.
        assert!((15000..=17000).contains(&(peak as i32)), "peak={peak}");

        fs::remove_file(&path).ok();
    }

    #[test]
    fn aiff_header_has_expected_chunk_ids() {
        let (l, r) = sine(512, 220.0);
        let path = tmp("aiff_header.aiff");
        write(&path, Format::Aiff, &l, &r).unwrap();

        let mut bytes = Vec::new();
        File::open(&path).unwrap().read_to_end(&mut bytes).unwrap();

        assert_eq!(&bytes[0..4], b"FORM");
        assert_eq!(&bytes[8..12], b"AIFF");
        assert_eq!(&bytes[12..16], b"COMM");
        // COMM size field.
        assert_eq!(
            u32::from_be_bytes(bytes[16..20].try_into().unwrap()),
            18u32
        );
        // SSND chunk follows the COMM payload (18 bytes).
        let ssnd_off = 20 + 18;
        assert_eq!(&bytes[ssnd_off..ssnd_off + 4], b"SSND");

        // COMM: numChannels, numFrames, bits.
        assert_eq!(
            i16::from_be_bytes(bytes[20..22].try_into().unwrap()),
            CHANNELS as i16
        );
        assert_eq!(
            u32::from_be_bytes(bytes[22..26].try_into().unwrap()),
            l.len() as u32
        );
        assert_eq!(
            i16::from_be_bytes(bytes[26..28].try_into().unwrap()),
            BITS as i16
        );
        // Sample rate bytes are the hard-coded 80-bit constant.
        assert_eq!(&bytes[28..38], &AIFF_SR_44100);

        fs::remove_file(&path).ok();
    }

    #[test]
    fn write_rejects_mismatched_channels() {
        let l = vec![0.0f32; 10];
        let r = vec![0.0f32; 12];
        let path = tmp("mismatch.wav");
        let err = write(&path, Format::Wav, &l, &r).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn write_clamps_out_of_range_samples() {
        let l = vec![2.0f32, -2.0, 1.5, -1.5];
        let r = vec![2.0f32, -2.0, 1.5, -1.5];
        let path = tmp("clamp.wav");
        write(&path, Format::Wav, &l, &r).unwrap();
        let mut reader = hound::WavReader::open(&path).unwrap();
        let samples: Vec<i16> = reader.samples::<i16>().collect::<Result<_, _>>().unwrap();
        // i16 range already bounds this by type; the assertion just
        // documents intent that clamping happened.
        assert!(samples.iter().all(|s| *s >= -32767));
        fs::remove_file(&path).ok();
    }
}
