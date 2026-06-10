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

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use serde::Deserialize;
use serde_json::json;
use tauri::{AppHandle, Listener, Runtime};

const DEFAULT_BACKEND_URL: &str = "https://my.oliv.ai";
const PROVIDER_LOCAL: &str = "local";
// reqwest sends no User-Agent by default, and the prod WAF 403s requests with an
// empty/missing UA before they reach the middleware. Always set one.
pub(crate) const USER_AGENT: &str = concat!("OlivRecorder/", env!("CARGO_PKG_VERSION"));

// "Sensitive meeting" toggle (Home screen). When set, only the cleaned mic
// channel is uploaded; otherwise both mic + system are uploaded. Never raw mic.
static SENSITIVE: AtomicBool = AtomicBool::new(false);

/// Set by the Home "Sensitive meeting" toggle.
#[tauri::command]
pub fn oliv_set_sensitive(sensitive: bool) {
    SENSITIVE.store(sensitive, Ordering::SeqCst);
    log::info!("ingest: sensitive meeting = {sensitive}");
}

// Source app that triggered the recording (e.g. "zoom.us" from the auto-detect
// prompt). Tags the ingest session; cleared at session end so a later manual
// start isn't mislabelled. None for manual starts.
static SOURCE_APP: Mutex<Option<String>> = Mutex::new(None);

/// Set by the meeting-detected prompt before an auto-started recording.
#[tauri::command]
pub fn oliv_set_source_app(app: Option<String>) {
    let v = app.and_then(|s| {
        let t = s.trim().to_string();
        (!t.is_empty()).then_some(t)
    });
    log::info!("ingest: source app = {v:?}");
    *SOURCE_APP.lock().unwrap() = v;
}

struct SessionState {
    session_id: String,
    segment_count: u64,
    // Token resolved once at session start and reused for segments/end, so the
    // keychain is read at most once per recording (not per POST).
    token: String,
    // The server-side session row is created asynchronously by start_session. Until
    // it confirms, segments are buffered (not POSTed) so they can't race ahead of
    // the row's creation (which can be slow on a cold middleware cache).
    ready: bool,
    buffer: Vec<serde_json::Value>,
}

static CURRENT: Mutex<Option<SessionState>> = Mutex::new(None);

