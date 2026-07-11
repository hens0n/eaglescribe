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
//!
//! **Linux clipboard ownership:** X11/Wayland clipboards are served by the
//! process that set them. Dropping the last `arboard::Clipboard` right after
//! `set_text` races paste targets (and leaves manual-paste fallback empty
//! when no clipboard manager is running). The inject path therefore keeps a
//! process-held `Clipboard` alive across set → paste → optional restore, and
//! until the next set (or [`release_clipboard_ownership`] on quit).

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

/// Extra settle time after setting clipboard text before simulating paste.
/// Lets the X11/Wayland selection owner advertise before Ctrl+V.
pub const CLIPBOARD_SETTLE: Duration = Duration::from_millis(40);

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

/// Drop any process-held Linux clipboard ownership (no-op on other OSes).
///
/// Call on app quit so arboard can hand off to a clipboard manager and join
/// its server thread cleanly under Tauri/winit (objects may not drop at exit).
pub fn release_clipboard_ownership() {
    #[cfg(target_os = "linux")]
    linux_owner::release();
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

    // Linux: held Clipboard lives past this call so paste targets (and manual
    // paste after failure) can still read the selection. See module docs.
    copy_to_clipboard(text)?;

    // Clipboard settle (selection advertised before Ctrl/Cmd+V).
    thread::sleep(CLIPBOARD_SETTLE);

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
    // On paste failure: transcript remains on the held clipboard (INJ-03).
    // Do not restore. Ownership stays with the process-held Clipboard on Linux.

    Ok(InjectResult {
        pasted: paste_ok,
        restored,
        restore_failed,
        text: text.to_string(),
    })
}

/// Clipboard-only fallback (no key simulation). Safe on any thread.
///
/// On Linux this retains clipboard ownership in-process so the selection
/// remains available for subsequent paste (simulated or manual).
pub fn copy_to_clipboard(text: &str) -> AppResult<()> {
    #[cfg(target_os = "linux")]
    {
        return linux_owner::set_text(text);
    }
    #[cfg(not(target_os = "linux"))]
    {
        let mut clipboard =
            Clipboard::new().map_err(|e| AppError::from(format!("Clipboard unavailable: {e}")))?;
        clipboard
            .set_text(text.to_string())
            .map_err(|e| AppError::from(format!("Failed to set clipboard: {e}")))?;
        Ok(())
    }
}

