//! Mic-in-use meeting detection (macOS).
//!
//! Polls CoreAudio for processes currently using microphone INPUT. When a
//! whitelisted meeting app starts using the mic, emits a `meeting-detected`
//! event so the UI can offer to start recording. Edge-triggered: one event per
//! meeting-app appearance (no re-prompt spam).
//!
//! Whitelist: a built-in default list, overridden per-org from the middleware
//! (`GET /api/recorder/whitelist`, refreshed periodically). Google Meet runs in
//! a browser, so it can't be matched by bundle id alone — when `browser_meet`
//! is enabled we additionally check whether a browser using the mic has a
//! Meet-titled window (needs Screen Recording permission, which the app already
//! holds for system-audio capture).

use tauri::{AppHandle, Runtime};

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use cidre::{core_audio as ca, ns};
    use once_cell::sync::Lazy;
    use std::sync::Mutex;
    use tauri::Emitter;

    /// Our own bundle id — never treat the recorder's own mic use as a meeting.
    const OUR_BUNDLE: &str = "ai.oliv.recorder";

    /// Browsers that may host Google Meet. Only treated as a meeting when
    /// `browser_meet` is on AND a Meet-titled window is present.
    const BROWSER_BUNDLES: &[&str] = &[
        "com.google.chrome",
        "com.apple.safari",
        "company.thebrowser.browser", // Arc
        "com.microsoft.edgemac",
        "com.brave.browser",
        "org.chromium.chromium",
        "com.vivaldi.vivaldi",
        "com.operasoftware.opera",
    ];

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

    /// Detect an in-progress meeting from mic-input usage.
    /// Returns (display_name, source_id) — source_id is the bundle id, or
    /// "browser:meet" for a detected Google Meet call.
    fn detect() -> Option<(String, String)> {
        let processes = ca::System::processes().ok()?;
        let (bundle_ids, browser_meet) = {
            let cfg = CONFIG.lock().unwrap();
            (cfg.bundle_ids.clone(), cfg.browser_meet)
        };

        let mut browser_pids: Vec<i64> = Vec::new();
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
            if bundle_ids.iter().any(|w| *w == bl) {
                let name = app_name(p).unwrap_or_else(|| bundle.clone());
                return Some((name, bundle));
            }
            if browser_meet && BROWSER_BUNDLES.contains(&bl.as_str()) {
                if let Ok(pid) = p.pid() {
                    browser_pids.push(pid as i64);
                }
            }
        }

        if !browser_pids.is_empty() && browser_has_meet_window(&browser_pids) {
            return Some(("Google Meet".to_string(), "browser:meet".to_string()));
        }
        None
    }

    /// Is any window owned by one of `pids` titled like a Google Meet call?
    /// Window titles require Screen Recording permission; without it titles are
    /// empty and this returns false (degrades gracefully).
    fn browser_has_meet_window(pids: &[i64]) -> bool {
        use core_foundation::base::TCFType;
        use core_foundation::string::CFString;
        use core_foundation_sys::array::{CFArrayGetCount, CFArrayGetValueAtIndex};
        use core_foundation_sys::dictionary::{CFDictionaryGetValue, CFDictionaryRef};
        use core_foundation_sys::number::{kCFNumberSInt64Type, CFNumberGetValue, CFNumberRef};
        use core_foundation_sys::string::{kCFStringEncodingUTF8, CFStringGetCString, CFStringRef};
        use core_graphics::display::{
            kCGWindowListExcludeDesktopElements, kCGWindowListOptionOnScreenOnly, CGDisplay,
        };
        use std::os::raw::{c_char, c_void};

        let infos = match CGDisplay::window_list_info(
            kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
            None,
        ) {
            Some(a) => a,
            None => return false,
        };
        // The dict-key CFStrings match by CFEqual, so we can build them by name
        // instead of linking the kCGWindow* extern constants.
        let pid_key = CFString::new("kCGWindowOwnerPID");
        let name_key = CFString::new("kCGWindowName");
        let arr = infos.as_concrete_TypeRef();
        let count = unsafe { CFArrayGetCount(arr) };
        for i in 0..count {
            let dict = unsafe { CFArrayGetValueAtIndex(arr, i) } as CFDictionaryRef;
            if dict.is_null() {
                continue;
            }
            let pid_val =
                unsafe { CFDictionaryGetValue(dict, pid_key.as_concrete_TypeRef() as *const c_void) };
            if pid_val.is_null() {
                continue;
            }
            let mut pid: i64 = 0;
            let ok = unsafe {
                CFNumberGetValue(
                    pid_val as CFNumberRef,
                    kCFNumberSInt64Type,
                    &mut pid as *mut i64 as *mut c_void,
                )
            };
            if !ok || !pids.contains(&pid) {
                continue;
            }
            let name_val =
                unsafe { CFDictionaryGetValue(dict, name_key.as_concrete_TypeRef() as *const c_void) };
            if name_val.is_null() {
                continue;
            }
            let mut buf = [0 as c_char; 512];
            let got = unsafe {
                CFStringGetCString(
                    name_val as CFStringRef,
                    buf.as_mut_ptr(),
                    buf.len() as _,
                    kCFStringEncodingUTF8,
                )
            };
            // CFStringGetCString returns a u8 Boolean (unlike CFNumberGetValue → bool).
            if got == 0 {
                continue;
            }
            if let Ok(s) = unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) }.to_str() {
                if s.to_ascii_lowercase().contains("meet") {
                    return true;
                }
            }
        }
        false
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
                            log::info!("mic_monitor: meeting detected — {name} ({source})");
                            let _ = app.emit(
                                "meeting-detected",
                                serde_json::json!({ "app": name, "bundleId": source }),
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
