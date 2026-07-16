//! Captures the cleaned audio channels to separate WAV files for end-of-call S3
//! upload: `mic.wav` + `system.wav` (per-channel, used to reconstruct Me/Them in
//! transcription) plus `mixed.wav` — a stereo track with the cleaned mic on the
//! left channel and system audio on the right. The hard channel split serves
//! playback and lets multichannel-aware transcription (e.g. AssemblyAI
//! `multichannel`) separate the speakers exactly instead of diarizing.
//!
//! We tap `mic_window` (cleaned mic) and `sys_window` (system) in the pipeline
//! right before they are mixed — never the raw mic. The two windows come from the
//! same ring-buffer extraction, so the stereo channels stay sample-aligned. Files
//! land in the meeting folder; the ingest uploads them at `recording-stopped`.
//! The "mixed" channel is always uploaded for playback (mic-only substitute for
//! sensitive meetings, where the system channel and this stereo `mixed.wav` are
//! withheld).

use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::Result;
use log::{info, warn};

pub const MIC_WAV: &str = "mic.wav";
pub const SYSTEM_WAV: &str = "system.wav";
pub const MIXED_WAV: &str = "mixed.wav";

// Set by the recording saver when it creates the meeting folder; the pipeline
// reads it to open the channel writers for that recording.
static CHANNEL_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

pub fn set_channel_dir(dir: Option<PathBuf>) {
    *CHANNEL_DIR.lock().unwrap() = dir;
}

fn channel_dir() -> Option<PathBuf> {
    CHANNEL_DIR.lock().unwrap().clone()
}

/// Streaming 16-bit PCM WAV writer, mono or stereo (header sizes patched on
/// finalize).
struct WavWriter {
    writer: BufWriter<File>,
    data_bytes: u32,
}

impl WavWriter {
    fn create(path: &Path, sample_rate: u32, channels: u16) -> Result<Self> {
        let mut writer = BufWriter::new(File::create(path)?);
        write_wav_header(&mut writer, sample_rate, channels, 0)?;
        Ok(Self { writer, data_bytes: 0 })
    }

    fn write_samples(&mut self, samples: &[f32]) -> Result<()> {
        for &s in samples {
            let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
            self.writer.write_all(&v.to_le_bytes())?;
        }
        self.data_bytes = self.data_bytes.saturating_add((samples.len() * 2) as u32);
        Ok(())
    }

    /// Interleave two aligned windows as L/R stereo frames. If one window is
    /// shorter (shouldn't happen — both come from the same extraction), the
    /// missing side is padded with silence so the channels never drift.
    fn write_interleaved(&mut self, left: &[f32], right: &[f32]) -> Result<()> {
        let frames = left.len().max(right.len());
        for i in 0..frames {
            for s in [
                left.get(i).copied().unwrap_or(0.0),
                right.get(i).copied().unwrap_or(0.0),
            ] {
                let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
                self.writer.write_all(&v.to_le_bytes())?;
            }
        }
        self.data_bytes = self.data_bytes.saturating_add((frames * 4) as u32);
        Ok(())
    }

    fn finalize(mut self) -> Result<()> {
        self.writer.flush()?;
        let mut file = self
            .writer
            .into_inner()
            .map_err(|e| anyhow::anyhow!("wav finalize into_inner: {e}"))?;
        // RIFF chunk size at offset 4, data chunk size at offset 40.
        file.seek(SeekFrom::Start(4))?;
        file.write_all(&(36 + self.data_bytes).to_le_bytes())?;
        file.seek(SeekFrom::Start(40))?;
        file.write_all(&self.data_bytes.to_le_bytes())?;
        file.flush()?;
        Ok(())
    }
}

