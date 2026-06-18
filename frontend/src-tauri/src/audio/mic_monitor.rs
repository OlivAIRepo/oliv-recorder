//! Mic-in-use meeting detection (macOS).
//!
//! Polls CoreAudio for processes currently using microphone INPUT. When a
//! whitelisted meeting app starts using the mic, emits a `meeting-detected`
//! event so the UI can offer to start recording. Edge-triggered: one event per
//! meeting-app appearance (no re-prompt spam).
//!
//! Whitelist: a built-in default list, overridden per-org from the middleware
//! (`GET /api/recorder/whitelist`, refreshed periodically). Web meetings (Google
//! Meet, ClickUp, …) run in a browser, so when `browser_meet` is enabled any
//! browser actively holding the mic is treated as a meeting — matched by bundle
//! substring since the audio runs in a browser helper process.

use tauri::{AppHandle, Runtime};

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use cidre::{core_audio as ca, ns};
    use once_cell::sync::Lazy;
    use std::sync::Mutex;
    use tauri::{Emitter, LogicalPosition, LogicalSize, Manager};

    /// Our own bundle id — never treat the recorder's own mic use as a meeting.
    const OUR_BUNDLE: &str = "ai.oliv.recorder";

    /// Friendly label for a browser holding the mic. Matched by SUBSTRING because
    /// the mic is held by a browser *helper* process whose bundle id varies
    /// (e.g. "com.google.Chrome.helper"). Any of these using the mic is treated
    /// as a web meeting (Google Meet, ClickUp, etc.) when `browser_meet` is on.
    fn browser_label(bl: &str) -> Option<&'static str> {
        const MAP: &[(&str, &str)] = &[
            ("chrome", "Browser meeting"),
            ("chromium", "Browser meeting"),
            ("safari", "Browser meeting"),
            ("edgemac", "Browser meeting"),
            ("brave", "Browser meeting"),
            ("thebrowser", "Browser meeting"), // Arc
            ("vivaldi", "Browser meeting"),
            ("opera", "Browser meeting"),
        ];
        MAP.iter().find(|(k, _)| bl.contains(k)).map(|(_, v)| *v)
    }

    /// Built-in fallback — must mirror the middleware default
    /// (recording_service.DEFAULT_WHITELIST_BUNDLE_IDS).
    fn builtin_bundle_ids() -> Vec<String> {
        [
            "us.zoom.xos",
            "com.microsoft.teams2",
            "com.microsoft.teams",
            "com.tinyspeck.slackmacgap",
            "com.webex.meetingmanager",
            "cisco-systems.spark",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    struct WhitelistConfig {
        bundle_ids: Vec<String>, // lowercased
        browser_meet: bool,
    }

    static CONFIG: Lazy<Mutex<WhitelistConfig>> = Lazy::new(|| {
        Mutex::new(WhitelistConfig {
            bundle_ids: builtin_bundle_ids(),
            browser_meet: true,
        })
    });

    fn app_name(p: &ca::Process) -> Option<String> {
        p.pid()
            .ok()
            .and_then(ns::RunningApp::with_pid)
            .and_then(|a| a.localized_name().map(|s| s.to_string()))
    }

    /// Detect an in-progress meeting from mic-input usage. Returns
    /// (display_name, source_id). A native meeting app matches by bundle id
    /// (exact, or a helper under it). A browser holding the mic is a web meeting
    /// (Meet, ClickUp, …) — its audio runs in a helper process, so we match the
    /// bundle id by substring; no window-title check is needed.
    fn detect() -> Option<(String, String)> {
        let processes = ca::System::processes().ok()?;
        let (bundle_ids, browser_meet) = {
            let cfg = CONFIG.lock().unwrap();
            (cfg.bundle_ids.clone(), cfg.browser_meet)
        };

        let mut browser_hit: Option<&'static str> = None;
        for p in &processes {
            if !p.is_running_input().unwrap_or(false) {
                continue;
            }
            let bundle = match p.bundle_id() {
                Ok(b) => b.to_string(),
                Err(_) => continue,
            };
            let bl = bundle.to_ascii_lowercase();
            if bl == OUR_BUNDLE {
                continue;
            }
            if bundle_ids
                .iter()
                .any(|w| bl == *w || bl.starts_with(&format!("{w}.")))
            {
                let name = app_name(p).unwrap_or_else(|| bundle.clone());
                return Some((name, bundle));
            }
            if browser_meet && browser_hit.is_none() {
                browser_hit = browser_label(&bl);
            }
        }
        browser_hit.map(|lbl| (lbl.to_string(), "browser:meeting".to_string()))
    }

    /// Fetch the per-org whitelist from the middleware and update CONFIG.
    /// Keeps the current config on any failure (incl. not logged in).
    async fn refresh_whitelist() {
        let token = match crate::auth::ic_token() {
            Some(t) => t,
            None => return,
        };
        let url = format!("{}/api/recorder/whitelist", crate::ingest::backend_url());
        let resp = reqwest::Client::new()
            .get(&url)
            .header("User-Agent", crate::ingest::USER_AGENT)
            .header("Cookie", format!("ic_token={token}"))
            .header("ic_token", &token)
            .send()
            .await;
        let resp = match resp {
            Ok(r) if r.status().is_success() => r,
            _ => return,
        };
        #[derive(serde::Deserialize)]
        struct Wl {
            #[serde(default)]
            bundle_ids: Vec<String>,
            #[serde(default)]
            browser_meet: bool,
        }
        let wl: Wl = match resp.json().await {
            Ok(w) => w,
            Err(_) => return,
        };
        let bundle_ids: Vec<String> = wl.bundle_ids.iter().map(|s| s.to_ascii_lowercase()).collect();
        if bundle_ids.is_empty() && !wl.browser_meet {
            return; // empty/garbage — keep current config
        }
        let mut cfg = CONFIG.lock().unwrap();
        cfg.bundle_ids = bundle_ids;
        cfg.browser_meet = wl.browser_meet;
        log::info!(
            "mic_monitor: whitelist updated ({} apps, browser_meet={})",
            cfg.bundle_ids.len(),
            cfg.browser_meet
        );
    }

    /// Center the floating prompt on the active monitor and show it
    /// (always-on-top, unfocused). The window lives hidden from startup, so its
    /// webview is already loaded and listening for `meeting-detected` /
    /// `meeting-ended`; the event payload decides which UI it renders.
    fn show_prompt_window<R: Runtime>(app: &AppHandle<R>) {
        let Some(w) = app.get_webview_window("meeting-prompt") else {
            return;
        };
        // Remember whether the main window was visible — so closing the prompt
        // (the app's key window) doesn't surface a previously-hidden main window.
        let main_visible = app
            .get_webview_window("main")
            .and_then(|m| m.is_visible().ok())
            .unwrap_or(false);
        crate::MAIN_VISIBLE_BEFORE_PROMPT.store(main_visible, std::sync::atomic::Ordering::SeqCst);

        if let Ok(Some(monitor)) = w.current_monitor() {
            let scale = monitor.scale_factor();
            let msize = monitor.size().to_logical::<f64>(scale);
            let mpos = monitor.position().to_logical::<f64>(scale);
            let wsize = w
                .outer_size()
                .map(|s| s.to_logical::<f64>(scale))
                .unwrap_or(LogicalSize::new(380.0, 200.0));
            // Centered horizontally, near the top of the screen (center-top).
            let x = mpos.x + (msize.width - wsize.width) / 2.0;
            let y = mpos.y + 56.0;
            let _ = w.set_position(LogicalPosition::new(x, y));
        }
        let _ = w.set_always_on_top(true);
        let _ = w.show();
    }

    pub fn run<R: Runtime>(app: AppHandle<R>) {
        // Periodically pull the per-org whitelist (async; no-op until logged in).
        tauri::async_runtime::spawn(async {
            loop {
                refresh_whitelist().await;
                tokio::time::sleep(std::time::Duration::from_secs(300)).await;
            }
        });

        // Edge-triggered mic-in-use detection.
        std::thread::spawn(move || {
            let mut present = false;
            loop {
                std::thread::sleep(std::time::Duration::from_millis(1500));
                match detect() {
                    Some((name, source)) => {
                        if !present {
                            present = true;
                            // Only prompt logged-in users — transcription needs an account.
                            if crate::auth::ic_token().is_some() {
                                let app2 = app.clone();
                                tauri::async_runtime::spawn(async move {
                                    // Already transcribing (e.g. back-to-back call after
                                    // "Continue") → don't re-prompt to start.
                                    if crate::is_recording().await {
                                        return;
                                    }
                                    log::info!("mic_monitor: meeting detected — {name} ({source})");
                                    show_prompt_window(&app2);
                                    let _ = app2.emit(
                                        "meeting-detected",
                                        serde_json::json!({ "app": name, "bundleId": source }),
                                    );
                                });
                            }
                        }
                    }
                    None => {
                        if present {
                            present = false;
                            // Mic released. If we're transcribing, show the persistent
                            // Continue/End banner — never auto-stop, never native notify.
                            let app2 = app.clone();
                            tauri::async_runtime::spawn(async move {
                                if crate::is_recording().await {
                                    log::info!("mic_monitor: meeting ended — prompting continue/end");
                                    show_prompt_window(&app2);
                                    let _ = app2.emit("meeting-ended", serde_json::json!({}));
                                }
                            });
                        }
                    }
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
