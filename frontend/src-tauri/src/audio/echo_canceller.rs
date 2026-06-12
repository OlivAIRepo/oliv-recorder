//! Cross-platform acoustic echo canceller (NLMS) — removes speaker bleed (the
//! far side playing through the speakers) from the mic channel before it is
//! written to mic.wav. The system (far-end) audio is the reference signal.
//!
//! Without this, mic.wav = your voice + an echo of the far side (whatever the
//! mic hears from the speakers). An NLMS adaptive filter models the
//! speaker→room→mic echo path from the reference and subtracts the predicted
//! echo. A Geigel-style double-talk guard freezes adaptation while you speak,
//! so your own voice is never cancelled (the filter only learns during far-end-
//! only stretches). Same code runs on macOS and Windows — no platform AEC.
//!
//! Tunable via env (read once at construction), so the echo path can be dialed
//! in from real recordings without a rebuild:
//!   OLIV_AEC_ENABLED  (default on; "0"/"false" disables — mic passes through)
//!   OLIV_AEC_TAPS     (filter length in samples; default 2048 ≈ 43ms @48kHz)
//!   OLIV_AEC_MU       (NLMS step size 0..1; default 0.30)
//!   OLIV_AEC_DTD      (double-talk threshold; default 4.0; higher = adapt more)

fn env_bool(key: &str, default: bool) -> bool {
    match std::env::var(key) {
        Ok(v) => !matches!(v.trim().to_ascii_lowercase().as_str(), "0" | "false" | "no" | ""),
        Err(_) => default,
    }
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key).ok().and_then(|v| v.trim().parse().ok()).unwrap_or(default)
}

fn env_f32(key: &str, default: f32) -> f32 {
    std::env::var(key).ok().and_then(|v| v.trim().parse().ok()).unwrap_or(default)
}

pub struct EchoCanceller {
    enabled: bool,
    taps: usize,
    mu: f32,
    dtd: f32,
    w: Vec<f32>,      // adaptive filter weights
    x_hist: Vec<f32>, // reference history, x_hist[0] = newest
    energy: f32,      // running sum of squares of x_hist (maintained incrementally)
}

impl EchoCanceller {
    pub fn new() -> Self {
        let enabled = env_bool("OLIV_AEC_ENABLED", true);
        let taps = env_usize("OLIV_AEC_TAPS", 2048).clamp(64, 32_768);
        let mu = env_f32("OLIV_AEC_MU", 0.30).clamp(0.01, 1.0);
        let dtd = env_f32("OLIV_AEC_DTD", 4.0).max(1.0);
        log::info!(
            "echo_canceller: enabled={enabled} taps={taps} mu={mu} dtd={dtd}"
        );
        Self {
            enabled,
            taps,
            mu,
            dtd,
            w: vec![0.0; taps],
            x_hist: vec![0.0; taps],
            energy: 0.0,
        }
    }

    /// Remove the echo of `reference` (far-end/system) from `mic` (near-end +
    /// echo). `mic` and `reference` must be the same length and sample-aligned.
    /// Returns the cleaned near-end. State persists across calls (the filter is
    /// continuous over the whole recording).
    pub fn process(&mut self, mic: &[f32], reference: &[f32]) -> Vec<f32> {
        if !self.enabled || mic.len() != reference.len() {
            return mic.to_vec();
        }
        let n = self.taps;
        let mut out = Vec::with_capacity(mic.len());

        for i in 0..mic.len() {
            // Slide the reference history (newest at front) and keep `energy`
            // (Σx²) in sync incrementally.
            let leaving = self.x_hist[n - 1];
            self.x_hist.copy_within(0..n - 1, 1);
            let entering = reference[i];
            self.x_hist[0] = entering;
            self.energy += entering * entering - leaving * leaving;
            if self.energy < 0.0 {
                self.energy = 0.0; // guard against fp drift
            }

            // Predicted echo y = wᵀ·x_hist, residual e = mic − y.
            let mut y = 0.0f32;
            for k in 0..n {
                y += self.w[k] * self.x_hist[k];
            }
            let d = mic[i];
            let e = d - y;

            // Geigel-style double-talk guard: treat the near-end as active when
            // the mic sample is well above the average reference power, and only
            // adapt the filter when it is NOT (far-end-only). y is still always
            // subtracted, so the current echo estimate keeps being removed even
            // while adaptation is frozen.
            let ref_pow = self.energy / n as f32;
            let near_end = (d * d) > self.dtd * ref_pow;
            if !near_end && self.energy > 1e-6 {
                let step = self.mu * e / (self.energy + 1e-6);
                for k in 0..n {
                    self.w[k] += step * self.x_hist[k];
                }
            }

            out.push(e.clamp(-1.0, 1.0));
        }
        out
    }
}
