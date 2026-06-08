//! Oliv account auth — stores the `ic_token` minted by my.oliv.ai in the OS
//! keychain and exposes it to the ingest calls.
//!
//! Flow: the Settings "Login with Oliv" button opens
//!   https://my.oliv.ai/login?final-page=olivrecorder://auth-callback
//! in the system browser. After login, my.oliv.ai redirects to
//!   olivrecorder://auth-callback?ic_token=...&ic_user_id=...&email=...
//! which the OS routes back to this app via tauri-plugin-deep-link. We parse the
//! token and persist it (keychain), then notify the UI via `oliv-auth-changed`.

use keyring::Entry;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};

const SERVICE: &str = "ai.oliv.recorder";
const TOKEN_KEY: &str = "ic_token";
const EMAIL_KEY: &str = "oliv_email";
const USER_ID_KEY: &str = "oliv_user_id";

fn entry(key: &str) -> keyring::Result<Entry> {
    Entry::new(SERVICE, key)
}

fn store(key: &str, value: &str) {
    match entry(key).and_then(|e| e.set_password(value)) {
        Ok(()) => {}
        Err(e) => log::error!("auth: failed to store {key}: {e}"),
    }
}

fn read(key: &str) -> Option<String> {
    entry(key).ok().and_then(|e| e.get_password().ok())
}

fn clear(key: &str) {
    if let Ok(e) = entry(key) {
        // delete_credential is a no-op-style error when absent; ignore.
        let _ = e.delete_credential();
    }
}

/// The current ic_token, if logged in. Read by the recorder ingest calls.
///
/// `OLIV_RECORDER_TOKEN` (env) takes precedence — a dev/testing convenience that
/// bypasses the OS keychain entirely (and the keychain-access prompt you'd get
/// when the token wasn't written by the app itself, e.g. injected for a test).
pub fn ic_token() -> Option<String> {
    if let Ok(t) = std::env::var("OLIV_RECORDER_TOKEN") {
        if !t.trim().is_empty() {
            return Some(t);
        }
    }
    read(TOKEN_KEY)
}

#[derive(Serialize)]
pub struct OlivAccount {
    pub email: String,
}

/// Returns the signed-in account, or null when no token is stored.
#[tauri::command]
pub fn get_oliv_account() -> Option<OlivAccount> {
    if ic_token().is_some() {
        Some(OlivAccount {
            email: read(EMAIL_KEY).unwrap_or_default(),
        })
    } else {
        None
    }
}

#[tauri::command]
pub fn oliv_logout() -> Result<(), String> {
    clear(TOKEN_KEY);
    clear(EMAIL_KEY);
    clear(USER_ID_KEY);
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

    let mut token: Option<String> = None;
    let mut user_id: Option<String> = None;
    let mut email: Option<String> = None;
    for (k, v) in parsed.query_pairs() {
        match k.as_ref() {
            "ic_token" => token = Some(v.into_owned()),
            "ic_user_id" => user_id = Some(v.into_owned()),
            "email" => email = Some(v.into_owned()),
            _ => {}
        }
    }

    match token {
        Some(t) if !t.is_empty() => {
            store(TOKEN_KEY, &t);
            if let Some(em) = email.as_deref() {
                store(EMAIL_KEY, em);
            }
            if let Some(uid) = user_id.as_deref() {
                store(USER_ID_KEY, uid);
            }
            log::info!("auth: stored Oliv credentials from deep link");
            let _ = app.emit("oliv-auth-changed", true);
        }
        _ => log::warn!("auth: deep-link callback had no ic_token"),
    }
}
