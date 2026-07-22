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
//! built-in-speaker bleed cleanly).
//!
//! Tuning knobs (env vars, read once when the canceller is built — i.e. per
//! recording). Tune them offline first with the `aec-replay` harness
//! (`tools/aec-replay`), which reads the same variables:
//! - `OLIV_AEC_ENABLED`     "0"/"false" bypasses the canceller (default ON)
//! - `OLIV_NS_LEVEL`        off|low|moderate|high|veryhigh — WebRTC noise
//!                          suppression, ~6/12/18/21 dB (default off)
//! - `OLIV_HPF_ENABLED`     "1" adds the full-band high-pass filter; the AEC
//!                          always enforces a split-band HPF (default off)
//! - `OLIV_AEC_TRANSPARENT` legacy|hmm — AEC3's no-echo classifier (default
//!                          legacy; hmm reacts faster on headset/no-echo use)

use sonora::config::{
    EchoCanceller as Aec3Config, HighPassFilter, NoiseSuppression, NoiseSuppressionLevel,
    TransparentModeType,
};
use sonora::{AudioProcessing, Config, StreamConfig};

fn env_bool(key: &str, default: bool) -> bool {
    match std::env::var(key) {
        Ok(v) => !matches!(v.trim().to_ascii_lowercase().as_str(), "0" | "false" | "no" | ""),
        Err(_) => default,
    }
}

/// Build the APM config from the `OLIV_*` env knobs (documented above).
/// Defaults reproduce the shipped behavior: AEC3 only, everything else off.
fn build_config() -> Config {
    let ns = std::env::var("OLIV_NS_LEVEL").unwrap_or_default();
    let noise_suppression = match ns.trim().to_ascii_lowercase().as_str() {
        "low" => Some(NoiseSuppressionLevel::Low),
        "moderate" | "mod" => Some(NoiseSuppressionLevel::Moderate),
        "high" => Some(NoiseSuppressionLevel::High),
        "veryhigh" | "very_high" => Some(NoiseSuppressionLevel::VeryHigh),
        _ => None,
    }
    .map(|level| NoiseSuppression {
        level,
        analyze_linear_aec_output_when_available: true,
    });
    let transparent_mode = match std::env::var("OLIV_AEC_TRANSPARENT")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "hmm" => TransparentModeType::Hmm,
        _ => TransparentModeType::Legacy,
    };
    let high_pass_filter = env_bool("OLIV_HPF_ENABLED", false).then(HighPassFilter::default);
    log::info!(
        "echo_canceller(AEC3): config ns={noise_suppression:?} hpf={high_pass_filter:?} transparent={transparent_mode:?}"
    );
    Config {
        echo_canceller: Some(Aec3Config {
            transparent_mode,
            ..Default::default()
        }),
        noise_suppression,
        high_pass_filter,
        ..Default::default()
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
            Some(
                AudioProcessing::builder()
                    .config(build_config())
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
        if self.apm.is_none() || mic.len() != reference.len() {
            return mic.to_vec();
        }
        // sonora is v0.1.0 and has panicked on some frames ("slice index …").
        // A panic here would unwind the whole audio pipeline task, skipping WAV
        // finalize + the final transcript flush and corrupting the recording.
        // Contain it: on panic, disable AEC for the rest of the session and pass
        // the raw mic through, so a recording is never lost to an AEC bug.
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.process_inner(mic, reference)
        })) {
            Ok(out) => out,
            Err(_) => {
                log::error!(
                    "echo_canceller: AEC3 panicked — disabling AEC for this session, passing mic through"
                );
                self.apm = None;
                mic.to_vec()
            }
        }
    }

    fn process_inner(&mut self, mic: &[f32], reference: &[f32]) -> Vec<f32> {
        let f = self.frame;
        let apm = self
            .apm
            .as_mut()
            .expect("process_inner called with apm present");
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
