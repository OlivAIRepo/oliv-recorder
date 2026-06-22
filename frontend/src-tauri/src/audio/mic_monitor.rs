//! Mic-in-use meeting detection (macOS + Windows).
//!
//! Polls for the process currently capturing the microphone. When a whitelisted
//! meeting app — or a browser hosting a web meeting (Google Meet, ClickUp, …) —
//! starts using the mic, the floating prompt is shown and `meeting-detected` is
//! emitted; on release `meeting-ended` is emitted. Edge-triggered (one event per
//! transition). The prompt UI and the start/continue/end flow are identical on
//! both platforms — only the detection backend differs:
//!   * macOS   — CoreAudio process taps (cidre), matched by bundle id.
//!   * Windows — the CapabilityAccessManager ConsentStore registry (the same
//!     signal Windows uses for its mic-in-use indicator), matched by exe name.
//!
//! Whitelist: a built-in default list, overridden per-org from the middleware
//! (`GET /api/recorder/whitelist`). The per-org bundle-id override applies on
//! macOS; Windows matches a built-in exe list (+ browsers). `browser_meet`
//! applies on both.

use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Runtime};

const POLL_MS: u64 = 1500;

// ---------------------------------------------------------------------------
// Shared whitelist config (per-org override fetched from the middleware).
// ---------------------------------------------------------------------------

struct WhitelistConfig {
    bundle_ids: Vec<String>, // lowercased; used for matching on macOS
    browser_meet: bool,
}

/// Built-in default meeting apps. macOS uses bundle ids (mirrors the middleware
/// `DEFAULT_WHITELIST_BUNDLE_IDS`); Windows matches its own built-in exe list in
/// `detect`, so this is empty there.
#[cfg(target_os = "macos")]
fn builtin_ids() -> Vec<String> {
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
#[cfg(not(target_os = "macos"))]
fn builtin_ids() -> Vec<String> {
    Vec::new()
}

static CONFIG: once_cell::sync::Lazy<std::sync::Mutex<WhitelistConfig>> =
    once_cell::sync::Lazy::new(|| {
        std::sync::Mutex::new(WhitelistConfig {
            bundle_ids: builtin_ids(),
            browser_meet: true,
        })
    });

/// True if `s` (a lowercased bundle id, app name, or exe name) looks like a
/// browser. The mic is held by a browser *helper* whose id varies, so we match
/// loosely; any browser holding the mic is a web meeting when `browser_meet` is on.
fn is_browser(s: &str) -> bool {
    const KW: &[&str] = &[
        "chrome", "chromium", "msedge", "edge", "brave", "vivaldi", "opera", "firefox",
        "mozilla", "safari", "thebrowser",
    ];
    KW.iter().any(|k| s.contains(k))
}

/// Fetch the per-org whitelist from the middleware and update CONFIG. Keeps the
/// current config on any failure (incl. not logged in).
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

// ---------------------------------------------------------------------------
// macOS detection — CoreAudio process taps.
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
fn detect() -> Option<(String, String)> {
    use cidre::{core_audio as ca, ns};

    /// Our own bundle id — never treat the recorder's own mic use as a meeting.
    const OUR_BUNDLE: &str = "ai.oliv.recorder";

    fn app_name(p: &ca::Process) -> Option<String> {
        p.pid()
            .ok()
            .and_then(ns::RunningApp::with_pid)
            .and_then(|a| a.localized_name().map(|s| s.to_string()))
    }

    let processes = ca::System::processes().ok()?;
    let (bundle_ids, browser_meet) = {
        let cfg = CONFIG.lock().unwrap();
        (cfg.bundle_ids.clone(), cfg.browser_meet)
    };
    let debug = std::env::var("OLIV_MIC_DEBUG").is_ok();

    let mut browser_hit = false;
    for p in &processes {
        if !p.is_running_input().unwrap_or(false) {
            continue;
        }
        let bundle = p.bundle_id().map(|b| b.to_string()).unwrap_or_default();
        let bl = bundle.to_ascii_lowercase();
        let name = app_name(p).unwrap_or_default();
        let nl = name.to_ascii_lowercase();
        if debug {
            log::info!(
                "mic_monitor[debug]: mic-input pid={:?} bundle='{bundle}' name='{name}'",
                p.pid().ok()
            );
        }
        if bl == OUR_BUNDLE {
            continue;
        }
        if !bl.is_empty()
            && bundle_ids
                .iter()
                .any(|w| bl == *w || bl.starts_with(&format!("{w}.")))
        {
            let disp = if name.is_empty() { bundle.clone() } else { name };
            return Some((disp, bundle));
        }
        if browser_meet && (is_browser(&bl) || is_browser(&nl)) {
            browser_hit = true;
        }
    }
    if browser_hit {
        return Some(("Browser meeting".to_string(), "browser:meeting".to_string()));
    }
    None
}

// ---------------------------------------------------------------------------
// Windows detection — CapabilityAccessManager ConsentStore registry.
// A process is currently capturing the mic when its entry has
// LastUsedTimeStart != 0 and LastUsedTimeStop == 0 (Windows clears Stop while in
// use and stamps it on release — the same data behind the mic-in-use indicator).
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
fn detect() -> Option<(String, String)> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    fn meeting_exe_label(exe: &str) -> Option<&'static str> {
        match exe {
            "zoom.exe" => Some("Zoom"),
            "teams.exe" | "ms-teams.exe" | "msteams.exe" => Some("Microsoft Teams"),
            "slack.exe" => Some("Slack"),
            "webex.exe" | "webexmta.exe" | "atmgr.exe" | "ptoneclk.exe" => Some("Webex"),
            "gotomeeting.exe" | "goto.exe" => Some("GoTo Meeting"),
            _ => None,
        }
    }
    fn meeting_pkg_label(pfn: &str) -> Option<&'static str> {
        if pfn.contains("msteams") {
            Some("Microsoft Teams")
        } else if pfn.contains("zoom") {
            Some("Zoom")
        } else {
            None
        }
    }
    fn in_use(k: &RegKey) -> bool {
        let start: u64 = k.get_value("LastUsedTimeStart").unwrap_or(0);
        let stop: u64 = k.get_value("LastUsedTimeStop").unwrap_or(1);
        start != 0 && stop == 0
    }

    let browser_meet = CONFIG.lock().unwrap().browser_meet;
    let debug = std::env::var("OLIV_MIC_DEBUG").is_ok();

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let base = hkcu
        .open_subkey(
            r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone",
        )
        .ok()?;

    let mut browser_hit = false;

    // Desktop (NonPackaged) apps: subkey name is the exe path with '#' for '\'.
    if let Ok(np) = base.open_subkey("NonPackaged") {
        for name in np.enum_keys().flatten() {
            let sub = match np.open_subkey(&name) {
                Ok(s) => s,
                Err(_) => continue,
            };
            if !in_use(&sub) {
                continue;
            }
            let path = name.replace('#', "\\");
            let exe = path
                .rsplit('\\')
                .next()
                .unwrap_or(&path)
                .to_ascii_lowercase();
            if debug {
                log::info!("mic_monitor[debug]: mic-input exe='{exe}'");
            }
            if let Some(disp) = meeting_exe_label(&exe) {
                return Some((disp.to_string(), exe));
            }
            if browser_meet && is_browser(&exe) {
                browser_hit = true;
            }
        }
    }

    // Packaged (Store/MSIX) apps: direct subkeys named by PackageFamilyName.
    for name in base.enum_keys().flatten() {
        if name.eq_ignore_ascii_case("NonPackaged") {
            continue;
        }
        let sub = match base.open_subkey(&name) {
            Ok(s) => s,
            Err(_) => continue,
        };
        if !in_use(&sub) {
            continue;
        }
        let nl = name.to_ascii_lowercase();
        if debug {
            log::info!("mic_monitor[debug]: mic-input package='{name}'");
        }
        if let Some(disp) = meeting_pkg_label(&nl) {
            return Some((disp.to_string(), name));
        }
        if browser_meet && is_browser(&nl) {
            browser_hit = true;
        }
    }

    if browser_hit {
        return Some(("Browser meeting".to_string(), "browser:meeting".to_string()));
    }
    None
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn detect() -> Option<(String, String)> {
    None
}

