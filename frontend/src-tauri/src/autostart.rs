//! Launch-on-login enforcement.
//!
//! macOS: Apple's modern `SMAppService` API (macOS 13+). The 0.3.32 attempt
//! used a legacy LaunchAgent plist, which Background Task Management silently
//! refuses to load once the user disables/removes the item in System Settings —
//! rewriting the file does nothing. `SMAppService` registration is the
//! supported path: re-registering on every launch restores the login item in
//! the normal cases, and when the OS refuses (an explicit user disable is
//! final on macOS — apps cannot silently override it) the status is
//! detectable, so the Settings page shows a one-click "re-enable" prompt that
//! opens the Login Items pane.
//!
//! Windows/Linux: tauri-plugin-autostart (registry Run key / .desktop entry),
//! re-enabled on every launch.
//!
//! There is deliberately NO settings toggle — launch-on-login is part of the
//! product; the way out is Settings → Uninstall.

use tauri::{AppHandle, Runtime};

/// Re-assert the login item. Called on every (release-build) startup.
pub fn enforce<R: Runtime>(app: &AppHandle<R>) {
    #[cfg(target_os = "macos")]
    {
        let _ = app;
        use objc2_service_management::{SMAppService, SMAppServiceStatus};
        // Clean up the legacy 0.3.32 LaunchAgent so two mechanisms never fight.
        if let Some(home) = dirs::home_dir() {
            let _ = std::fs::remove_file(home.join("Library/LaunchAgents/Oliv AI.plist"));
        }
        unsafe {
            let service = SMAppService::mainAppService();
            if service.status() != SMAppServiceStatus::Enabled {
                match service.registerAndReturnError() {
                    Ok(()) => log::info!("autostart: registered login item (SMAppService)"),
                    Err(e) => log::warn!("autostart: register failed: {e:?}"),
                }
            }
            log::info!("autostart: status = {:?}", service.status());
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        use tauri_plugin_autostart::ManagerExt;
        match app.autolaunch().enable() {
            Ok(()) => log::info!("autostart: enabled (launch on login)"),
            Err(e) => log::warn!("autostart: enable failed: {e}"),
        }
    }
}

/// Remove the login item (used by uninstall).
pub fn unregister<R: Runtime>(app: &AppHandle<R>) {
    #[cfg(target_os = "macos")]
    {
        let _ = app;
        use objc2_service_management::SMAppService;
        unsafe {
            let service = SMAppService::mainAppService();
            if let Err(e) = service.unregisterAndReturnError() {
                log::warn!("autostart: unregister failed: {e:?}");
            }
        }
        if let Some(home) = dirs::home_dir() {
            let _ = std::fs::remove_file(home.join("Library/LaunchAgents/Oliv AI.plist"));
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        use tauri_plugin_autostart::ManagerExt;
        if let Err(e) = app.autolaunch().disable() {
            log::warn!("autostart: disable failed: {e}");
        }
    }
}

/// Whether launch-on-login is currently active. The Settings page shows a
/// re-enable prompt when false (only the user can flip it back on macOS).
#[tauri::command]
pub fn autostart_status<R: Runtime>(app: AppHandle<R>) -> bool {
    #[cfg(target_os = "macos")]
    {
        let _ = app;
        use objc2_service_management::{SMAppService, SMAppServiceStatus};
        unsafe { SMAppService::mainAppService().status() == SMAppServiceStatus::Enabled }
    }
    #[cfg(not(target_os = "macos"))]
    {
        use tauri_plugin_autostart::ManagerExt;
        app.autolaunch().is_enabled().unwrap_or(false)
    }
}

/// Open the OS surface where the user can re-enable the login item
/// (macOS: System Settings → General → Login Items).
#[tauri::command]
pub fn open_login_items_settings() {
    #[cfg(target_os = "macos")]
    unsafe {
        objc2_service_management::SMAppService::openSystemSettingsLoginItems();
    }
}
