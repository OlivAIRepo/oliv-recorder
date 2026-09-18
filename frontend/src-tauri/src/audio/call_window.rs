//! Reads what WhatsApp Desktop shows about a call, via the Accessibility API.
//!
//! WhatsApp exposes nothing about a call through the APIs we already use: the
//! mic/speaker flags cannot even tell a CALL from a VOICE NOTE (it runs both for
//! a voice note too — measured), and `CGWindowListCopyWindowInfo` returns no
//! window name for WhatsApp at all, though it does for other apps. The
//! Accessibility tree does carry it:
//!
//!     window title : "<contact name> - WhatsApp voice call"
//!                    "<phone number> - WhatsApp voice call"   (unsaved contact)
//!     chat entry   : "Voice call , 11 sec, 3:02 AM, Sent to <contact name>"
//!                    "Missed voice call, 10:14 AM, Received from <phone number>"
//!
//! That is who, which direction, voice or video, and how long. An outgoing call
//! is the case that needs it most: the server's WhatsApp event feed carries no
//! record of one, so without this the call is invisible to the backend.
//!
//! Both strings go to the server RAW. They are localized, carry invisible
//! direction marks and space their digits for VoiceOver, so parsing belongs
//! where it can be fixed without an app release.
//!
//! Timing matters: the call window is DESTROYED when the call ends, so the title
//! has to be caught while the call is up. The chat entry outlives the call, so
//! it is read once at the end.

//! Compiles on every platform: the macOS body is gated below and the other
//! platforms get stubs, so the Tauri command list and the poll need no `cfg`.

#[cfg(target_os = "macos")]
mod mac {

use std::ffi::c_void;
use std::sync::Mutex;

use core_foundation::array::{CFArrayGetCount, CFArrayGetValueAtIndex, CFArrayRef};
use core_foundation::base::{CFRelease, CFTypeRef, TCFType};
use core_foundation::string::{CFString, CFStringRef};

type AXUIElementRef = *const c_void;
type AXError = i32;
const AX_SUCCESS: AXError = 0;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: CFTypeRef) -> bool;
    fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> AXError;
}

/// PID of WhatsApp Desktop, when it is running.
pub fn whatsapp_pid() -> Option<i32> {
    use cidre::ns;
    ns::Workspace::shared()
        .running_apps()
        .iter()
        .find(|a| {
            a.bundle_id()
                .map(|b| b.to_string().to_lowercase().contains("whatsapp"))
                .unwrap_or(false)
        })
        .and_then(|a| a.pid().ok())
}

/// Whether this app may read other apps' UI. False until the user ticks us in
/// System Settings → Privacy & Security → Accessibility.
pub fn is_trusted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

/// Show the system Accessibility prompt. Returns whether we are trusted after.
pub fn request_trust() -> bool {
    use core_foundation::base::TCFType;
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::number::CFBoolean;
    if is_trusted() {
        return true;
    }
    // kAXTrustedCheckOptionPrompt — the system shows its own dialog and opens
    // System Settings; there is no way to grant it programmatically.
    let key = CFString::new("AXTrustedCheckOptionPrompt");
    let options = CFDictionary::from_CFType_pairs(&[(key, CFBoolean::true_value())]);
    unsafe { AXIsProcessTrustedWithOptions(options.as_CFTypeRef()) }
}

/// Copy one attribute off an element, as a string. None when absent or not a
/// string (AX returns arrays, numbers and element refs from the same call).
fn attr_string(element: AXUIElementRef, name: &str) -> Option<String> {
    let key = CFString::new(name);
    let mut value: CFTypeRef = std::ptr::null();
    let err = unsafe {
        AXUIElementCopyAttributeValue(element, key.as_concrete_TypeRef(), &mut value)
    };
    if err != AX_SUCCESS || value.is_null() {
        return None;
    }
    // Only interpret it if it really is a CFString; releasing either way.
    let s = unsafe {
        let cf = value as CFStringRef;
        let out = CFString::wrap_under_get_rule(cf).to_string();
        CFRelease(value);
        out
    };
    (!s.is_empty()).then_some(s)
}

/// Copy an attribute that is an array of elements (windows, children).
fn attr_elements(element: AXUIElementRef, name: &str) -> Vec<AXUIElementRef> {
    let key = CFString::new(name);
    let mut value: CFTypeRef = std::ptr::null();
    let err = unsafe {
        AXUIElementCopyAttributeValue(element, key.as_concrete_TypeRef(), &mut value)
    };
    if err != AX_SUCCESS || value.is_null() {
        return Vec::new();
    }
    let array = value as CFArrayRef;
    let count = unsafe { CFArrayGetCount(array) };
    let mut out = Vec::with_capacity(count as usize);
    for i in 0..count {
        let item = unsafe { CFArrayGetValueAtIndex(array, i) } as AXUIElementRef;
        if !item.is_null() {
            out.push(item);
        }
    }
    // NOTE: the array is released, its elements are not — they stay valid for
    // the life of the array's owner, which is what every AX example relies on.
    unsafe { CFRelease(value) };
    out
}

/// True when a window title is WhatsApp's CALL window rather than its main one.
/// Deliberately loose: the words are localized, the app name is not.
fn is_call_window(title: &str) -> bool {
    let t = title.to_lowercase();
    t.contains("whatsapp") && t.contains(" - ")
}