// ---------------------------------------------------------------------------
// Shared prompt window + run loop.
// ---------------------------------------------------------------------------

/// Center the floating prompt near the top of the active monitor and show it
/// (always-on-top, unfocused). The window lives hidden from startup, so its
/// webview is already listening for `meeting-detected` / `meeting-ended`; the
/// event payload decides which UI it renders.
fn show_prompt_window<R: Runtime>(app: &AppHandle<R>) {
    let Some(w) = app.get_webview_window("meeting-prompt") else {
        return;
    };
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
        let x = mpos.x + (msize.width - wsize.width) / 2.0;
        let y = mpos.y + 56.0;
        let _ = w.set_position(LogicalPosition::new(x, y));
    }
    let _ = w.set_always_on_top(true);
    let _ = w.show();
}

fn run<R: Runtime>(app: AppHandle<R>) {
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
            std::thread::sleep(std::time::Duration::from_millis(POLL_MS));
            match detect() {
                Some((name, source)) => {
                    if !present {
                        present = true;
                        // Only prompt logged-in users — transcription needs an account.
                        if crate::auth::ic_token().is_some() {
                            let app2 = app.clone();
                            tauri::async_runtime::spawn(async move {
                                // Already transcribing (back-to-back call) → don't
                                // re-prompt; tell the prompt to cancel any pending
                                // auto-end so we keep recording into the next call.
                                if crate::is_recording().await {
                                    let _ = app2.emit("meeting-resumed", serde_json::json!({}));
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

/// Start the background mic-in-use detector. No-op on unsupported platforms.
pub fn init<R: Runtime>(app: &AppHandle<R>) {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    run(app.clone());
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let _ = app;
}
