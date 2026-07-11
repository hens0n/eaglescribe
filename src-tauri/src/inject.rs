//! Insert text into the focused application via clipboard + paste shortcut.
//!
//! **macOS:** HIToolbox / Text Input Source APIs used by layout-dependent keys
//! (`Key::Unicode`) must run on the main thread. Calling them from the
//! transcription worker thread aborts with `EXC_BREAKPOINT` /
//! `_dispatch_assert_queue_fail` (see DiagnosticReports for eaglescribe).
//!
//! We therefore:
//! 1. Optionally snapshot the previous clipboard text.
//! 2. Set the clipboard on any thread (safe).
//! 3. Simulate Cmd/Ctrl+V on the **main thread**, using a physical keycode
//!    (`Key::Other`) so we never hit layout/TSM lookups.
//! 4. After a successful paste (and a short delay), restore the snapshot so
//!    dictation does not leave the transcript stuck on the system clipboard.

use crate::error::{AppError, AppResult};
use arboard::Clipboard;
use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Runtime};

/// How long to wait after a successful paste before restoring the previous
/// clipboard. Gives the target app time to read our temporary paste buffer.
pub const CLIPBOARD_RESTORE_DELAY: Duration = Duration::from_millis(200);

/// Virtual keycode for the "V" key (ANSI), layout-independent on macOS.
/// `kVK_ANSI_V` — used so we never call TSM/layout APIs.
#[cfg(target_os = "macos")]
const KEYCODE_V: u16 = 0x09;
/// `kVK_ANSI_C`
#[cfg(target_os = "macos")]
const KEYCODE_C: u16 = 0x08;

/// Whether to restore a prior clipboard snapshot after inject.
///
/// Restore only when: the user enabled it, paste succeeded (so the target
/// app has the text), and we actually captured a previous text value.
/// On paste failure the transcript stays on the clipboard for manual paste.
pub fn should_restore_clipboard(
    restore_enabled: bool,
    pasted: bool,
    previous: &Option<String>,
) -> bool {
    restore_enabled && pasted && previous.is_some()
}

/// Copy `text` to the clipboard and paste into the focused app.
///
/// When `restore_clipboard` is true and paste succeeds, the previous
/// clipboard text (if readable) is put back after [`CLIPBOARD_RESTORE_DELAY`].
/// If paste simulation fails, text remains on the clipboard for manual paste
/// and is **not** restored.
pub fn inject_text<R: Runtime>(
    app: &AppHandle<R>,
    text: &str,
    restore_clipboard: bool,
) -> AppResult<InjectResult> {
    if text.is_empty() {
        return Err(AppError::from("Nothing to inject (empty transcript)"));
    }

    let previous = if restore_clipboard {
        read_clipboard_text().ok()
    } else {
        None
    };

    copy_to_clipboard(text)?;

    // Clipboard settle
    thread::sleep(Duration::from_millis(40));

    let paste_ok = run_paste_on_main_thread(app)?;

    let mut restored = false;
    let mut restore_failed = false;
    if should_restore_clipboard(restore_clipboard, paste_ok, &previous) {
        // Wait so the focused app can consume our paste buffer first.
        // `should_restore_clipboard` requires `previous` is Some.
        let prev = previous.expect("guarded by should_restore_clipboard");
        thread::sleep(CLIPBOARD_RESTORE_DELAY);
        match copy_to_clipboard(&prev) {
            Ok(()) => restored = true,
            Err(e) => {
                eprintln!("[eaglescribe] clipboard restore failed: {e}");
                restore_failed = true;
            }
        }
    }

    Ok(InjectResult {
        pasted: paste_ok,
        restored,
        restore_failed,
        text: text.to_string(),
    })
}

/// Clipboard-only fallback (no key simulation). Safe on any thread.
pub fn copy_to_clipboard(text: &str) -> AppResult<()> {
    let mut clipboard = Clipboard::new()
        .map_err(|e| AppError::from(format!("Clipboard unavailable: {e}")))?;
    clipboard
        .set_text(text.to_string())
        .map_err(|e| AppError::from(format!("Failed to set clipboard: {e}")))?;
    Ok(())
}

pub fn read_clipboard_text() -> AppResult<String> {
    let mut clipboard = Clipboard::new()
        .map_err(|e| AppError::from(format!("Clipboard unavailable: {e}")))?;
    clipboard
        .get_text()
        .map_err(|e| AppError::from(format!("Failed to read clipboard: {e}")))
}

/// Capture the current selection by simulating copy on the main thread.
///
/// Returns selected text (may be empty if nothing was selected). Best-effort:
/// restores the previous clipboard contents when possible.
pub fn capture_selection<R: Runtime>(app: &AppHandle<R>) -> AppResult<String> {
    let previous = read_clipboard_text().ok();

    // Clear so we can detect a failed copy (selection empty).
    let _ = copy_to_clipboard("");

    run_copy_on_main_thread(app)?;
    thread::sleep(Duration::from_millis(80));

    let selected = read_clipboard_text().unwrap_or_default();

    if let Some(prev) = previous {
        // Don't restore if we successfully captured selection — user expects
        // the rewritten text path next. Only restore when selection was empty
        // so we don't wipe their clipboard with "".
        if selected.trim().is_empty() {
            let _ = copy_to_clipboard(&prev);
        }
    }

    Ok(selected)
}

