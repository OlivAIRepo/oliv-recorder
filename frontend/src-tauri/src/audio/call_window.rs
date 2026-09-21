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

use std::collections::HashSet;
use std::ffi::c_void;
use std::sync::Mutex;

use core_foundation::array::{CFArrayGetCount, CFArrayGetTypeID, CFArrayGetValueAtIndex, CFArrayRef};
use core_foundation::base::{CFGetTypeID, CFRelease, CFRetain, CFTypeRef, TCFType};
use core_foundation::string::{CFString, CFStringGetTypeID, CFStringRef};

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
        .map(|a| a.pid())
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
    use core_foundation::boolean::CFBoolean;
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
    // CHECK the type before casting. One attribute name returns different types
    // on different elements — `AXValue` is a string on a label but a number, a
    // boolean or another element elsewhere — and casting one of those to
    // CFStringRef throws an Objective-C exception, which Rust cannot catch and
    // which aborts the process ("Rust cannot catch foreign exceptions").
    let is_string = unsafe { CFGetTypeID(value) == CFStringGetTypeID() };
    if !is_string {
        unsafe { CFRelease(value) };
        return None;
    }
    let s = unsafe {
        let out = CFString::wrap_under_get_rule(value as CFStringRef).to_string();
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
    // Same reasoning as `attr_string`: `AXChildren` is an array on most elements
    // but not on all of them, and treating a non-array as one aborts.
    if unsafe { CFGetTypeID(value) != CFArrayGetTypeID() } {
        unsafe { CFRelease(value) };
        return Vec::new();
    }
    let array = value as CFArrayRef;
    let count = unsafe { CFArrayGetCount(array) };
    let mut out = Vec::with_capacity(count as usize);
    for i in 0..count {
        let item = unsafe { CFArrayGetValueAtIndex(array, i) } as AXUIElementRef;
        if !item.is_null() {
            // RETAIN each element before dropping the array. The array owns
            // them, so releasing it can free them — reading one afterwards is a
            // use-after-free, which macOS ends with SIGTRAP. Ownership passes to
            // the caller, which must `release_elements` when done.
            unsafe { CFRetain(item as CFTypeRef) };
            out.push(item);
        }
    }
    unsafe { CFRelease(value) };
    out
}

/// Release elements handed back by `attr_elements`.
fn release_elements(elements: &[AXUIElementRef]) {
    for e in elements {
        unsafe { CFRelease(*e as CFTypeRef) };
    }
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
    let windows = attr_elements(app, "AXWindows");
    let found = windows
        .iter()
        .filter_map(|w| attr_string(*w, "AXTitle"))
        .find(|t| is_call_window(t));
    release_elements(&windows);
    unsafe { CFRelease(app as CFTypeRef) };
    found
}

/// Call lines WhatsApp has written into the open chat, in tree order.
///
/// Returns ALL of them, not the first match: the chat shows history, so the
/// first call-shaped string is often days old — a 10-second call can read back
/// as "Outgoing, voice, unanswered, 1 call" against an entry written days
/// earlier. The server decides which
/// one (if any) describes the call just recorded, by checking the timestamp each
/// line carries.
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
    let mut found: Vec<String> = Vec::new();

    while let Some(element) = queue.pop() {
        seen += 1;
        if seen > MAX_ELEMENTS {
            unsafe { CFRelease(element as CFTypeRef) };
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
                found.push(value);
            }
        }
        queue.extend(attr_elements(element, "AXChildren"));
        unsafe { CFRelease(element as CFTypeRef) };
    }

    release_elements(&queue);   // whatever the walk did not reach
    unsafe { CFRelease(app as CFTypeRef) };
    if found.is_empty() {
        return None;
    }
    Some(found.iter().rev().take(6).cloned().collect::<Vec<_>>().join(" | "))
}

/// Every call line currently in the chat, as a set. Used as a BEFORE snapshot so
/// the end-of-call read can report only what appeared while we were recording.
fn call_lines(pid: i32) -> HashSet<String> {
    match call_chat_entry(pid) {
        Some(joined) => joined.split(" | ").map(|s| s.to_string()).collect(),
        None => HashSet::new(),
    }
}

/// Snapshot the chat's existing call lines at the START of a recording.
///
/// The chat shows history, so at the end there is no way to tell which line is
/// this call by reading alone — a five-day-old entry looks the same. Diffing
/// against this snapshot identifies it by construction: the line that was not
/// there before is the one that just happened.
pub fn snapshot_chat(pid: i32) {
    let lines = call_lines(pid);
    log::info!("call_window: baseline of {} existing call line(s)", lines.len());
    *BASELINE.lock().unwrap() = Some(lines);
}

/// Call lines that appeared since `snapshot_chat` — i.e. this call's.
///
/// Falls back to the newest few lines when the diff is empty, so a missed
/// baseline (app started mid-call, permission granted late) degrades to the old
/// behaviour rather than losing the entry altogether; the server still checks
/// the timestamp before believing one.
pub fn new_chat_lines(pid: i32) -> Option<String> {
    let current = call_lines(pid);
    let baseline = BASELINE.lock().unwrap().clone().unwrap_or_default();
    let mut fresh: Vec<String> = current.difference(&baseline).cloned().collect();
    if fresh.is_empty() {
        log::info!("call_window: no new chat line; falling back to recent history");
        return call_chat_entry(pid);
    }
    // Longest first: WhatsApp's completed-call line carries the most detail
    // ("Voice call , 11 sec, 3:02 AM, Sent to X") while a ring-only line is terse.
    fresh.sort_by_key(|l| std::cmp::Reverse(l.len()));
    Some(fresh.join(" | "))
}

// ---------------------------------------------------------------------------
// Capture for the recording in flight
// ---------------------------------------------------------------------------
// The call window dies with the call, so the title is grabbed the first time the
// poll sees it and kept until the session ends.

static CAPTURED_TITLE: Mutex<Option<String>> = Mutex::new(None);

/// Call lines already in the chat when this recording began.
static BASELINE: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// Called from the mic monitor's poll while a recording is running.
/// Cheap and idempotent: once a title is held, it does nothing.
pub fn observe(pid: i32) {
    if CAPTURED_TITLE.lock().unwrap().is_some() {
        return;
    }
    // Say so, once, when the permission is missing. Without this the capture
    // just returns nothing and looks identical to "no call on screen" — which
    // cost real debugging time after a re-sign silently dropped the grant.
    if !is_trusted() {
        use std::sync::atomic::{AtomicBool, Ordering};
        static WARNED: AtomicBool = AtomicBool::new(false);
        if !WARNED.swap(true, Ordering::Relaxed) {
            log::warn!(
                "call_window: no Accessibility permission — cannot read WhatsApp's \
                 call window, so an outgoing call has no counterparty"
            );
        }
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
    *BASELINE.lock().unwrap() = None;
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
pub fn snapshot_chat(_pid: i32) {}

#[cfg(not(target_os = "macos"))]
pub fn new_chat_lines(_pid: i32) -> Option<String> { None }

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
