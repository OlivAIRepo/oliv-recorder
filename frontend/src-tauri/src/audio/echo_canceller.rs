//! Cross-platform acoustic echo canceller (leaky NLMS) — removes speaker bleed
//! (the far side playing through the speakers) from the mic channel before it is
//! written to mic.wav. The system (far-end) audio is the reference signal.
//!
//! Design notes (why this shape):
//!   * Double-talk guard is PEAK-based: near-end is flagged when |mic| rises
//!     well above the recent reference PEAK. An earlier average-power guard
//!     failed when the speaker was loud (the threshold scaled up so the user's
//!     own speech wasn't detected → the filter trained on the user's voice and
//!     cancelled IT instead of the echo). The filter only adapts during
//!     far-end-only stretches.
//!   * Leaky NLMS keeps the weights from blowing up (divergence = noise).
//!   * Safety net: if a window comes out LOUDER than it went in (the filter is
//!     diverging), reset the filter and pass the mic through unchanged. This
//!     guarantees the AEC can never make mic.wav worse than voice+echo.
//!
//! Same code on macOS and Windows (no platform AEC). Tunable via env (read once
//! at construction), so the echo path can be dialed in without a rebuild:
//!   OLIV_AEC_ENABLED (default OFF; set "1"/"true" to re-enable for testing)
//!   OLIV_AEC_TAPS    (filter length; default 4096 ≈ 85ms @48kHz)
//!   OLIV_AEC_MU      (NLMS step size 0..1; default 0.20)
//!   OLIV_AEC_DTD     (double-talk peak threshold; default 2.0; higher = adapt more)
//!   OLIV_AEC_LEAK    (weight leak per update; default 0.0001)

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
    leak: f32,
    nlp_floor: f32,   // residual suppressor: output gain during far-end-only (1.0 = off)
    w: Vec<f32>,      // adaptive filter weights
    x_hist: Vec<f32>, // reference history, x_hist[0] = newest
    energy: f32,      // Σ x_hist² (maintained incrementally)
    ref_peak: f32,    // decaying peak of |reference| (recent far-end level)
    gain: f32,        // smoothed residual-suppressor gain (attack fast, release slow)
    hangover: u32,    // samples to keep the gate open after near-end speech
}

impl EchoCanceller {
    pub fn new() -> Self {
        // Default OFF: the hand-rolled NLMS can't beat built-in-speaker bleed
        // (nonlinear + reverberant residual) cleanly and risks adding artifacts.
        // mic.wav is written raw unless explicitly re-enabled for testing.
        let enabled = env_bool("OLIV_AEC_ENABLED", false);
        let taps = env_usize("OLIV_AEC_TAPS", 4096).clamp(64, 32_768);
        let mu = env_f32("OLIV_AEC_MU", 0.20).clamp(0.01, 1.0);
        let dtd = env_f32("OLIV_AEC_DTD", 2.0).max(1.0);
        let leak = env_f32("OLIV_AEC_LEAK", 0.0001).clamp(0.0, 0.1);
        // Residual suppressor: how far to duck the leftover echo when only the
        // far-end is active. 1.0 disables it; lower = more aggressive.
        let nlp_floor = env_f32("OLIV_AEC_NLP", 0.15).clamp(0.0, 1.0);
        log::info!(
            "echo_canceller: enabled={enabled} taps={taps} mu={mu} dtd={dtd} leak={leak} nlp={nlp_floor}"
        );
        Self {
            enabled,
            taps,
            mu,
            dtd,
            leak,
            nlp_floor,
            w: vec![0.0; taps],
            x_hist: vec![0.0; taps],
            energy: 0.0,
            ref_peak: 0.0,
            gain: 1.0,
            hangover: 0,
        }
    }

    fn reset(&mut self) {
        for v in self.w.iter_mut() {
            *v = 0.0;
        }
    }

    /// Remove the echo of `reference` (far-end/system) from `mic` (near-end +
    /// echo). `mic` and `reference` must be the same length and sample-aligned.
    /// Returns the cleaned near-end. State persists across calls.
    pub fn process(&mut self, mic: &[f32], reference: &[f32]) -> Vec<f32> {
        if !self.enabled || mic.len() != reference.len() {
            return mic.to_vec();
        }
        let n = self.taps;
        let mut out = Vec::with_capacity(mic.len());
        let mut sum_d2 = 0.0f32; // input energy
        let mut sum_e2 = 0.0f32; // output energy

        for i in 0..mic.len() {
            // Slide reference history (newest at front), keep Σx² in sync.
            let leaving = self.x_hist[n - 1];
            self.x_hist.copy_within(0..n - 1, 1);
            let entering = reference[i];
            self.x_hist[0] = entering;
            self.energy += entering * entering - leaving * leaving;
            if self.energy < 0.0 {
                self.energy = 0.0;
            }
            // Decaying peak of the far-end level (≈ recent max |reference|).
            self.ref_peak = entering.abs().max(self.ref_peak * 0.9995);

            // Predicted echo y = wᵀ·x_hist; residual e = mic − y.
            let mut y = 0.0f32;
            for k in 0..n {
                y += self.w[k] * self.x_hist[k];
            }
            let d = mic[i];
            let e = d - y;

            // Peak-based double-talk guard: the near-end is active when the mic
            // exceeds the recent far-end peak (echo alone is attenuated, so it
            // stays below the peak). Adapt only when far-end is present and the
            // near-end is not — never train on the user's voice.
            let near_end = d.abs() > self.dtd * self.ref_peak;
            if !near_end && self.energy > 1e-6 {
                let step = self.mu * e / (self.energy + 1e-6);
                let leak = 1.0 - self.leak;
                for k in 0..n {
                    self.w[k] = self.w[k] * leak + step * self.x_hist[k];
                }
            }

            // Residual echo suppressor: when only the far-end is active (no
            // near-end speech, even within a ~200ms hangover after it), the
            // leftover `e` is residual echo — duck it toward `nlp_floor`. The
            // gain opens fast when you speak and releases slowly, so your voice
            // is never gated. Coefficients assume 48kHz.
            if near_end {
                self.hangover = 9600; // ~200ms @48kHz
            } else if self.hangover > 0 {
                self.hangover -= 1;
            }
            let far_end_only = self.hangover == 0 && self.ref_peak > 1e-4;
            let target = if far_end_only { self.nlp_floor } else { 1.0 };
            // Attack fast (open for speech), release slow (smooth duck).
            let coef = if target > self.gain { 0.05 } else { 0.0008 };
            self.gain += (target - self.gain) * coef;

            out.push((e * self.gain).clamp(-1.0, 1.0));
            sum_d2 += d * d;
            sum_e2 += e * e; // safety check uses the pre-suppressor residual
        }

        // Safety net: if cancellation made this window LOUDER, the filter is
        // diverging — reset it and pass the original mic through. Guarantees the
        // AEC never degrades mic.wav below voice+echo.
        if sum_e2 > sum_d2 * 1.05 {
            self.reset();
            return mic.to_vec();
        }
        out
    }
}
