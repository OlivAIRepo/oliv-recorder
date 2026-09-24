//! Enforcing the forced-update floor OUTSIDE the webview.
//!
//! The floor itself is not new — `min-version.json` has been published for
//! months and `MandatoryUpdateGate` renders a blocking screen when the running
//! version is below it. The gate is a React component, and that is the problem:
//! the normal way to run this app is menubar-only with auto-detect recording,
//! where the window is never opened. Nobody sees the gate, nobody clicks
//! Update, and the app keeps recording on an old build indefinitely.
//!
//! So a floor enforced only there is really a floor for people who happen to
//! open the window, and silently no floor at all for everyone else.
//!
//! So the check also runs here, in Rust, on a timer, and announces itself the
//! way the reconnect prompt already does — a native notification and a tray
//! warning, which a background user does see.
//!
//! Deliberately NOT blocking recording. This fails OPEN by design: a network
//! blip, or a floor published wrong, must never be able to stop capture for
//! everyone at once. Being unmissable is the fix; being destructive is not.

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{AppHandle, Emitter, Runtime};
use tauri_plugin_notification::NotificationExt;

/// Set once we know this build is below the floor. Read by the webview so the
/// gate agrees with the notification instead of re-deciding.
static BELOW_FLOOR: AtomicBool = AtomicBool::new(false);
/// One notification per app run — the poll repeats, the nagging should not.
static NOTIFIED: AtomicBool = AtomicBool::new(false);

/// How often to re-check while the app runs. An update published mid-session
/// should reach a machine that stays up for weeks, which is exactly the
/// population that never sees the in-app gate.
const POLL_SECS: u64 = 60 * 60;

/// Whether the running build is below the published floor.
#[tauri::command]
pub fn recorder_update_required() -> bool {
    BELOW_FLOOR.load(Ordering::SeqCst)
}

/// `a < b` over dot-separated numbers, shorter side zero-padded ("1.2.9" <
/// "1.2.10", which a string compare gets wrong). Any non-numeric part makes the
/// comparison unsafe, so it reports "not below" and the floor is ignored.
fn is_below(running: &str, floor: &str) -> Option<bool> {
    let parse = |v: &str| -> Option<Vec<u64>> {
        v.trim().trim_start_matches('v').split('.')
            .map(|p| p.parse::<u64>().ok())
            .collect()
    };
    let (a, b) = (parse(running)?, parse(floor)?);
    let n = a.len().max(b.len());
    for i in 0..n {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return Some(x < y);
        }
    }
    Some(false)
}

async fn check_once<R: Runtime>(app: &AppHandle<R>) {
    let floor = match crate::oliv_min_version_value().await {
        Some(f) => f,
        None => return, // offline, or nothing published — fail open
    };
    let running = env!("CARGO_PKG_VERSION");
    match is_below(running, &floor) {
        Some(true) => {}
        _ => {
            BELOW_FLOOR.store(false, Ordering::SeqCst);
            return;
        }
    }
    BELOW_FLOOR.store(true, Ordering::SeqCst);
    log::warn!("update: running {running} is below the required {floor}");
    let _ = app.emit("oliv-update-required", floor.clone());
    if !NOTIFIED.swap(true, Ordering::SeqCst) {
        let _ = app
            .notification()
            .builder()
            .title("Oliv — update required")
            .body("This version is no longer supported. Open Oliv to update.")
            .show();
    }
    crate::tray::refresh_update_state(app);
}

/// Start the periodic floor check. Called once from app setup.
pub fn start<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        loop {
            check_once(&app).await;
            tokio::time::sleep(std::time::Duration::from_secs(POLL_SECS)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::is_below;

    #[test]
    fn compares_numerically_not_as_strings() {
        // The case a string compare gets wrong, and the reason this is not `<`.
        assert_eq!(is_below("1.2.9", "1.2.10"), Some(true));
        assert_eq!(is_below("1.2.10", "1.2.9"), Some(false));
    }

    #[test]
    fn equal_is_not_below() {
        assert_eq!(is_below("1.2.1", "1.2.1"), Some(false));
    }

    #[test]
    fn older_builds_are_below_a_newer_floor() {
        for old in ["1.0.0", "1.1.9", "1.2.0"] {
            assert_eq!(is_below(old, "1.2.1"), Some(true), "{old}");
        }
        for ok in ["1.2.1", "1.2.2", "1.3.0"] {
            assert_eq!(is_below(ok, "1.2.1"), Some(false), "{ok}");
        }
    }

    #[test]
    fn a_leading_v_is_tolerated() {
        assert_eq!(is_below("1.2.0", "v1.2.1"), Some(true));
    }

    #[test]
    fn shorter_versions_pad_with_zero() {
        assert_eq!(is_below("1.2", "1.2.1"), Some(true));
        assert_eq!(is_below("1.3", "1.2.1"), Some(false));
    }

    #[test]
    fn anything_unparseable_never_forces() {
        // A garbled floor must not lock people out of their own app.
        assert_eq!(is_below("1.2.0", "not-a-version"), None);
        assert_eq!(is_below("dev", "1.2.1"), None);
    }
}