pub(crate) fn backend_url() -> String {
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

async fn post_json(token: &str, path: &str, body: serde_json::Value) -> Result<(), String> {
    let url = format!("{}/api/recorder/{}", backend_url(), path);
    let resp = reqwest::Client::new()
        .post(&url)
        .header("User-Agent", USER_AGENT)
        // Send as a cookie: the prod gateway strips underscore headers like
        // `ic_token`, so the header alone never reaches the middleware. The
        // middleware reads the token from either the cookie or the header.
        .header("Cookie", format!("ic_token={token}"))
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
    // Resolve the token once for the whole recording (keychain read happens here only).
    let token = match crate::auth::ic_token() {
        Some(t) => t,
        None => {
            log::debug!("ingest: not logged in — skipping session");
            return;
        }
    };
    let session_id = uuid::Uuid::new_v4().to_string();
    {
        let mut cur = CURRENT.lock().unwrap();
        *cur = Some(SessionState {
            session_id: session_id.clone(),
            segment_count: 0,
            token: token.clone(),
            ready: false,
            buffer: Vec::new(),
        });
    }
    let source_app = SOURCE_APP.lock().unwrap().clone();
    let body = json!({
        "provider": PROVIDER_LOCAL,
        "session_id": session_id,
        "title": meeting_name,
        "source_app": source_app,
        "started_at": now_iso(),
    });
    if let Err(e) = post_json(&token, "session", body).await {
        // Leave ready=false; segments keep buffering. end_session will retry the flush.
        log::warn!("ingest: session start failed: {e}");
        return;
    }
    // Session row exists server-side now: mark ready and flush anything buffered
    // while it was being created (avoids segments racing ahead of the row).
    let buffered = {
        let mut cur = CURRENT.lock().unwrap();
        match cur.as_mut() {
            Some(s) => {
                s.ready = true;
                std::mem::take(&mut s.buffer)
            }
            None => Vec::new(),
        }
    };
    log::info!("ingest: started session {session_id}");
    if !buffered.is_empty() {
        let body = json!({ "session_id": session_id, "segments": buffered });
        if let Err(e) = post_json(&token, "segments", body).await {
            log::warn!("ingest: flush buffered segments failed: {e}");
        }
    }
}

fn segment_json(ev: &TranscriptEvent) -> serde_json::Value {
    json!({
        "seq": ev.sequence_id,
        "channel": "mixed",   // live stream is the mixed transcript; per-channel comes from S3 audio
        "speaker": "Speaker",
        "text": ev.text,
        "start_ms": (ev.audio_start_time * 1000.0) as i64,
        "end_ms": (ev.audio_end_time * 1000.0) as i64,
        "is_final": true,
        "confidence": ev.confidence,
    })
}

async fn push_segment(ev: TranscriptEvent) {
    // Either POST now (session ready) or buffer (still being created). Never POST
    // before the session row exists, or it 500s with "no Session".
    let to_post = {
        let mut cur = CURRENT.lock().unwrap();
        match cur.as_mut() {
            Some(s) => {
                s.segment_count += 1;
                let seg = segment_json(&ev);
                if s.ready {
                    Some((s.session_id.clone(), s.token.clone(), seg))
                } else {
                    s.buffer.push(seg);
                    None
                }
            }
            None => return, // no active ingest session
        }
    };
    if let Some((session_id, token, seg)) = to_post {
        let body = json!({ "session_id": session_id, "segments": [seg] });
        if let Err(e) = post_json(&token, "segments", body).await {
            log::warn!("ingest: {e}");
        }
    }
}

async fn end_session() {
    // Wait for the session to be confirmed (cold-start can delay creation), so the
    // final segments + end land after the row exists. Bounded so we never hang.
    for _ in 0..120 {
        let ready = CURRENT.lock().unwrap().as_ref().map(|s| s.ready).unwrap_or(true);
        if ready {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    let (session_id, count, token, buffer) = {
        let mut cur = CURRENT.lock().unwrap();
        match cur.take() {
            Some(s) => (s.session_id, s.segment_count, s.token, s.buffer),
            None => return,
        }
    };
    // Reset source tag so a subsequent manual recording isn't mislabelled.
    *SOURCE_APP.lock().unwrap() = None;

    // Flush any still-buffered segments before ending.
    if !buffer.is_empty() {
        let body = json!({ "session_id": session_id, "segments": buffer });
        if let Err(e) = post_json(&token, "segments", body).await {
            log::warn!("ingest: end flush failed: {e}");
        }
    }
    let body = json!({
        "session_id": session_id,
        "ended_at": now_iso(),
        "segment_count": count,
    });
    if let Err(e) = post_json(&token, "session/end", body).await {
        log::warn!("ingest: {e}");
    } else {
        log::info!("ingest: ended session {session_id} ({count} segments)");
    }
}

async fn upload_channel(token: &str, session_id: &str, channel: &str, path: &Path) -> Result<(), String> {
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    let part = reqwest::multipart::Part::bytes(bytes)
        .file_name(format!("{channel}.wav"))
        .mime_str("audio/wav")
        .map_err(|e| e.to_string())?;
    let form = reqwest::multipart::Form::new()
        .text("session_id", session_id.to_string())
        .text("channel", channel.to_string())
        .part("file", part);
    let url = format!("{}/api/recorder/audio", backend_url());
    let resp = reqwest::Client::new()
        .post(&url)
        .header("User-Agent", USER_AGENT)
        .header("Cookie", format!("ic_token={token}"))
        .header("ic_token", token)
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("audio upload ({channel}) failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("audio upload ({channel}) -> HTTP {}", resp.status()));
    }
    Ok(())
}

/// Upload the cleaned channel WAV(s) from the recording folder per the sensitive
/// flag: sensitive => mic only; otherwise mic + system. Never the raw mic.
async fn upload_audio(token: &str, session_id: &str, folder: &str) {
    let sensitive = SENSITIVE.load(Ordering::SeqCst);
    let dir = Path::new(folder);

    let mic = dir.join(crate::audio::channel_writer::MIC_WAV);
    if mic.exists() {
        match upload_channel(token, session_id, "mic", &mic).await {
            Ok(()) => log::info!("ingest: uploaded mic channel"),
            Err(e) => log::warn!("ingest: {e}"),
        }
    } else {
        log::warn!("ingest: {} not found — skipping mic upload", mic.display());
    }

    if sensitive {
        log::info!("ingest: sensitive meeting — system channel withheld");
        return;
    }
    let sys = dir.join(crate::audio::channel_writer::SYSTEM_WAV);
    if sys.exists() {
        match upload_channel(token, session_id, "system", &sys).await {
            Ok(()) => log::info!("ingest: uploaded system channel"),
            Err(e) => log::warn!("ingest: {e}"),
        }
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

    // recording-stopped is the final stop event (carries folder_path + meeting_name):
    // close the session, then upload the cleaned channel WAV(s) from the folder.
    app.listen("recording-stopped", move |event| {
        let folder = serde_json::from_str::<serde_json::Value>(event.payload())
            .ok()
            .and_then(|v| v.get("folder_path").and_then(|f| f.as_str()).map(String::from));
        tauri::async_runtime::spawn(async move {
            // Capture session creds before end_session() consumes CURRENT.
            let creds = {
                let cur = CURRENT.lock().unwrap();
                cur.as_ref().map(|s| (s.session_id.clone(), s.token.clone()))
            };
            end_session().await;
            if let (Some((session_id, token)), Some(dir)) = (creds, folder) {
                upload_audio(&token, &session_id, &dir).await;
            }
        });
    });

    log::info!("ingest: recorder ingest listeners registered");
}