/// WhatsApp's call-window title, if a call is on screen right now.
///
/// Cheap — one AX call for the window list plus one per window, no tree walk —
/// so it is safe on the mic monitor's existing poll.
pub fn call_window_title(pid: i32) -> Option<String> {
    if !is_trusted() {
        return None;
    }
    let app = unsafe { AXUIElementCreateApplication(pid) };
    if app.is_null() {
        return None;
    }
    let found = attr_elements(app, "AXWindows")
        .into_iter()
        .filter_map(|w| attr_string(w, "AXTitle"))
        .find(|t| is_call_window(t));
    unsafe { CFRelease(app as CFTypeRef) };
    found
}

/// The most recent call line WhatsApp has written into the open chat.
///
/// This IS a tree walk and costs tens of milliseconds, so it runs once when a
/// recording ends — never on the poll. Bounded hard: a runaway tree must not
/// hang the stop path.
pub fn call_chat_entry(pid: i32) -> Option<String> {
    if !is_trusted() {
        return None;
    }
    let app = unsafe { AXUIElementCreateApplication(pid) };
    if app.is_null() {
        return None;
    }

    const MAX_ELEMENTS: usize = 600;
    let mut queue: Vec<AXUIElementRef> = attr_elements(app, "AXWindows");
    let mut seen = 0usize;
    let mut best: Option<String> = None;

    while let Some(element) = queue.pop() {
        seen += 1;
        if seen > MAX_ELEMENTS {
            break;
        }
        if let Some(value) = attr_string(element, "AXValue") {
            // WhatsApp writes the call entry as the accessibility label of the
            // chat bubble. Match on the app-independent shape — a call word and
            // a direction word — rather than a full localized sentence.
            let v = value.to_lowercase();
            let is_call_line = (v.contains("call") || v.contains("llamada") || v.contains("appel"))
                && !v.contains("voice message")
                && !v.contains("voice note");
            if is_call_line {
                best = Some(value);
                break;
            }
        }
        queue.extend(attr_elements(element, "AXChildren"));
    }

    unsafe { CFRelease(app as CFTypeRef) };
    best
}

// ---------------------------------------------------------------------------
// Capture for the recording in flight
// ---------------------------------------------------------------------------
// The call window dies with the call, so the title is grabbed the first time the
// poll sees it and kept until the session ends.

static CAPTURED_TITLE: Mutex<Option<String>> = Mutex::new(None);

/// Called from the mic monitor's poll while a recording is running.
/// Cheap and idempotent: once a title is held, it does nothing.
pub fn observe(pid: i32) {
    if CAPTURED_TITLE.lock().unwrap().is_some() {
        return;
    }
    if let Some(title) = call_window_title(pid) {
        log::info!("call_window: captured '{title}'");
        *CAPTURED_TITLE.lock().unwrap() = Some(title);
    }
}

/// The title captured during this recording, if any.
pub fn captured_title() -> Option<String> {
    CAPTURED_TITLE.lock().unwrap().clone()
}

/// Clear between recordings, so one call's counterparty can never be reported
/// for the next.
pub fn reset() {
    *CAPTURED_TITLE.lock().unwrap() = None;
}

} // mod mac

#[cfg(target_os = "macos")]
pub use mac::*;

// ---------------------------------------------------------------------------
// Other platforms: WhatsApp exposes this through UIAutomation on Windows, which
// is not built yet. Everything reports "nothing seen", so the server simply gets
// no call details and falls back to the time-only match.
// ---------------------------------------------------------------------------

#[cfg(not(target_os = "macos"))]
pub fn whatsapp_pid() -> Option<i32> { None }

#[cfg(not(target_os = "macos"))]
pub fn is_trusted() -> bool { false }

#[cfg(not(target_os = "macos"))]
pub fn call_window_title(_pid: i32) -> Option<String> { None }

#[cfg(not(target_os = "macos"))]
pub fn call_chat_entry(_pid: i32) -> Option<String> { None }

#[cfg(not(target_os = "macos"))]
pub fn observe(_pid: i32) {}

#[cfg(not(target_os = "macos"))]
pub fn captured_title() -> Option<String> { None }

#[cfg(not(target_os = "macos"))]
pub fn reset() {}

#[cfg(not(target_os = "macos"))]
pub fn request_trust() -> bool { false }

// ---------------------------------------------------------------------------
// Tauri commands. Declared once, at the top level, so the handler list needs no
// `cfg` and the paths in `generate_handler!` are literal.
// ---------------------------------------------------------------------------

/// Ask for the Accessibility permission (macOS shows its own dialog).
///
/// Without it we cannot read WhatsApp's call window, so an OUTGOING call has no
/// counterparty and no direction — and the server sees no event for one either,
/// so the recording stays unmatched. Everything else keeps working: this is
/// additive, and every read returns None when the permission is absent.
#[tauri::command]
pub fn oliv_request_accessibility() -> bool {
    request_trust()
}

/// Whether the permission is currently granted — for the settings screen.
#[tauri::command]
pub fn oliv_accessibility_granted() -> bool {
    is_trusted()
}