fn run_copy_on_main_thread<R: Runtime>(app: &AppHandle<R>) -> AppResult<()> {
    let (tx, rx) = mpsc::channel();
    app.run_on_main_thread(move || {
        let result = simulate_copy();
        let _ = tx.send(result);
    })
    .map_err(|e| AppError::from(format!("Failed to schedule copy on main thread: {e}")))?;

    match rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(AppError::from("Copy timed out waiting for main thread")),
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct InjectResult {
    pub pasted: bool,
    /// True when the previous clipboard text was written back after paste.
    pub restored: bool,
    /// True when restore was attempted after paste but writing it back failed
    /// (transcript is left on the clipboard).
    pub restore_failed: bool,
    pub text: String,
}

fn run_paste_on_main_thread<R: Runtime>(app: &AppHandle<R>) -> AppResult<bool> {
    let (tx, rx) = mpsc::channel();

    app.run_on_main_thread(move || {
        let result = simulate_paste();
        let _ = tx.send(result);
    })
    .map_err(|e| AppError::from(format!("Failed to schedule paste on main thread: {e}")))?;

    // Wait for main-thread work (paste is fast).
    match rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(())) => Ok(true),
        Ok(Err(e)) => {
            eprintln!("[eaglescribe] paste simulation failed (text is on clipboard): {e}");
            Ok(false)
        }
        Err(_) => {
            eprintln!("[eaglescribe] paste timed out waiting for main thread (text is on clipboard)");
            Ok(false)
        }
    }
}

/// Simulate platform paste shortcut. **Must run on the main thread on macOS.**
fn simulate_paste() -> AppResult<()> {
    chord_mod_key('v', KEYCODE_V_OR_UNICODE)
}

/// Simulate platform copy shortcut. **Must run on the main thread on macOS.**
fn simulate_copy() -> AppResult<()> {
    chord_mod_key('c', KEYCODE_C_OR_UNICODE)
}

#[cfg(target_os = "macos")]
const KEYCODE_V_OR_UNICODE: u32 = KEYCODE_V as u32;
#[cfg(target_os = "macos")]
const KEYCODE_C_OR_UNICODE: u32 = KEYCODE_C as u32;

#[cfg(not(target_os = "macos"))]
const KEYCODE_V_OR_UNICODE: u32 = 0;
#[cfg(not(target_os = "macos"))]
const KEYCODE_C_OR_UNICODE: u32 = 0;

fn chord_mod_key(letter: char, macos_keycode: u32) -> AppResult<()> {
    let mut enigo = Enigo::new(&Settings::default())
        .map_err(|e| AppError::from(format!("Input simulation unavailable: {e}")))?;

    #[cfg(target_os = "macos")]
    {
        let _ = letter;
        enigo
            .key(Key::Meta, Direction::Press)
            .map_err(|e| AppError::from(format!("Key chord failed: {e}")))?;
        enigo
            .key(Key::Other(macos_keycode), Direction::Click)
            .map_err(|e| AppError::from(format!("Key chord failed: {e}")))?;
        enigo
            .key(Key::Meta, Direction::Release)
            .map_err(|e| AppError::from(format!("Key chord failed: {e}")))?;
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = macos_keycode;
        enigo
            .key(Key::Control, Direction::Press)
            .map_err(|e| AppError::from(format!("Key chord failed: {e}")))?;
        enigo
            .key(Key::Unicode(letter), Direction::Click)
            .map_err(|e| AppError::from(format!("Key chord failed: {e}")))?;
        enigo
            .key(Key::Control, Direction::Release)
            .map_err(|e| AppError::from(format!("Key chord failed: {e}")))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: layout-dependent Unicode paste off-main used to SIGTRAP on macOS.
    /// Physical keycode path must not call TSM and must not abort the process.
    #[test]
    fn simulate_paste_from_background_thread_does_not_abort() {
        let handle = thread::spawn(|| {
            // We only assert the process survives. On CI without display this may
            // return Err, which is fine — the bug was an abort, not a Result.
            let _ = simulate_paste();
        });
        assert!(
            handle.join().is_ok(),
            "paste simulation aborted the process (was EXC_BREAKPOINT / TSM off main thread)"
        );
    }

    #[test]
    fn copy_to_clipboard_accepts_text() {
        // May fail in headless CI without pasteboard; skip soft-fail.
        let _ = copy_to_clipboard("eaglescribe-test");
    }

    #[test]
    fn should_restore_only_when_enabled_pasted_and_snapshot_present() {
        let prev = Some("prior".into());
        assert!(should_restore_clipboard(true, true, &prev));
        assert!(!should_restore_clipboard(false, true, &prev));
        assert!(!should_restore_clipboard(true, false, &prev));
        assert!(!should_restore_clipboard(true, true, &None));
        assert!(!should_restore_clipboard(false, false, &None));
    }

    /// Round-trip: after inject-style overwrite, restoring puts prior text back.
    /// Soft-skips when the system pasteboard is unavailable (headless CI).
    #[test]
    fn restore_writes_back_previous_clipboard_text() {
        let marker_prev = "eaglescribe-clipboard-prev-9f3a";
        let marker_inject = "eaglescribe-clipboard-inject-9f3a";

        if copy_to_clipboard(marker_prev).is_err() {
            return;
        }
        let previous = match read_clipboard_text() {
            Ok(t) => t,
            Err(_) => return,
        };
        assert_eq!(previous, marker_prev);

        copy_to_clipboard(marker_inject).expect("set inject text");
        assert_eq!(
            read_clipboard_text().unwrap_or_default(),
            marker_inject
        );

        // Same path as successful-paste restore.
        assert!(should_restore_clipboard(
            true,
            true,
            &Some(previous.clone())
        ));
        copy_to_clipboard(&previous).expect("restore prior");
        assert_eq!(
            read_clipboard_text().unwrap_or_default(),
            marker_prev,
            "previous clipboard text must be restored after successful paste"
        );
    }
}
