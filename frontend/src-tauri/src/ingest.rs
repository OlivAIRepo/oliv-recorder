//! Streams the live transcript (and, later, uploads audio) to the Oliv recorder
//! ingest endpoints on baserow-middleware (`/api/recorder/*`).
//!
//! Best-effort by design: any failure is logged and never interrupts recording.
//! The authoritative transcript comes from the end-of-call S3 audio (re-transcribed
//! server-side); this live stream is intermediate capture only.
//!
//! Auth: sends the `ic_token` header (from the OS keychain via `crate::auth`).
//! If the user isn't logged in, ingest is skipped silently.
//!
//! Backend URL: `OLIV_RECORDER_BACKEND` env override, else the prod default.
//! Endpoints (mounted under /api): session | segments | session/end | audio.

use std::sync::Mutex;

use serde::Deserialize;
use serde_json::json;
use tauri::{AppHandle, Listener, Runtime};

const DEFAULT_BACKEND_URL: &str = "https://br-mw.oliv.ai";
const PROVIDER_LOCAL: &str = "local";

struct SessionState {
    session_id: String,
    segment_count: u64,
}

static CURRENT: Mutex<Option<SessionState>> = Mutex::new(None);

fn backend_url() -> String {
    match std::env::var("OLIV_RECORDER_BACKEND") {
        Ok(v) if !v.trim().is_empty() => v.trim().trim_end_matches('/').to_string(),
        _ => DEFAULT_BACKEND_URL.to_string(),
    }
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Minimal view of the worker's TranscriptUpdate event payload.
#[derive(Deserialize)]
struct TranscriptEvent {
    text: String,
    sequence_id: u64,
    #[serde(default)]
    is_partial: bool,
    #[serde(default)]
    confidence: f32,
    #[serde(default)]
    audio_start_time: f64,
    #[serde(default)]
    audio_end_time: f64,
}

async fn post_json(path: &str, body: serde_json::Value) -> Result<(), String> {
    let token = match crate::auth::ic_token() {
        Some(t) => t,
        None => {
            log::debug!("ingest: not logged in — skipping POST /{path}");
            return Ok(());
        }
    };
    let url = format!("{}/api/recorder/{}", backend_url(), path);
    let resp = reqwest::Client::new()
        .post(&url)
        .header("ic_token", token)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("ingest POST /{path} failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("ingest POST /{path} -> HTTP {}", resp.status()));
    }
    Ok(())
}

async fn start_session(meeting_name: Option<String>) {
    // New session id per recording; segments/end reference it.
    let session_id = uuid::Uuid::new_v4().to_string();
    {
        let mut cur = CURRENT.lock().unwrap();
        *cur = Some(SessionState { session_id: session_id.clone(), segment_count: 0 });
    }
    let body = json!({
        "provider": PROVIDER_LOCAL,
        "session_id": session_id,
        "title": meeting_name,
        "started_at": now_iso(),
    });
    if let Err(e) = post_json("session", body).await {
        log::warn!("ingest: {e}");
    } else {
        log::info!("ingest: started session {session_id}");
    }
}

async fn push_segment(ev: TranscriptEvent) {
    let session_id = {
        let mut cur = CURRENT.lock().unwrap();
        match cur.as_mut() {
            Some(s) => {
                s.segment_count += 1;
                s.session_id.clone()
            }
            None => return, // no active ingest session
        }
    };
    let segment = json!({
        "seq": ev.sequence_id,
        "channel": "mixed",   // live stream is the mixed transcript; per-channel comes from S3 audio
        "speaker": "Speaker",
        "text": ev.text,
        "start_ms": (ev.audio_start_time * 1000.0) as i64,
        "end_ms": (ev.audio_end_time * 1000.0) as i64,
        "is_final": true,
        "confidence": ev.confidence,
    });
    let body = json!({ "session_id": session_id, "segments": [segment] });
    if let Err(e) = post_json("segments", body).await {
        log::warn!("ingest: {e}");
    }
}

async fn end_session() {
    let (session_id, count) = {
        let mut cur = CURRENT.lock().unwrap();
        match cur.take() {
            Some(s) => (s.session_id, s.segment_count),
            None => return,
        }
    };
    let body = json!({
        "session_id": session_id,
        "ended_at": now_iso(),
        "segment_count": count,
    });
    if let Err(e) = post_json("session/end", body).await {
        log::warn!("ingest: {e}");
    } else {
        log::info!("ingest: ended session {session_id} ({count} segments)");
    }
}

/// Register lifecycle listeners. Call once from the app setup hook.
pub fn init<R: Runtime>(app: &AppHandle<R>) {
    // recording-started carries the meeting name; open an ingest session.
    app.listen("recording-started", move |event| {
        let meeting_name = serde_json::from_str::<serde_json::Value>(event.payload())
            .ok()
            .and_then(|v| v.get("meeting_name").and_then(|m| m.as_str()).map(String::from));
        tauri::async_runtime::spawn(async move { start_session(meeting_name).await });
    });

    // Each finalized transcript segment streams to the server.
    app.listen("transcript-update", move |event| {
        if let Ok(ev) = serde_json::from_str::<TranscriptEvent>(event.payload()) {
            if ev.is_partial {
                return;
            }
            tauri::async_runtime::spawn(async move { push_segment(ev).await });
        }
    });

    // recording-stopped is the final stop event (carries folder_path + meeting_name);
    // close the session here. Audio upload is wired in the audio phase.
    app.listen("recording-stopped", move |_event| {
        tauri::async_runtime::spawn(async move { end_session().await });
    });

    log::info!("ingest: recorder ingest listeners registered");
}
