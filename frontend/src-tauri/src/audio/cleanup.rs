//! Local audio retention. Meeting folders keep their (small) transcripts.json
//! and metadata.json forever; the audio files are pruned in two stages:
//!   1. mic.wav / system.wav are deleted right after their upload succeeds
//!      (ingest::upload_audio) — the S3 copy is the durable one.
//!   2. audio.mp4 (local playback) and any leftover WAVs from failed uploads
//!      are deleted here once they are older than RETENTION_DAYS.
//!
//! Runs once at app startup on a background task, over the user's configured
//! recordings folder.

use std::path::Path;
use std::time::{Duration, SystemTime};

use log::{info, warn};
use tauri::{AppHandle, Runtime};

use super::recording_preferences::load_recording_preferences;

const RETENTION_DAYS: u64 = 7;
// Audio artifacts eligible for retention pruning. mixed.wav is a leftover from
// pre-0.3.24 builds; the WAVs normally disappear at upload time already.
const PRUNABLE: [&str; 4] = ["audio.mp4", "mic.wav", "system.wav", "mixed.wav"];

fn is_older_than(path: &Path, cutoff: Duration) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    let Ok(modified) = meta.modified() else {
        return false;
    };
    match SystemTime::now().duration_since(modified) {
        Ok(age) => age > cutoff,
        Err(_) => false, // mtime in the future — leave it alone
    }
}

/// Delete audio files older than RETENTION_DAYS from every meeting folder.
/// Transcripts and metadata are never touched.
fn prune_recordings_dir(dir: &Path) {
    let cutoff = Duration::from_secs(RETENTION_DAYS * 24 * 60 * 60);
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            info!("cleanup: recordings dir {dir:?} not readable ({e}) — nothing to prune");
            return;
        }
    };
    let mut removed = 0usize;
    let mut freed: u64 = 0;
    for entry in entries.flatten() {
        let folder = entry.path();
        if !folder.is_dir() {
            continue;
        }
        for name in PRUNABLE {
            let f = folder.join(name);
            if f.exists() && is_older_than(&f, cutoff) {
                let size = std::fs::metadata(&f).map(|m| m.len()).unwrap_or(0);
                match std::fs::remove_file(&f) {
                    Ok(()) => {
                        removed += 1;
                        freed += size;
                    }
                    Err(e) => warn!("cleanup: failed to delete {}: {e}", f.display()),
                }
            }
        }
    }
    if removed > 0 {
        info!(
            "cleanup: pruned {removed} audio file(s) older than {RETENTION_DAYS} days ({:.1} MB freed)",
            freed as f64 / 1_048_576.0
        );
    }
}

/// Spawn the startup retention pass (non-blocking; failures only log).
pub fn init<R: Runtime>(app: &AppHandle<R>) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let dir = load_recording_preferences(&app)
            .await
            .map(|p| p.save_folder)
            .unwrap_or_else(|_| super::recording_preferences::get_default_recordings_folder());
        // Blocking fs walk off the async runtime's core threads.
        let _ = tokio::task::spawn_blocking(move || prune_recordings_dir(&dir)).await;
    });
}
