//! Oliv account auth — stores the `ic_token` minted by my.oliv.ai and exposes
//! it to the ingest calls.
//!
//! Flow: the "Login with Oliv" button opens
//!   https://my.oliv.ai/login?redirect=https://my.oliv.ai/recorder-auth
//! in the system browser. After login, the same-origin /recorder-auth bridge
//! redirects to
//!   olivrecorder://auth-callback?ic_token=...&ic_user_id=...&email=...
//! which the OS routes back to this app via tauri-plugin-deep-link. We parse the
//! token and persist it, then notify the UI via `oliv-auth-changed`.
//!
//! Storage: a single JSON file (`oliv_account.json`) in the app-data dir,
//! user-only (0600) on unix. We deliberately do NOT use the OS keychain — on an
//! unsigned build every keychain access raises two ACL prompts (item + key) and
//! "Always Allow" doesn't persist across rebuilds. A file in app-data (same
//! place as onboarding/DB) avoids the prompts entirely. Cleared on logout and
//! wiped by reset/uninstall.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Emitter, Manager, Runtime};

const ACCOUNT_FILE: &str = "oliv_account.json";

#[derive(Serialize, Deserialize, Default, Clone)]
struct StoredAccount {
    token: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    user_id: String,
}

// Absolute path to the credentials file, resolved once at startup.
static ACCOUNT_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Resolve and remember the credentials file path. Call once from app setup.
pub fn init_store<R: Runtime>(app: &AppHandle<R>) {
    match app.path().app_data_dir() {
        Ok(dir) => {
            let _ = ACCOUNT_PATH.set(dir.join(ACCOUNT_FILE));
        }
        Err(e) => log::error!("auth: could not resolve app_data_dir: {e}"),
    }
}

fn account_path() -> Option<PathBuf> {
    ACCOUNT_PATH.get().cloned()
}

// In-memory cache so the file is read at most once per launch; writes/logout
// keep it in sync.
struct Cache {
    loaded: bool,
    account: Option<StoredAccount>,
}
static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();

fn cache() -> &'static Mutex<Cache> {
    CACHE.get_or_init(|| {
        Mutex::new(Cache {
            loaded: false,
            account: None,
        })
    })
}

fn cached_account() -> Option<StoredAccount> {
    let mut c = cache().lock().unwrap();
    if !c.loaded {
        c.account = read_account();
        c.loaded = true;
    }
    c.account.clone()
}

fn cache_set(account: Option<StoredAccount>) {
    let mut c = cache().lock().unwrap();
    c.account = account;
    c.loaded = true;
}

fn store_account(acct: &StoredAccount) {
    let json = match serde_json::to_string(acct) {
        Ok(j) => j,
        Err(e) => {
            log::error!("auth: failed to serialize account: {e}");
            return;
        }
    };
    if let Some(path) = account_path() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match std::fs::write(&path, json) {
            Ok(()) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
                }
            }
            Err(e) => log::error!("auth: failed to store account: {e}"),
        }
    } else {
        log::error!("auth: account path not initialized");
    }
    // Keep the in-memory cache in sync.
    cache_set(Some(acct.clone()));
}

fn read_account() -> Option<StoredAccount> {
    let path = account_path()?;
    let json = std::fs::read_to_string(&path).ok()?;
    match serde_json::from_str::<StoredAccount>(&json) {
        Ok(a) if !a.token.is_empty() => Some(a),
        _ => None,
    }
}

fn clear_account() {
    if let Some(path) = account_path() {
        let _ = std::fs::remove_file(&path);
    }
    cache_set(None);
}

/// The current ic_token, if logged in. Read by the recorder ingest calls.
///
/// `OLIV_RECORDER_TOKEN` (env) takes precedence — a dev/testing convenience that
/// bypasses the OS keychain entirely (and its access prompt).
pub fn ic_token() -> Option<String> {
    if let Ok(t) = std::env::var("OLIV_RECORDER_TOKEN") {
        if !t.trim().is_empty() {
            return Some(t);
        }
    }
    cached_account().map(|a| a.token)
}

#[derive(Serialize)]
pub struct OlivAccount {
    pub email: String,
}

/// Returns the signed-in account, or null when no token is stored.
#[tauri::command]
pub fn get_oliv_account() -> Option<OlivAccount> {
    // Env override counts as logged-in but has no stored profile.
    if std::env::var("OLIV_RECORDER_TOKEN")
        .map(|t| !t.trim().is_empty())
        .unwrap_or(false)
    {
        return Some(OlivAccount {
            email: String::new(),
        });
    }
    cached_account().map(|a| OlivAccount { email: a.email })
}

#[tauri::command]
pub fn oliv_logout<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    clear_account();
    // Notify the whole UI (LoginGate, Settings, …) so it re-gates immediately,
    // not just on next launch.
    let _ = app.emit("oliv-auth-changed", false);
    Ok(())
}

/// Parse an `olivrecorder://auth-callback?ic_token=...` URL, persist the
/// credentials, and notify the frontend. Called from the deep-link handler.
pub fn handle_auth_callback<R: Runtime>(app: &AppHandle<R>, url: &str) {
    let parsed = match url::Url::parse(url) {
        Ok(u) => u,
        Err(e) => {
            log::warn!("auth: could not parse deep-link url: {e}");
            return;
        }
    };

    let mut acct = StoredAccount::default();
    for (k, v) in parsed.query_pairs() {
        match k.as_ref() {
            "ic_token" => acct.token = v.into_owned(),
            "ic_user_id" => acct.user_id = v.into_owned(),
            "email" => acct.email = v.into_owned(),
            _ => {}
        }
    }

    if acct.token.is_empty() {
        log::warn!("auth: deep-link callback had no ic_token");
        return;
    }
    store_account(&acct);
    log::info!("auth: stored Oliv credentials from deep link");
    let _ = app.emit("oliv-auth-changed", true);
}