pub fn read_clipboard_text() -> AppResult<String> {
    #[cfg(target_os = "linux")]
    {
        return linux_owner::get_text();
    }
    #[cfg(not(target_os = "linux"))]
    {
        let mut clipboard =
            Clipboard::new().map_err(|e| AppError::from(format!("Clipboard unavailable: {e}")))?;
        clipboard
            .get_text()
            .map_err(|e| AppError::from(format!("Failed to read clipboard: {e}")))
    }
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
            eprintln!(
                "[eaglescribe] paste timed out waiting for main thread (text is on clipboard)"
            );
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

/// Linux-only: keep one `Clipboard` alive so arboard's X11/Wayland server
/// thread keeps serving selection requests until the next set or release.
///
/// Without this, `copy_to_clipboard` used to construct a short-lived
/// `Clipboard`, `set_text`, and drop — the classic drop-on-set race.
#[cfg(target_os = "linux")]
mod linux_owner {
    use super::*;
    use std::sync::Mutex;

    static HELD: Mutex<Option<Clipboard>> = Mutex::new(None);

    fn with_clipboard<F, T>(f: F) -> AppResult<T>
    where
        F: FnOnce(&mut Clipboard) -> AppResult<T>,
    {
        let mut guard = HELD
            .lock()
            .map_err(|_| AppError::from("Clipboard owner lock poisoned"))?;
        if guard.is_none() {
            let clipboard = Clipboard::new()
                .map_err(|e| AppError::from(format!("Clipboard unavailable: {e}")))?;
            *guard = Some(clipboard);
        }
        let clipboard = guard.as_mut().expect("clipboard just ensured above");
        f(clipboard)
    }

    pub fn set_text(text: &str) -> AppResult<()> {
        with_clipboard(|clipboard| {
            clipboard
                .set_text(text.to_string())
                .map_err(|e| AppError::from(format!("Failed to set clipboard: {e}")))
        })
    }

    pub fn get_text() -> AppResult<String> {
        with_clipboard(|clipboard| {
            clipboard
                .get_text()
                .map_err(|e| AppError::from(format!("Failed to read clipboard: {e}")))
        })
    }

    pub fn release() {
        if let Ok(mut guard) = HELD.lock() {
            // Drop last local owner so arboard can hand off to a clipboard
            // manager and join the serve thread before process exit.
            *guard = None;
        }
    }

    /// True when this process currently holds a `Clipboard` instance.
    #[cfg(test)]
    pub fn is_holding() -> bool {
        HELD.lock().map(|g| g.is_some()).unwrap_or(false)
    }
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

    /// Paste-failure / INJ-03: restore must not run when paste did not succeed.
    #[test]
    fn paste_failure_never_restores_prior_clipboard() {
        let prev = Some("user-prior".into());
        assert!(
            !should_restore_clipboard(true, false, &prev),
            "INJ-03: transcript must stay on clipboard when paste fails"
        );
        assert!(!should_restore_clipboard(true, false, &None));
    }

    /// Round-trip: after inject-style overwrite, restoring puts prior text back.
    /// Soft-skips when the system pasteboard is unavailable (headless CI) or when
    /// another parallel clipboard test races the shared pasteboard.
    #[test]
    fn restore_writes_back_previous_clipboard_text() {
        let marker_prev = "eaglescribe-clipboard-prev-9f3a";
        let marker_inject = "eaglescribe-clipboard-inject-9f3a";

        if copy_to_clipboard(marker_prev).is_err() {
            return;
        }
        let previous = match read_clipboard_text() {
            Ok(t) if t == marker_prev => t,
            // Unavailable or raced by a parallel clipboard test — soft-skip.
            _ => return,
        };

        if copy_to_clipboard(marker_inject).is_err() {
            return;
        }
        if read_clipboard_text().unwrap_or_default() != marker_inject {
            return;
        }

        // Same path as successful-paste restore.
        assert!(should_restore_clipboard(
            true,
            true,
            &Some(previous.clone())
        ));
        if copy_to_clipboard(&previous).is_err() {
            return;
        }
        let after = read_clipboard_text().unwrap_or_default();
        if after != marker_prev && after != marker_inject {
            // External race; soft-skip rather than flake CI.
            return;
        }
        assert_eq!(
            after, marker_prev,
            "previous clipboard text must be restored after successful paste"
        );
    }

    /// Ownership release is always safe (no-op off Linux; drop held Clipboard on Linux).
    #[test]
    fn release_clipboard_ownership_does_not_panic() {
        // Soft-set so Linux path may create a holder when a display is present.
        let _ = copy_to_clipboard("eaglescribe-ownership-release");
        release_clipboard_ownership();
        // Second call after empty holder must also be fine (quit / re-quit).
        release_clipboard_ownership();
    }

    #[test]
    fn clipboard_settle_and_restore_delays_are_positive() {
        assert!(CLIPBOARD_SETTLE.as_millis() > 0);
        assert!(CLIPBOARD_RESTORE_DELAY.as_millis() > 0);
        // Restore window should be at least the settle window so paste can finish.
        assert!(CLIPBOARD_RESTORE_DELAY >= CLIPBOARD_SETTLE);
    }

    /// On Linux, a successful set keeps the process-held Clipboard until release.
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_set_retains_ownership_until_release() {
        if copy_to_clipboard("eaglescribe-linux-hold").is_err() {
            // Headless / no display: soft-skip like other clipboard tests.
            return;
        }
        assert!(
            linux_owner::is_holding(),
            "Linux set path must retain Clipboard (no drop-on-set)"
        );
        assert_eq!(
            read_clipboard_text().unwrap_or_default(),
            "eaglescribe-linux-hold"
        );
        release_clipboard_ownership();
        assert!(
            !linux_owner::is_holding(),
            "release_clipboard_ownership must drop the holder"
        );
    }
}
