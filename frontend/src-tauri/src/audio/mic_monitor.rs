//! Mic-in-use meeting detection (macOS).
//!
//! Polls CoreAudio for processes currently using microphone INPUT. When a
//! whitelisted meeting app (Zoom, Teams, Slack, Webex, …) starts using the mic,
//! emits a `meeting-detected` event so the UI can offer to start recording.
//! Edge-triggered: one event per meeting-app appearance (no re-prompt spam).
//!
//! Browsers are intentionally NOT whitelisted by default — a browser's bundle
//! id can't tell Google Meet from any other mic-using site, and a false
//! auto-start is worse than a miss. Per-org overrides (incl. browser-based
//! Meet) are a planned Baserow follow-up.

use tauri::{AppHandle, Emitter, Runtime};

#[cfg(target_os = "macos")]
use cidre::{core_audio as ca, ns};

#[cfg(target_os = "macos")]
mod imp {
    use super::*;

    /// Our own bundle id — never treat the recorder's own mic use as a meeting.
    const OUR_BUNDLE: &str = "ai.oliv.recorder";

    /// Built-in meeting-app whitelist (bundle ids, compared case-insensitively).
    /// A per-org Baserow override is a planned follow-up.
    const WHITELIST: &[&str] = &[
        "us.zoom.xos",               // Zoom
        "com.microsoft.teams2",      // Teams (new)
        "com.microsoft.teams",       // Teams (classic)
        "com.tinyspeck.slackmacgap", // Slack (huddles)
        "com.webex.meetingmanager",  // Webex
        "cisco-systems.spark",       // Webex / Spark
        "com.google.meet",           // Google Meet desktop PWA (if installed)
    ];

    fn is_whitelisted(bundle: &str) -> bool {
        let b = bundle.to_ascii_lowercase();
        WHITELIST.iter().any(|w| *w == b)
    }

    /// The first whitelisted app currently using mic input, as (display_name, bundle_id).
    fn whitelisted_mic_app() -> Option<(String, String)> {
        let processes = ca::System::processes().ok()?;
        for p in processes {
            if !p.is_running_input().unwrap_or(false) {
                continue;
            }
            let bundle = match p.bundle_id() {
                Ok(b) => b.to_string(),
                Err(_) => continue,
            };
            if bundle.eq_ignore_ascii_case(OUR_BUNDLE) || !is_whitelisted(&bundle) {
                continue;
            }
            let name = p
                .pid()
                .ok()
                .and_then(ns::RunningApp::with_pid)
                .and_then(|a| a.localized_name().map(|s| s.to_string()))
                .unwrap_or_else(|| bundle.clone());
            return Some((name, bundle));
        }
        None
    }

    pub fn run<R: Runtime>(app: AppHandle<R>) {
        std::thread::spawn(move || {
            // Edge-triggered: emit only when a whitelisted app newly appears on
            // the mic, and re-arm once it releases the mic.
            let mut present = false;
            loop {
                std::thread::sleep(std::time::Duration::from_millis(1500));
                match whitelisted_mic_app() {
                    Some((name, bundle)) => {
                        if !present {
                            present = true;
                            log::info!("mic_monitor: meeting detected — {name} ({bundle})");
                            let _ = app.emit(
                                "meeting-detected",
                                serde_json::json!({ "app": name, "bundleId": bundle }),
                            );
                        }
                    }
                    None => present = false,
                }
            }
        });
        log::info!("mic_monitor: started");
    }
}

/// Start the background mic-in-use detector. No-op on non-macOS.
pub fn init<R: Runtime>(app: &AppHandle<R>) {
    #[cfg(target_os = "macos")]
    imp::run(app.clone());
    #[cfg(not(target_os = "macos"))]
    let _ = app;
}
