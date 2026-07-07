//! Acoustic echo cancellation via WebRTC AEC3 (the `sonora` pure-Rust port) —
//! the same echo canceller Zoom/Meet/Chrome use. Removes far-side speaker bleed
//! (system audio playing through the speakers back into the mic) from the mic
//! channel before it's transcribed / written to mic.wav. On headphones there's
//! no echo, so it's effectively a no-op there.
//!
//! `process(mic, reference)`: `mic` = near-end (your voice + echo), `reference`
//! = far-end (system output). Returns the cleaned near-end. State (the adaptive
//! filter + delay estimate) persists across calls. Audio is 48kHz mono; AEC3
//! works in 10ms (480-sample) frames — the far-end (render) frame is submitted
//! first, then the near-end (capture) frame is cleaned against it.
//!
//! Replaces the earlier hand-rolled NLMS canceller (which couldn't beat
//! built-in-speaker bleed cleanly). `OLIV_AEC_ENABLED` (default ON) bypasses it
//! (raw mic passthrough) when set to "0"/"false".

use sonora::{AudioProcessing, Config, EchoCanceller as Aec3Config, StreamConfig};

fn env_bool(key: &str, default: bool) -> bool {
    match std::env::var(key) {
        Ok(v) => !matches!(v.trim().to_ascii_lowercase().as_str(), "0" | "false" | "no" | ""),
        Err(_) => default,
    }
}

pub struct EchoCanceller {
    apm: Option<AudioProcessing>,
    frame: usize, // samples per 10ms frame at the stream rate
}

impl EchoCanceller {
    /// Build an AEC3 canceller for `sample_rate` (must be 8/16/32/48 kHz — the
    /// rates WebRTC APM supports). Passthrough if the rate is unsupported or
    /// `OLIV_AEC_ENABLED` is off.
    pub fn new(sample_rate: u32) -> Self {
        let enabled = env_bool("OLIV_AEC_ENABLED", true);
        let supported = matches!(sample_rate, 8000 | 16000 | 32000 | 48000);
        let frame = (sample_rate as usize / 100).max(1); // 10ms
        let apm = if enabled && supported {
            let config = Config {
                echo_canceller: Some(Aec3Config::default()),
                ..Default::default()
            };
            Some(
                AudioProcessing::builder()
                    .config(config)
                    .capture_config(StreamConfig::new(sample_rate, 1))
                    .render_config(StreamConfig::new(sample_rate, 1))
                    .build(),
            )
        } else {
            None
        };
        log::info!(
            "echo_canceller(AEC3): active={} rate={sample_rate} frame={frame} (env_enabled={enabled} supported={supported})",
            apm.is_some()
        );
        Self { apm, frame }
    }

    /// Remove the echo of `reference` (far-end/system) from `mic` (near-end +
    /// echo). Both must be the same length and sample-aligned. Returns the
    /// cleaned near-end; passthrough (returns `mic`) when disabled/unsupported,
    /// length-mismatched, or on any per-frame error.
    pub fn process(&mut self, mic: &[f32], reference: &[f32]) -> Vec<f32> {
        let f = self.frame;
        let Some(apm) = self.apm.as_mut() else {
            return mic.to_vec();
        };
        if mic.len() != reference.len() {
            return mic.to_vec();
        }
        let n = mic.len();
        let mut out = Vec::with_capacity(n);
        // Fixed-size 10ms scratch frames (a short trailing frame is zero-padded).
        let mut render_in = vec![0.0f32; f];
        let mut render_out = vec![0.0f32; f];
        let mut capture_in = vec![0.0f32; f];
        let mut capture_out = vec![0.0f32; f];
        let mut i = 0;
        while i < n {
            let end = (i + f).min(n);
            let len = end - i;
            render_in[..len].copy_from_slice(&reference[i..end]);
            capture_in[..len].copy_from_slice(&mic[i..end]);
            if len < f {
                render_in[len..].fill(0.0);
                capture_in[len..].fill(0.0);
            }
            // Far-end first (updates the echo model), then clean the near-end.
            let _ = apm.process_render_f32(&[&render_in], &mut [&mut render_out]);
            if apm
                .process_capture_f32(&[&capture_in], &mut [&mut capture_out])
                .is_ok()
            {
                out.extend_from_slice(&capture_out[..len]);
            } else {
                out.extend_from_slice(&mic[i..end]);
            }
            i = end;
        }
        out
    }
}
