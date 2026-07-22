//! Offline AEC tuning harness. Replays a raw mic recording against its system
//! reference through the same sonora (WebRTC APM/AEC3) pipeline the app uses in
//! `frontend/src-tauri/src/audio/echo_canceller.rs`, so configs can be swept
//! over identical audio instead of re-recording live calls.
//!
//! Getting a test pair: record one call with `OLIV_AEC_ENABLED=0` — the meeting
//! folder's `mic.wav` is then the RAW mic (echo included) and `system.wav` is
//! the reference. Both are 48kHz mono 16-bit and sample-aligned.
//!
//! Usage:
//!   cargo run -p aec-replay --release -- <raw_mic.wav> <system.wav> [out.wav]
//!
//! Config env vars (same names/semantics as the app):
//!   OLIV_NS_LEVEL=off|low|moderate|high|veryhigh   noise suppression (default off)
//!   OLIV_HPF_ENABLED=1                             full-band high-pass filter
//!   OLIV_AEC_TRANSPARENT=legacy|hmm                AEC3 transparent-mode classifier
//!
//! Example sweep:
//!   for ns in off low moderate high; do
//!     OLIV_NS_LEVEL=$ns cargo run -p aec-replay --release -- mic.wav system.wav out_$ns.wav
//!   done
//!
//! Prints comparative metrics (noise floor, echo attenuation while the far end
//! is talking, onset vs steady-state) for raw vs processed.

use std::process::exit;

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

// Mirrors build_config() in the app's echo_canceller.rs — keep in sync.
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
    eprintln!(
        "config: ns={noise_suppression:?} hpf={high_pass_filter:?} transparent={transparent_mode:?}"
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

fn read_mono_f32(path: &str) -> (Vec<f32>, u32) {
    let mut reader = hound::WavReader::open(path)
        .unwrap_or_else(|e| { eprintln!("failed to open {path}: {e}"); exit(1) });
    let spec = reader.spec();
    if spec.channels != 1 {
        eprintln!("{path}: expected mono, got {} channels", spec.channels);
        exit(1);
    }
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => {
            let scale = ((1i64 << (spec.bits_per_sample - 1)) as f32).max(1.0);
            reader.samples::<i32>().map(|s| s.unwrap() as f32 / scale).collect()
        }
        hound::SampleFormat::Float => reader.samples::<f32>().map(|s| s.unwrap()).collect(),
    };
    (samples, spec.sample_rate)
}

/// Per-100ms-frame RMS in dBFS.
fn frame_db(x: &[f32], frame: usize) -> Vec<f32> {
    x.chunks_exact(frame)
        .map(|c| {
            let rms = (c.iter().map(|s| s * s).sum::<f32>() / c.len() as f32).sqrt();
            20.0 * rms.max(1e-8).log10()
        })
        .collect()
}

fn noise_floor_db(db: &[f32]) -> f32 {
    let mut sorted = db.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let decile = &sorted[..(sorted.len() / 10).max(1)];
    decile.iter().sum::<f32>() / decile.len() as f32
}

fn median(mut v: Vec<f32>) -> f32 {
    if v.is_empty() {
        return f32::NAN;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: aec-replay <raw_mic.wav> <system.wav> [out.wav]");
        exit(2);
    }
    let (mic, mic_rate) = read_mono_f32(&args[1]);
    let (reference, ref_rate) = read_mono_f32(&args[2]);
    if mic_rate != ref_rate {
        eprintln!("sample-rate mismatch: mic={mic_rate} reference={ref_rate}");
        exit(1);
    }
    if !matches!(mic_rate, 8000 | 16000 | 32000 | 48000) {
        eprintln!("unsupported rate {mic_rate} (need 8/16/32/48 kHz)");
        exit(1);
    }
    let n = mic.len().min(reference.len());
    eprintln!(
        "loaded {:.1}s @ {mic_rate} Hz (mic {} / ref {} samples, using {n})",
        n as f32 / mic_rate as f32,
        mic.len(),
        reference.len()
    );

    // Same shape as the app: 10ms frames, render (far end) first, then capture.
    let mut apm = AudioProcessing::builder()
        .config(build_config())
        .capture_config(StreamConfig::new(mic_rate, 1))
        .render_config(StreamConfig::new(mic_rate, 1))
        .build();
    let f = mic_rate as usize / 100;
    let mut out = Vec::with_capacity(n);
    let mut render_out = vec![0.0f32; f];
    let mut capture_out = vec![0.0f32; f];
    let mut i = 0;
    while i + f <= n {
        let _ = apm.process_render_f32(&[&reference[i..i + f]], &mut [&mut render_out]);
        if apm
            .process_capture_f32(&[&mic[i..i + f]], &mut [&mut capture_out])
            .is_ok()
        {
            out.extend_from_slice(&capture_out);
        } else {
            out.extend_from_slice(&mic[i..i + f]);
        }
        i += f;
    }
    out.extend_from_slice(&mic[i..n]); // trailing partial frame passthrough

    // ---- Metrics over 100ms frames ----
    let mf = mic_rate as usize / 10;
    let mic_db = frame_db(&mic[..n], mf);
    let ref_db = frame_db(&reference[..n], mf);
    let out_db = frame_db(&out, mf);
    let frames = mic_db.len().min(ref_db.len()).min(out_db.len());

    const FAR_ACTIVE_DB: f32 = -35.0;
    // Onset = far end becomes active after >=300ms of far-end silence. The
    // first 500ms after an onset is where slow convergence leaks echo.
    let mut onset_att = Vec::new();
    let mut steady_att = Vec::new();
    let mut silent_run = usize::MAX / 2;
    let mut since_onset = usize::MAX / 2;
    for t in 0..frames {
        if ref_db[t] > FAR_ACTIVE_DB {
            if silent_run >= 3 {
                since_onset = 0;
            }
            let att = mic_db[t] - out_db[t];
            if since_onset < 5 {
                onset_att.push(att);
            } else {
                steady_att.push(att);
            }
            silent_run = 0;
            since_onset += 1;
        } else {
            silent_run += 1;
        }
    }

    println!();
    println!("                         raw mic    processed");
    println!(
        "noise floor (dBFS)      {:8.1}    {:9.1}",
        noise_floor_db(&mic_db),
        noise_floor_db(&out_db)
    );
    println!(
        "far-end-active frames: {} ({:.1}s) — attenuation = raw minus processed level",
        onset_att.len() + steady_att.len(),
        (onset_att.len() + steady_att.len()) as f32 / 10.0
    );
    println!(
        "echo attenuation, onsets   (first 0.5s): {:6.1} dB median over {} frames",
        median(onset_att.clone()),
        onset_att.len()
    );
    println!(
        "echo attenuation, steady state:          {:6.1} dB median over {} frames",
        median(steady_att.clone()),
        steady_att.len()
    );
    println!(
        "(attenuation counts near-end speech too — compare configs on the SAME file, \
         higher = more removed)"
    );

    if let Some(out_path) = args.get(3) {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: mic_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(out_path, spec)
            .unwrap_or_else(|e| { eprintln!("failed to create {out_path}: {e}"); exit(1) });
        for s in &out {
            writer
                .write_sample((s.clamp(-1.0, 1.0) * 32767.0) as i16)
                .unwrap();
        }
        writer.finalize().unwrap();
        println!("wrote {out_path}");
    }
}
