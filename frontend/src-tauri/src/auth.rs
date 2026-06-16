//! Oliv account auth — stores the `ic_token` minted by my.oliv.ai in the OS
//! keychain and exposes it to the ingest calls.
//!
//! Flow: the "Login with Oliv" button opens
//!   https://my.oliv.ai/login?redirect=https://my.oliv.ai/recorder-auth
//! in the system browser. After login, the same-origin /recorder-auth bridge
//! redirects to
//!   olivrecorder://auth-callback?ic_token=...&ic_user_id=...&email=...
//! which the OS routes back to this app via tauri-plugin-deep-link. We parse the
//! token and persist it (keychain), then notify the UI via `oliv-auth-changed`.
//!
//! All credentials live in a SINGLE keychain item (one ACL → one keychain
//! prompt) rather than one item per field, which previously triggered a prompt
//! per field per read site.

use keyring::Entry;
use serde::{Deserialize, Serialize};
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Emitter, Runtime};

const SERVICE: &str = "ai.oliv.recorder";
const ACCOUNT_KEY: &str = "oliv_account";

#[derive(Serialize, Deserialize, Default, Clone)]
struct StoredAccount {
    token: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    user_id: String,
}

// In-memory cache so the OS keychain is read at most ONCE per launch. Every
// other read (login gate, Settings, ingest) is served from memory — otherwise
// each distinct keychain access re-prompts on unsigned dev builds. Writes/logout
// keep the cache in sync, so we never need to re-read.
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

fn entry() -> keyring::Result<Entry> {
    Entry::new(SERVICE, ACCOUNT_KEY)
}

/// Cached account: hits the keychain only on the first call of the launch.
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
    match entry().and_then(|e| e.set_password(&json)) {
        Ok(()) => {}
        Err(e) => log::error!("auth: failed to store account: {e}"),
    }
    // Keep the in-memory cache in sync so subsequent reads don't hit the keychain.
    cache_set(Some(acct.clone()));
}

fn read_account() -> Option<StoredAccount> {
    let json = entry().ok().and_then(|e| e.get_password().ok())?;
    match serde_json::from_str::<StoredAccount>(&json) {
        Ok(a) if !a.token.is_empty() => Some(a),
        _ => None,
    }
}

fn clear_account() {
    if let Ok(e) = entry() {
        // delete_credential errors when absent; ignore.
        let _ = e.delete_credential();
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