fn write_wav_header<W: Write>(
    w: &mut W,
    sample_rate: u32,
    channels: u16,
    data_bytes: u32,
) -> Result<()> {
    let bits: u16 = 16;
    let byte_rate = sample_rate * channels as u32 * (bits as u32 / 8);
    let block_align = channels * (bits / 8);
    w.write_all(b"RIFF")?;
    w.write_all(&(36 + data_bytes).to_le_bytes())?;
    w.write_all(b"WAVE")?;
    w.write_all(b"fmt ")?;
    w.write_all(&16u32.to_le_bytes())?; // fmt chunk size (PCM)
    w.write_all(&1u16.to_le_bytes())?; // audio format = PCM
    w.write_all(&channels.to_le_bytes())?;
    w.write_all(&sample_rate.to_le_bytes())?;
    w.write_all(&byte_rate.to_le_bytes())?;
    w.write_all(&block_align.to_le_bytes())?;
    w.write_all(&bits.to_le_bytes())?;
    w.write_all(b"data")?;
    w.write_all(&data_bytes.to_le_bytes())?;
    Ok(())
}

/// Writes the mic + system channels (for per-channel transcription) plus a mixed
/// track (for human playback) for one recording. Writers open lazily on the first
/// window so an unconfigured recording (no channel dir) is a no-op.
pub struct DualChannelWriter {
    mic: Option<WavWriter>,
    system: Option<WavWriter>,
    mixed: Option<WavWriter>,
    sample_rate: u32,
    dir: PathBuf,
}

impl DualChannelWriter {
    /// Some(..) iff a meeting folder was set for this recording.
    pub fn try_new(sample_rate: u32) -> Option<Self> {
        let dir = channel_dir()?;
        info!("channel_writer: capturing mic/system/mixed channels into {:?}", dir);
        Some(Self { mic: None, system: None, mixed: None, sample_rate, dir })
    }

    pub fn write_mic(&mut self, samples: &[f32]) {
        if self.mic.is_none() {
            match WavWriter::create(&self.dir.join(MIC_WAV), self.sample_rate, 1) {
                Ok(w) => self.mic = Some(w),
                Err(e) => {
                    warn!("channel_writer: failed to create {MIC_WAV}: {e}");
                    return;
                }
            }
        }
        if let Some(w) = self.mic.as_mut() {
            if let Err(e) = w.write_samples(samples) {
                warn!("channel_writer: mic write failed: {e}");
            }
        }
    }

    pub fn write_system(&mut self, samples: &[f32]) {
        if self.system.is_none() {
            match WavWriter::create(&self.dir.join(SYSTEM_WAV), self.sample_rate, 1) {
                Ok(w) => self.system = Some(w),
                Err(e) => {
                    warn!("channel_writer: failed to create {SYSTEM_WAV}: {e}");
                    return;
                }
            }
        }
        if let Some(w) = self.system.as_mut() {
            if let Err(e) = w.write_samples(samples) {
                warn!("channel_writer: system write failed: {e}");
            }
        }
    }

    /// Stereo mixed track: cleaned mic on the left channel, system on the right.
    pub fn write_mixed(&mut self, mic: &[f32], system: &[f32]) {
        if self.mixed.is_none() {
            match WavWriter::create(&self.dir.join(MIXED_WAV), self.sample_rate, 2) {
                Ok(w) => self.mixed = Some(w),
                Err(e) => {
                    warn!("channel_writer: failed to create {MIXED_WAV}: {e}");
                    return;
                }
            }
        }
        if let Some(w) = self.mixed.as_mut() {
            if let Err(e) = w.write_interleaved(mic, system) {
                warn!("channel_writer: mixed write failed: {e}");
            }
        }
    }

    pub fn finalize(self) {
        if let Some(w) = self.mic {
            if let Err(e) = w.finalize() {
                warn!("channel_writer: mic finalize failed: {e}");
            }
        }
        if let Some(w) = self.system {
            if let Err(e) = w.finalize() {
                warn!("channel_writer: system finalize failed: {e}");
            }
        }
        if let Some(w) = self.mixed {
            if let Err(e) = w.finalize() {
                warn!("channel_writer: mixed finalize failed: {e}");
            }
        }
        info!("channel_writer: finalized channel WAVs in {:?}", self.dir);
    }
}
