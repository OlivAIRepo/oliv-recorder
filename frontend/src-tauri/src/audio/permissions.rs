// macOS audio permissions handling
use anyhow::Result;
use log::{info, warn, error};

#[cfg(target_os = "macos")]
use std::process::Command;

// CoreGraphics (already linked by the core-graphics crate). On modern macOS the
// system-audio capture used by the Core Audio tap is gated by the "Screen &
// System Audio Recording" permission; these give a real NON-prompting check and
// a native prompt for it.
#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

/// Real, non-prompting check for the system-audio recording permission required
/// by the Core Audio tap. (Previously hardcoded to `true`, which made Settings
/// falsely report it as granted.)
#[cfg(target_os = "macos")]
pub fn check_screen_recording_permission() -> bool {
    unsafe { CGPreflightScreenCaptureAccess() }
}

#[cfg(not(target_os = "macos"))]
pub fn check_screen_recording_permission() -> bool {
    true // Not required on other platforms
}

/// Real, non-prompting check for the microphone (TCC) permission. Mirrors the
/// screen-recording check: queries AVCaptureDevice's authorization status rather
/// than inferring from "do input devices exist" (input devices are enumerable
/// without mic consent, which made Settings falsely report it as granted).
#[cfg(target_os = "macos")]
pub fn check_microphone_permission() -> bool {
    use cidre::av;
    matches!(
        av::CaptureDevice::authorization_status_for_media_type(av::MediaType::audio()),
        Ok(av::AuthorizationStatus::Authorized)
    )
}

#[cfg(not(target_os = "macos"))]
pub fn check_microphone_permission() -> bool {
    true // Gated by the OS prompt at capture time on other platforms
}

/// Tauri command: non-prompting microphone permission status.
#[tauri::command]
pub async fn check_microphone_permission_command() -> bool {
    check_microphone_permission()
}

/// Open System Settings to a specific Privacy pane. macOS only; no-op elsewhere.
#[cfg(target_os = "macos")]
pub fn open_privacy_settings(pane: &str) {
    let _ = Command::new("open")
        .arg(format!(
            "x-apple.systempreferences:com.apple.preference.security?Privacy_{pane}"
        ))
        .spawn();
}

#[cfg(not(target_os = "macos"))]
pub fn open_privacy_settings(_pane: &str) {}

/// Tauri command: open the Microphone privacy pane (for the already-denied case,
/// where macOS will never re-show the prompt).
#[tauri::command]
pub async fn open_microphone_settings_command() {
    open_privacy_settings("Microphone");
}

/// Tauri command: open the Screen & System Audio Recording privacy pane.
#[tauri::command]
pub async fn open_screen_recording_settings_command() {
    open_privacy_settings("ScreenCapture");
}

/// Request Audio Capture permission from the user
/// This will open System Settings to the Privacy & Security page
#[cfg(target_os = "macos")]
pub fn request_screen_recording_permission() -> Result<()> {
    // Native prompt — grants in-place when the permission is undetermined.
    let granted = unsafe { CGRequestScreenCaptureAccess() };
    info!("🔐 CGRequestScreenCaptureAccess -> {granted}");
    if !granted {
        // Already-determined/denied: the prompt won't re-appear, so open the
        // exact Privacy pane for the user to toggle it manually.
        let _ = Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture")
            .spawn();
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn request_screen_recording_permission() -> Result<()> {
    Ok(()) // Not required on other platforms
}

/// Check and request Audio Capture permission if not granted
/// Returns true if permission is granted, false otherwise
pub fn ensure_screen_recording_permission() -> bool {
    if check_screen_recording_permission() {
        return true;
    }

    warn!("Audio Capture permission not granted - requesting...");

    if let Err(e) = request_screen_recording_permission() {
        error!("Failed to request Audio Capture permission: {}", e);
        return false;
    }

    false // Permission will be granted after restart
}

/// Tauri command to check Screen Recording permission
#[tauri::command]
pub async fn check_screen_recording_permission_command() -> bool {
    check_screen_recording_permission()
}

/// Tauri command to request Screen Recording permission
#[tauri::command]
pub async fn request_screen_recording_permission_command() -> Result<(), String> {
    request_screen_recording_permission()
        .map_err(|e| e.to_string())
}

/// Trigger system audio permission request and verify it was granted
/// Returns Ok(true) if permission granted (tap created successfully), Ok(false) if denied
#[cfg(target_os = "macos")]
pub fn trigger_system_audio_permission() -> Result<bool> {
    info!("🔐 Triggering Audio Capture permission request...");

    // Create AND briefly START the tap. Tap *creation* alone does NOT trigger
    // the Audio Capture TCC prompt — it only fires when the tap starts streaming
    // (stream() → start_device), which is what the real recording does. So we
    // must start the stream here, then tear it down, to actually request/grant.
    match crate::audio::capture::CoreAudioCapture::new() {
        Ok(capture) => match capture.stream() {
            Ok(stream) => {
                // Hold the stream briefly so the prompt is presented, then stop.
                std::thread::sleep(std::time::Duration::from_millis(800));
                drop(stream);
                info!("✅ Core Audio tap started — Audio Capture prompt triggered if needed");
                Ok(true)
            }
            Err(e) => {
                let msg = e.to_string().to_lowercase();
                if msg.contains("permission") || msg.contains("denied") {
                    info!("🔐 Audio Capture permission denied");
                    return Ok(false);
                }
                warn!("⚠️ Failed to start Core Audio stream for permission: {}", e);
                Ok(false)
            }
        },
        Err(e) => {
            let msg = e.to_string().to_lowercase();
            if msg.contains("permission") || msg.contains("denied") {
                info!("🔐 Audio Capture permission denied");
                return Ok(false);
            }
            warn!("⚠️ Failed to create Core Audio tap: {}", e);
            Ok(false)
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn trigger_system_audio_permission() -> Result<bool> {
    // System audio permissions not required on other platforms
    info!("System audio permissions not required on this platform");
    Ok(true)
}

/// Tauri command to trigger system audio permission request
/// Returns true if permission was granted (stream created), false if denied
#[tauri::command]
pub async fn trigger_system_audio_permission_command() -> Result<bool, String> {
    // Run in blocking task to avoid blocking the async runtime
    tokio::task::spawn_blocking(|| {
        trigger_system_audio_permission()
    })
    .await
    .map_err(|e| format!("Task join error: {}", e))?
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_permission() {
        let has_permission = check_screen_recording_permission();
        println!("Has Screen Recording permission: {}", has_permission);
    }
}