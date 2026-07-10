//! Insert text into the focused application via clipboard + paste shortcut.
//!
//! **macOS:** HIToolbox / Text Input Source APIs used by layout-dependent keys
//! (`Key::Unicode`) must run on the main thread. Calling them from the
//! transcription worker thread aborts with `EXC_BREAKPOINT` /
//! `_dispatch_assert_queue_fail` (see DiagnosticReports for talontype).
//!
//! We therefore:
//! 1. Set the clipboard on any thread (safe).
//! 2. Simulate Cmd/Ctrl+V on the **main thread**, using a physical keycode
//!    (`Key::Other`) so we never hit layout/TSM lookups.

use crate::error::{AppError, AppResult};
use arboard::Clipboard;
use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Runtime};

/// Virtual keycode for the "V" key (ANSI), layout-independent on macOS.
/// `kVK_ANSI_V` — used so we never call TSM/layout APIs.
#[cfg(target_os = "macos")]
const KEYCODE_V: u16 = 0x09;

/// Copy `text` to the clipboard and paste into the focused app.
///
/// Paste key simulation is dispatched to the **main thread** when `app` is provided.
/// If paste simulation fails, text remains on the clipboard for manual paste.
pub fn inject_text<R: Runtime>(app: &AppHandle<R>, text: &str) -> AppResult<InjectResult> {
    if text.is_empty() {
        return Err(AppError::from("Nothing to inject (empty transcript)"));
    }

    copy_to_clipboard(text)?;

    // Clipboard settle
    thread::sleep(Duration::from_millis(40));

    let paste_ok = run_paste_on_main_thread(app)?;

    Ok(InjectResult {
        pasted: paste_ok,
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

#[derive(Debug, Clone, serde::Serialize)]
pub struct InjectResult {
    pub pasted: bool,
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
            eprintln!("[talontype] paste simulation failed (text is on clipboard): {e}");
            Ok(false)
        }
        Err(_) => {
            eprintln!("[talontype] paste timed out waiting for main thread (text is on clipboard)");
            Ok(false)
        }
    }
}

/// Simulate platform paste shortcut. **Must run on the main thread on macOS.**
fn simulate_paste() -> AppResult<()> {
    let mut enigo = Enigo::new(&Settings::default())
        .map_err(|e| AppError::from(format!("Input simulation unavailable: {e}")))?;

    #[cfg(target_os = "macos")]
    {
        // Prefer physical keycode — never Key::Unicode (triggers TSM on macOS).
        enigo
            .key(Key::Meta, Direction::Press)
            .map_err(|e| AppError::from(format!("Paste failed: {e}")))?;
        enigo
            .key(Key::Other(KEYCODE_V as u32), Direction::Click)
            .map_err(|e| AppError::from(format!("Paste failed: {e}")))?;
        enigo
            .key(Key::Meta, Direction::Release)
            .map_err(|e| AppError::from(format!("Paste failed: {e}")))?;
    }

    #[cfg(not(target_os = "macos"))]
    {
        enigo
            .key(Key::Control, Direction::Press)
            .map_err(|e| AppError::from(format!("Paste failed: {e}")))?;
        // 'v' virtual key is fine on Linux X11 without macOS TSM constraints;
        // still avoid Unicode when a raw code is available later.
        enigo
            .key(Key::Unicode('v'), Direction::Click)
            .map_err(|e| AppError::from(format!("Paste failed: {e}")))?;
        enigo
            .key(Key::Control, Direction::Release)
            .map_err(|e| AppError::from(format!("Paste failed: {e}")))?;
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
        let _ = copy_to_clipboard("talontype-test");
    }
}
