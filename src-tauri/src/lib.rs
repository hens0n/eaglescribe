pub mod audio;
mod dictionary;
mod error;
mod history;
mod hotkey;
mod inject;
mod llm;
mod permissions_help;
mod polish;
pub mod recognition;
mod settings;
mod snippets;
mod state;
pub mod stt;

use error::{AppError, AppResult};
use hotkey::{
    detect_linux_session, hotkey_registration_failure_log, parse_shortcut, HotkeyRegisterReport,
    DEFAULT_COMMAND_HOTKEY, DEFAULT_DICTATION_HOTKEY, ESCAPE_CANCEL_HOTKEY,
    HOTKEYS_UNAVAILABLE_USER_MSG,
};
use polish::PolishMode;
use settings::HotkeyMode;
use state::{AppState, SharedState, StatusSnapshot};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use stt::resolve_model_path;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, RunEvent, WindowEvent,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

#[tauri::command]
fn get_status(state: tauri::State<'_, SharedState>) -> StatusSnapshot {
    state.inner().snapshot()
}

#[tauri::command]
fn set_model_path(path: String, state: tauri::State<'_, SharedState>) -> AppResult<StatusSnapshot> {
    let path = path.trim().to_string();
    if path.is_empty() {
        return Err(AppError::from("Model path is empty"));
    }
    state.inner().set_model_path(std::path::PathBuf::from(path));
    Ok(state.inner().snapshot())
}

#[tauri::command]
fn set_polish_mode(
    mode: String,
    state: tauri::State<'_, SharedState>,
) -> AppResult<StatusSnapshot> {
    let mode = match mode.to_lowercase().as_str() {
        "smart" => PolishMode::Smart,
        "verbatim" => PolishMode::Verbatim,
        other => {
            return Err(AppError::from(format!(
                "Unknown polish mode '{other}' (use smart or verbatim)"
            )))
        }
    };
    state.inner().set_polish_mode(mode);
    Ok(state.inner().snapshot())
}

#[tauri::command]
fn set_hotkey_mode(
    mode: String,
    state: tauri::State<'_, SharedState>,
) -> AppResult<StatusSnapshot> {
    let mode = HotkeyMode::parse(&mode)?;
    state.inner().set_hotkey_mode(mode)?;
    Ok(state.inner().snapshot())
}

/// Rebind global dictation and/or Command Mode hotkeys (persisted + re-registered).
///
/// Never panics when the OS cannot grab shortcuts (Linux Wayland / no X11). On
/// registration failure, settings roll back when the previous bindings can still
/// register; if nothing can register, bindings stay updated and
/// `global_hotkeys_ok` is false so the UI stays honest.
#[tauri::command]
fn set_hotkeys(
    dictation: String,
    command: String,
    app: AppHandle,
    state: tauri::State<'_, SharedState>,
) -> AppResult<StatusSnapshot> {
    let prev_d = state.inner().dictation_hotkey();
    let prev_c = state.inner().command_hotkey();

    state.inner().set_hotkey_bindings(&dictation, &command)?;
    let report = register_app_hotkeys(&app, state.inner());

    if report.all_ok() {
        apply_hotkey_register_report(state.inner(), &report);
        return Ok(state.inner().snapshot());
    }

    // Attempt to restore previous OS bindings when the new chord failed.
    let _ = state.inner().set_hotkey_bindings(&prev_d, &prev_c);
    let rollback = register_app_hotkeys(&app, state.inner());

    if rollback.all_ok() {
        apply_hotkey_register_report(state.inner(), &rollback);
        // Classic conflict / invalid grab — previous shortcuts work again.
        return Err(AppError::from(format!(
            "Could not register hotkeys (maybe in use by another app?): {}",
            report.summary()
        )));
    }

    // Platform cannot register (or both old and new fail). Keep the *new*
    // validated bindings so a later X11 session / restart can use them; never crash.
    let _ = state.inner().set_hotkey_bindings(&dictation, &command);
    // Persist the *last attempt* outcome (may be partial — UI must not claim full unavailable
    // when only one chord failed). Prefer new-chord report over empty rollback.
    apply_hotkey_register_report(state.inner(), &report);
    // Soft success: rebind persisted, UI path remains; honesty via snapshot flag + log.
    Ok(state.inner().snapshot())
}

#[tauri::command]
fn reset_hotkeys(
    app: AppHandle,
    state: tauri::State<'_, SharedState>,
) -> AppResult<StatusSnapshot> {
    set_hotkeys(
        DEFAULT_DICTATION_HOTKEY.to_string(),
        DEFAULT_COMMAND_HOTKEY.to_string(),
        app,
        state,
    )
}

#[tauri::command]
fn set_llm_settings(
    base_url: String,
    model: String,
    api_key: String,
    state: tauri::State<'_, SharedState>,
) -> AppResult<StatusSnapshot> {
    state
        .inner()
        .set_llm_settings(&base_url, &model, &api_key)?;
    Ok(state.inner().snapshot())
}

#[tauri::command]
fn set_history_enabled(
    enabled: bool,
    state: tauri::State<'_, SharedState>,
) -> AppResult<StatusSnapshot> {
    state.inner().set_history_enabled(enabled)?;
    Ok(state.inner().snapshot())
}

#[tauri::command]
fn set_clipboard_restore(
    enabled: bool,
    state: tauri::State<'_, SharedState>,
) -> AppResult<StatusSnapshot> {
    state.inner().set_clipboard_restore(enabled)?;
    Ok(state.inner().snapshot())
}

/// Persist silence trim (leading/trailing before Whisper). Applies next completed take.
#[tauri::command]
fn set_silence_trim(
    enabled: bool,
    state: tauri::State<'_, SharedState>,
) -> AppResult<StatusSnapshot> {
    state.inner().set_silence_trim(enabled)?;
    Ok(state.inner().snapshot())
}

/// Persist menu-bar-only (hide Dock). macOS applies on next launch; no-op elsewhere.
#[tauri::command]
fn set_menu_bar_only(
    enabled: bool,
    state: tauri::State<'_, SharedState>,
) -> AppResult<StatusSnapshot> {
    state.inner().set_menu_bar_only(enabled)?;
    Ok(state.inner().snapshot())
}

/// Persist first-run setup checklist dismiss (Settings can re-open content anytime).
#[tauri::command]
fn set_onboarding_dismissed(
    dismissed: bool,
    state: tauri::State<'_, SharedState>,
) -> AppResult<StatusSnapshot> {
    state.inner().set_onboarding_dismissed(dismissed)?;
    Ok(state.inner().snapshot())
}

/// macOS Privacy & Security deep-link URLs (Ventura+ extension pane first).
const MACOS_MIC_PRIVACY_URL: &str =
    "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_Microphone";
const MACOS_AX_PRIVACY_URL: &str =
    "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_Accessibility";
const MACOS_MIC_PRIVACY_URL_LEGACY: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone";
const MACOS_AX_PRIVACY_URL_LEGACY: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";

/// Resolve a privacy-pane token to modern + legacy System Settings URLs.
///
/// Tokens: `microphone` | `mic`, `accessibility` | `ax`. Pure helper for tests + open path.
fn macos_privacy_pane_urls(pane: &str) -> AppResult<(&'static str, &'static str)> {
    match pane.trim().to_ascii_lowercase().as_str() {
        "microphone" | "mic" => Ok((MACOS_MIC_PRIVACY_URL, MACOS_MIC_PRIVACY_URL_LEGACY)),
        "accessibility" | "ax" => Ok((MACOS_AX_PRIVACY_URL, MACOS_AX_PRIVACY_URL_LEGACY)),
        other => Err(AppError::from(format!(
            "Unknown privacy pane '{other}' (use microphone or accessibility)"
        ))),
    }
}

/// Open macOS System Settings → Privacy pane (Microphone or Accessibility).
///
/// Uses `/usr/bin/open` with validated pane tokens only — does not depend on the
/// JS opener URL allowlist. Non-macOS returns a clear error (UI hides these buttons).
#[tauri::command]
fn open_macos_privacy_pane(pane: String) -> AppResult<()> {
    let (primary, legacy) = macos_privacy_pane_urls(&pane)?;

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (primary, legacy);
        return Err(AppError::from(
            "System Settings deep links are only available on macOS. Use the manual path in the checklist.",
        ));
    }

    #[cfg(target_os = "macos")]
    {
        // Prefer modern PrivacySecurity extension URL; fall back to legacy preference pane.
        let primary_ok = std::process::Command::new("open")
            .arg(primary)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if primary_ok {
            return Ok(());
        }
        let legacy_ok = std::process::Command::new("open")
            .arg(legacy)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if legacy_ok {
            return Ok(());
        }
        Err(AppError::from(format!(
            "Could not open System Settings for '{pane}'. Use System Settings → Privacy & Security → {}",
            match pane.trim().to_ascii_lowercase().as_str() {
                "microphone" | "mic" => "Microphone",
                _ => "Accessibility",
            }
        )))
    }
}

/// Enumerate host input devices for the Settings mic picker and Refresh control.
///
/// No device list is cached: each invoke re-queries cpal so newly plugged mics
/// appear without restarting the app (spec §3.4 / acceptance criterion 7).
#[tauri::command]
fn list_mic_devices() -> AppResult<Vec<audio::InputDeviceInfo>> {
    audio::list_input_devices()
}

/// Persist preferred microphone (`null`/empty = system default). Applies on next recording.
#[tauri::command]
fn set_input_device(
    name: Option<String>,
    state: tauri::State<'_, SharedState>,
) -> AppResult<StatusSnapshot> {
    state.inner().set_input_device(name.as_deref())?;
    Ok(state.inner().snapshot())
}

#[tauri::command]
fn clear_history(state: tauri::State<'_, SharedState>) -> AppResult<StatusSnapshot> {
    state.inner().clear_history()?;
    Ok(state.inner().snapshot())
}

#[tauri::command]
fn dictionary_add(
    from: String,
    to: String,
    state: tauri::State<'_, SharedState>,
) -> AppResult<StatusSnapshot> {
    state.inner().dictionary_add(&from, &to)?;
    Ok(state.inner().snapshot())
}

#[tauri::command]
fn dictionary_remove(
    from: String,
    state: tauri::State<'_, SharedState>,
) -> AppResult<StatusSnapshot> {
    state.inner().dictionary_remove(&from)?;
    Ok(state.inner().snapshot())
}

#[tauri::command]
fn dictionary_edit(
    identity: dictionary::DictionaryEntryIdentity,
    from: String,
    to: String,
    state: tauri::State<'_, SharedState>,
) -> AppResult<StatusSnapshot> {
    state
        .inner()
        .dictionary_edit(&identity, &from, &to)?;
    Ok(state.inner().snapshot())
}

#[tauri::command]
fn dictionary_remove_entry(
    identity: dictionary::DictionaryEntryIdentity,
    state: tauri::State<'_, SharedState>,
) -> AppResult<StatusSnapshot> {
    state.inner().dictionary_remove_entry(&identity)?;
    Ok(state.inner().snapshot())
}

#[tauri::command]
fn dictionary_resolve_migration_conflict(
    resolution: dictionary::MigrationConflictResolution,
    state: tauri::State<'_, SharedState>,
) -> AppResult<StatusSnapshot> {
    state
        .inner()
        .dictionary_resolve_migration_conflict(&resolution)?;
    Ok(state.inner().snapshot())
}

#[tauri::command]
fn snippet_add(
    cue: String,
    expansion: String,
    state: tauri::State<'_, SharedState>,
) -> AppResult<StatusSnapshot> {
    state.inner().snippet_add(&cue, &expansion)?;
    Ok(state.inner().snapshot())
}

#[tauri::command]
fn snippet_remove(cue: String, state: tauri::State<'_, SharedState>) -> AppResult<StatusSnapshot> {
    state.inner().snippet_remove(&cue)?;
    Ok(state.inner().snapshot())
}

#[tauri::command]
fn load_model(state: tauri::State<'_, SharedState>) -> AppResult<StatusSnapshot> {
    state.inner().ensure_engine()?;
    Ok(state.inner().snapshot())
}

/// UI button: toggle listen (for when you cannot hold a key).
#[tauri::command]
fn toggle_dictation(
    app: AppHandle,
    state: tauri::State<'_, SharedState>,
) -> AppResult<StatusSnapshot> {
    toggle_dictation_inner(&app, state.inner())?;
    Ok(state.inner().snapshot())
}

/// UI button: run one Command Mode capture (toggle-style).
#[tauri::command]
fn toggle_command_mode(
    app: AppHandle,
    state: tauri::State<'_, SharedState>,
) -> AppResult<StatusSnapshot> {
    toggle_command_inner(&app, state.inner())?;
    Ok(state.inner().snapshot())
}

#[tauri::command]
fn cancel_dictation(
    app: AppHandle,
    state: tauri::State<'_, SharedState>,
) -> AppResult<StatusSnapshot> {
    state.inner().cancel_recording()?;
    // Defer: safe if this command is ever reached from a nested hotkey path.
    schedule_disarm_escape_cancel(&app);
    let snap = state.inner().snapshot();
    let _ = app.emit("dictation-status", &snap);
    Ok(snap)
}

fn busy_message(status: state::DictationStatus) -> AppError {
    match status {
        state::DictationStatus::WaitingLlm => AppError::from("Waiting on local LLM — please wait"),
        _ => AppError::from("Already busy — please wait"),
    }
}

fn start_dictation(app: &AppHandle, state: &SharedState) -> AppResult<()> {
    let status = state.snapshot().status;
    if status.is_busy() {
        return Err(busy_message(status));
    }
    match status {
        state::DictationStatus::Idle | state::DictationStatus::Error => {
            state.start_recording()?;
            // Must not register/unregister shortcuts inside a global-hotkey
            // callback — the plugin holds its mutex for the duration of the
            // handler (deadlocks on macOS). Queue for the next event-loop tick.
            schedule_arm_escape_cancel(app, state);
            let _ = app.emit("dictation-status", state.snapshot());
            Ok(())
        }
        state::DictationStatus::Recording => Ok(()),
        _ => Err(busy_message(status)),
    }
}

fn start_command(app: &AppHandle, state: &SharedState) -> AppResult<()> {
    let status = state.snapshot().status;
    if status.is_busy() {
        return Err(busy_message(status));
    }
    match status {
        state::DictationStatus::Idle | state::DictationStatus::Error => {
            state.start_command_recording(app)?;
            schedule_arm_escape_cancel(app, state);
            let _ = app.emit("dictation-status", state.snapshot());
            Ok(())
        }
        state::DictationStatus::Recording => Ok(()),
        _ => Err(busy_message(status)),
    }
}

fn stop_session(app: &AppHandle, state: &SharedState) -> AppResult<()> {
    let status = state.snapshot().status;
    if status.is_busy() {
        return Err(busy_message(status));
    }
    match status {
        state::DictationStatus::Recording => {
            // Leave recording → disarm Escape before STT/LLM (spec: armed only while recording).
            // Deferred: stop is often invoked from the dictation/command hotkey release handler.
            schedule_disarm_escape_cancel(app);
            let app_bg = app.clone();
            let state_bg = Arc::clone(state);
            std::thread::spawn(move || {
                let result = state_bg.stop_and_transcribe(&app_bg);
                if let Err(e) = &result {
                    state_bg.push_log(format!("Error: {e}"));
                }
                // Final status already emitted on success/error paths inside;
                // emit once more so UI always settles (e.g. after polish dictation).
                let _ = app_bg.emit("dictation-status", state_bg.snapshot());
            });
            Ok(())
        }
        state::DictationStatus::Idle | state::DictationStatus::Error => Ok(()),
        _ => Err(busy_message(status)),
    }
}

fn toggle_dictation_inner(app: &AppHandle, state: &SharedState) -> AppResult<()> {
    let status = state.snapshot().status;
    if status.is_busy() {
        return Err(busy_message(status));
    }
    match status {
        state::DictationStatus::Idle | state::DictationStatus::Error => start_dictation(app, state),
        state::DictationStatus::Recording => stop_session(app, state),
        _ => Err(busy_message(status)),
    }
}

fn toggle_command_inner(app: &AppHandle, state: &SharedState) -> AppResult<()> {
    let status = state.snapshot().status;
    if status.is_busy() {
        return Err(busy_message(status));
    }
    match status {
        state::DictationStatus::Idle | state::DictationStatus::Error => start_command(app, state),
        state::DictationStatus::Recording => stop_session(app, state),
        _ => Err(busy_message(status)),
    }
}

fn handle_dictation_hotkey(app: &AppHandle, state: &SharedState, key_state: ShortcutState) {
    // Self-heal: if the OS delivered this chord, the banner must not claim "unavailable".
    note_hotkey_observed(app, state, HotkeyRole::Dictation);

    let result = match state.hotkey_mode() {
        HotkeyMode::Hold => match key_state {
            ShortcutState::Pressed => start_dictation(app, state),
            ShortcutState::Released => {
                // Hold-safe cancel: discard leftover release after Escape / UI Cancel.
                if state.consume_hotkey_release_suppress() {
                    Ok(())
                } else {
                    stop_session(app, state)
                }
            }
        },
        HotkeyMode::Toggle => match key_state {
            ShortcutState::Pressed => toggle_dictation_inner(app, state),
            ShortcutState::Released => {
                // Toggle ignores release for start/stop, but still clear post-cancel suppress
                // so a later hold-mode switch or shared flag does not stick.
                let _ = state.consume_hotkey_release_suppress();
                Ok(())
            }
        },
    };
    if let Err(e) = result {
        state.push_log(format!("Hotkey error: {e}"));
        let _ = app.emit("dictation-status", state.snapshot());
    }
}

/// Command Mode always uses hold semantics on its own hotkey for predictability.
fn handle_command_hotkey(app: &AppHandle, state: &SharedState, key_state: ShortcutState) {
    note_hotkey_observed(app, state, HotkeyRole::Command);

    let result = match key_state {
        ShortcutState::Pressed => start_command(app, state),
        ShortcutState::Released => {
            if state.should_ignore_command_release() {
                // Synthetic Cmd/Ctrl+C during selection capture, or leftover release
                // after Escape cancel mid-hold — ignore those.
                Ok(())
            } else {
                stop_session(app, state)
            }
        }
    };
    if let Err(e) = result {
        state.push_log(format!("Command hotkey error: {e}"));
        let _ = app.emit("dictation-status", state.snapshot());
    }
}

/// Which global chord was observed (for registration self-heal).
enum HotkeyRole {
    Dictation,
    Command,
}

/// If the OS fired a global shortcut, mark that role as registered and refresh the UI.
///
/// Fixes a false "hotkeys unavailable" banner when registration bookkeeping is
/// wrong/stale but the chord still works (the user-visible truth).
fn note_hotkey_observed(app: &AppHandle, state: &SharedState, role: HotkeyRole) {
    let snap = state.snapshot();
    let (need_dict, need_cmd) = match role {
        HotkeyRole::Dictation => (!snap.dictation_hotkey_ok, false),
        HotkeyRole::Command => (false, !snap.command_hotkey_ok),
    };
    if !need_dict && !need_cmd {
        return;
    }
    let dict_ok = snap.dictation_hotkey_ok || need_dict;
    let cmd_ok = snap.command_hotkey_ok || need_cmd;
    state.set_hotkey_registration(dict_ok, cmd_ok);
    let label = match role {
        HotkeyRole::Dictation => "dictation",
        HotkeyRole::Command => "command",
    };
    state.push_log(format!(
        "Global {label} hotkey observed live — cleared stale unavailable state \
         (dictation_ok={dict_ok}, command_ok={cmd_ok})"
    ));
    let _ = app.emit("dictation-status", state.snapshot());
}

fn escape_shortcut() -> AppResult<Shortcut> {
    parse_shortcut(ESCAPE_CANCEL_HOTKEY)
}

/// Queue Escape arm after the current global-hotkey callback returns.
///
/// **Must** be used from global-shortcut handlers. Calling
/// [`arm_escape_cancel`] synchronously inside a hotkey callback deadlocks:
/// `tauri-plugin-global-shortcut` holds its internal mutex for the whole
/// handler, and register/unregister tries to take that same mutex.
///
/// Important: Tauri's `run_on_main_thread` runs the closure **inline** when
/// already on the main thread (hotkey handlers are). So we hop to a worker
/// thread first, then post back — that path uses the event-loop proxy and
/// runs only after the handler has released the plugin mutex.
///
/// Only arms if still `recording` when the deferred work runs (quick
/// press/release must not leave Escape grabbed after stop).
fn schedule_arm_escape_cancel(app: &AppHandle, state: &SharedState) {
    let app_c = app.clone();
    let state_c = Arc::clone(state);
    std::thread::spawn(move || {
        let app_m = app_c.clone();
        let _ = app_c.run_on_main_thread(move || {
            if state_c.is_recording() {
                arm_escape_cancel(&app_m, &state_c);
            }
        });
    });
}

/// Queue Escape disarm after the current global-hotkey callback returns.
///
/// Same re-entrancy rule as [`schedule_arm_escape_cancel`]: never call
/// plugin unregister from inside a hotkey (or Escape) callback. Worker hop
/// required because hotkey handlers already run on the main thread.
fn schedule_disarm_escape_cancel(app: &AppHandle) {
    let app_c = app.clone();
    std::thread::spawn(move || {
        let app_m = app_c.clone();
        let _ = app_c.run_on_main_thread(move || {
            disarm_escape_cancel(&app_m);
        });
    });
}

/// Register global Escape only while `recording`. Failures are logged, not fatal.
///
/// Call only when **not** already inside a global-shortcut handler (startup /
/// rebind / deferred schedule). From hotkey paths use [`schedule_arm_escape_cancel`].
fn arm_escape_cancel(app: &AppHandle, state: &SharedState) {
    let sc = match escape_shortcut() {
        Ok(s) => s,
        Err(e) => {
            state.push_log(format!("Escape cancel: cannot parse key: {e}"));
            return;
        }
    };

    // Idempotent: drop any prior Escape registration before re-arming.
    let _ = app.global_shortcut().unregister(sc);

    let handle = app.clone();
    let state_h = Arc::clone(state);
    if let Err(e) = app
        .global_shortcut()
        .on_shortcut(sc, move |_app, _sc, event| {
            if event.state != ShortcutState::Pressed {
                return;
            }
            match state_h.cancel_recording() {
                Ok(()) => {
                    // Defer unregister — we are inside the Escape hotkey callback.
                    schedule_disarm_escape_cancel(&handle);
                    let _ = handle.emit("dictation-status", state_h.snapshot());
                }
                Err(_) => {
                    // Race: left recording between arm and key; ensure disarmed.
                    schedule_disarm_escape_cancel(&handle);
                }
            }
        })
    {
        state.push_log(format!(
            "Escape cancel: could not register global shortcut ({e}). Use UI Cancel instead."
        ));
    }
}

/// Unregister global Escape (idempotent). Must run on every exit from `recording`.
///
/// Call only when **not** already inside a global-shortcut handler. From hotkey
/// paths use [`schedule_disarm_escape_cancel`].
fn disarm_escape_cancel(app: &AppHandle) {
    if let Ok(sc) = escape_shortcut() {
        let _ = app.global_shortcut().unregister(sc);
    }
}

/// Stable tray menu item ids (must match `handle_tray_menu_id`).
const TRAY_SHOW: &str = "tray-show";
const TRAY_HIDE: &str = "tray-hide";
const TRAY_QUIT: &str = "tray-quit";

/// Dedicated monochrome tray glyph (black silhouette on transparent).
/// Used as a macOS template image; same asset on Linux. Not the full-color app icon.
const TRAY_TEMPLATE_PNG: &[u8] = include_bytes!("../icons/tray-template.png");

fn tray_template_icon() -> tauri::Result<tauri::image::Image<'static>> {
    Ok(tauri::image::Image::from_bytes(TRAY_TEMPLATE_PNG)?.to_owned())
}

fn main_window(app: &AppHandle) -> Option<tauri::WebviewWindow> {
    app.get_webview_window("main")
        .or_else(|| app.webview_windows().into_values().next())
}

/// Generation for deferred restore passes. Bumped on every show and hide so a
/// late re-focus cannot re-show the window after the user intentionally hides.
static WINDOW_RESTORE_GEN: AtomicU64 = AtomicU64::new(0);

/// Bump and return the new restore generation (for a fresh Show / click / reopen).
fn next_window_restore_gen() -> u64 {
    WINDOW_RESTORE_GEN
        .fetch_add(1, Ordering::SeqCst)
        .wrapping_add(1)
}

/// True when `gen` is still the latest show/hide generation.
fn is_current_window_restore_gen(gen: u64) -> bool {
    WINDOW_RESTORE_GEN.load(Ordering::SeqCst) == gen
}

/// Invalidate pending deferred restores (Hide Window, close-to-tray).
fn invalidate_window_restore_gen() {
    let _ = next_window_restore_gen();
}

/// Core restore steps: unhide app (macOS), unminimize, show, focus (order front).
///
/// On macOS, `set_focus` maps to `makeKeyAndOrderFront` + `activateIgnoringOtherApps`.
fn apply_show_main_window(app: &AppHandle) {
    // Unhide the NSApplication so a close-to-tray / hidden window can reappear
    // (also required for Dock reopen when no windows are visible).
    #[cfg(target_os = "macos")]
    {
        let _ = app.show();
    }
    if let Some(window) = main_window(app) {
        let _ = window.unminimize();
        let _ = window.show();
        // Focus + order front (macOS: makeKeyAndOrderFront via Tauri/tao).
        let _ = window.set_focus();
    }
}

/// Deferred re-assert: only runs if this restore generation is still current
/// (no newer Show and no intentional Hide/close-to-tray in between).
fn apply_show_main_window_if_current(app: &AppHandle, gen: u64) {
    if !is_current_window_restore_gen(gen) {
        return;
    }
    apply_show_main_window(app);
}

/// Show and focus the main window after hide or close-to-tray.
///
/// Shared by tray **Show Window**, left-click, and Dock reopen so restore is
/// not path-dependent. Tray menus often steal focus as they dismiss, so we
/// re-apply show/focus on the next main-thread tick and once more after a
/// short delay on macOS — making Show repeatable rather than flaky.
///
/// Deferred passes carry a restore generation so Show→quick-Hide cannot be
/// undone by a stale 80 ms timer (or a next-tick pass after Hide).
fn show_main_window(app: &AppHandle) {
    let gen = next_window_restore_gen();
    apply_show_main_window(app);

    // Next event-loop turn: re-assert after tray menu / click handling settles.
    let app_tick = app.clone();
    let _ = app.run_on_main_thread(move || {
        apply_show_main_window_if_current(&app_tick, gen);
    });

    // macOS: menu dismissal can reclaim focus after the tick above. One delayed
    // pass keeps Show Window / left-click reliable across hide→show cycles.
    #[cfg(target_os = "macos")]
    {
        let app_delay = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(80));
            let app_main = app_delay.clone();
            let _ = app_delay.run_on_main_thread(move || {
                apply_show_main_window_if_current(&app_main, gen);
            });
        });
    }
}

fn hide_main_window(app: &AppHandle) {
    // Drop any pending deferred restore so it cannot re-show after hide.
    invalidate_window_restore_gen();
    if let Some(window) = main_window(app) {
        let _ = window.hide();
    }
}

fn handle_tray_menu_id(app: &AppHandle, id: &str) {
    match id {
        TRAY_SHOW => show_main_window(app),
        TRAY_HIDE => hide_main_window(app),
        TRAY_QUIT => {
            // Explicitly drop global shortcuts (incl. Escape) before exit.
            let _ = app.global_shortcut().unregister_all();
            // Linux: drop process-held clipboard so arboard can hand off / join cleanly.
            inject::release_clipboard_ownership();
            app.exit(0);
        }
        _ => {}
    }
}

/// Unregister all global shortcuts, then register dictation + command from settings.
///
/// Never panics. Individual grab failures are collected in the report so partial
/// success is visible and quit/`unregister_all` remain safe.
/// Re-arms Escape cancel if a recording is in progress (rebind while recording).
fn register_app_hotkeys(app: &AppHandle, state: &SharedState) -> HotkeyRegisterReport {
    let dictation_str = state.dictation_hotkey();
    let command_str = state.command_hotkey();

    let mut report = HotkeyRegisterReport {
        dictation_ok: false,
        command_ok: false,
        errors: Vec::new(),
    };

    let dictation = match parse_shortcut(&dictation_str) {
        Ok(s) => s,
        Err(e) => {
            report
                .errors
                .push(format!("parse dictation '{dictation_str}': {e}"));
            // Still try command so partial/config errors are independent.
            let _ = try_register_command_hotkey(app, state, &command_str, &mut report);
            // Escape only when recording; ignore failures.
            if state.is_recording() {
                arm_escape_cancel(app, state);
            }
            return report;
        }
    };

    let command = match parse_shortcut(&command_str) {
        Ok(s) => s,
        Err(e) => {
            report
                .errors
                .push(format!("parse command '{command_str}': {e}"));
            // Clear previous; register dictation alone if possible.
            let _ = app.global_shortcut().unregister_all();
            try_register_dictation_hotkey(app, state, dictation, &mut report);
            if state.is_recording() {
                arm_escape_cancel(app, state);
            }
            return report;
        }
    };

    // Clear previous bindings so rebind is clean (includes Escape if it was armed).
    // Ignore errors: nothing registered yet / plugin unavailable must not crash.
    let _ = app.global_shortcut().unregister_all();

    try_register_dictation_hotkey(app, state, dictation, &mut report);
    try_register_command_hotkey_sc(app, state, command, &mut report);

    // Dynamic Escape is not part of persistent settings; restore if still recording.
    if state.is_recording() {
        arm_escape_cancel(app, state);
    }

    report
}

fn try_register_dictation_hotkey(
    app: &AppHandle,
    state: &SharedState,
    dictation: Shortcut,
    report: &mut HotkeyRegisterReport,
) {
    let handle = app.clone();
    let state_h = Arc::clone(state);
    match app
        .global_shortcut()
        .on_shortcut(dictation, move |_app, _sc, event| {
            handle_dictation_hotkey(&handle, &state_h, event.state);
        }) {
        // Trust Ok from the plugin. Do not second-guess with is_registered —
        // a false negative there left the UI saying "unavailable" while the
        // chord still fired.
        Ok(()) => report.dictation_ok = true,
        Err(e) => {
            report.dictation_ok = false;
            report
                .errors
                .push(format!("Register dictation hotkey failed: {e}"));
        }
    }
}

fn try_register_command_hotkey(
    app: &AppHandle,
    state: &SharedState,
    command_str: &str,
    report: &mut HotkeyRegisterReport,
) {
    match parse_shortcut(command_str) {
        Ok(sc) => {
            let _ = app.global_shortcut().unregister_all();
            try_register_command_hotkey_sc(app, state, sc, report);
        }
        Err(e) => {
            report
                .errors
                .push(format!("parse command '{command_str}': {e}"));
        }
    }
}

fn try_register_command_hotkey_sc(
    app: &AppHandle,
    state: &SharedState,
    command: Shortcut,
    report: &mut HotkeyRegisterReport,
) {
    let handle = app.clone();
    let state_h = Arc::clone(state);
    match app
        .global_shortcut()
        .on_shortcut(command, move |_app, _sc, event| {
            handle_command_hotkey(&handle, &state_h, event.state);
        }) {
        Ok(()) => report.command_ok = true,
        Err(e) => {
            report.command_ok = false;
            report
                .errors
                .push(format!("Register command hotkey failed: {e}"));
        }
    }
}

/// Apply registration outcome to state: flag + logs. Never claims full active when partial.
fn apply_hotkey_register_report(state: &SharedState, report: &HotkeyRegisterReport) {
    // Per-role flags so the UI can hide the "all unavailable" chip when dictation
    // works even if Command Mode failed (and vice versa).
    state.set_hotkey_registration(report.dictation_ok, report.command_ok);

    // Always log explicit per-role outcome (drives debugging when UI disagrees).
    state.push_log(format!(
        "Hotkey registration: dictation={} ({}) · command={} ({})",
        if report.dictation_ok { "ok" } else { "FAIL" },
        state.dictation_hotkey(),
        if report.command_ok { "ok" } else { "FAIL" },
        state.command_hotkey(),
    ));

    if report.all_ok() {
        state.push_log(format!(
            "Global hotkeys active: dictation={} · command={}",
            state.dictation_hotkey(),
            state.command_hotkey()
        ));
        return;
    }

    let session = detect_linux_session();
    let detail = report.summary();
    state.push_log(hotkey_registration_failure_log(&detail, session));

    if report.any_ok() {
        // Partial success — do not claim everything is unavailable.
        let which = match (report.dictation_ok, report.command_ok) {
            (true, false) => format!(
                "Dictation hotkey active ({}); Command Mode hotkey failed — use the window Command button.",
                state.dictation_hotkey()
            ),
            (false, true) => format!(
                "Command Mode hotkey active ({}); dictation hotkey failed — use Start dictation in the window.",
                state.command_hotkey()
            ),
            _ => "Partial global hotkey registration.".into(),
        };
        state.push_log(which);
    } else {
        state.push_log(HOTKEYS_UNAVAILABLE_USER_MSG.to_string());
    }
}

/// Whether launch should use macOS Accessory policy (no Dock icon).
///
/// Separated so unit tests drive the decision from settings without GUI.
#[cfg(target_os = "macos")]
fn should_use_accessory_policy(menu_bar_only: bool) -> bool {
    menu_bar_only
}

/// Apply persisted menu-bar-only as activation policy (next-launch semantics).
///
/// Only changes Dock presence when `menu_bar_only` is true. Default Regular
/// (Dock present) is left untouched when off.
#[cfg(target_os = "macos")]
fn apply_menu_bar_only_activation_policy(app: &mut tauri::App, menu_bar_only: bool) {
    if should_use_accessory_policy(menu_bar_only) {
        app.set_activation_policy(tauri::ActivationPolicy::Accessory);
    }
}

/// Menu bar / system tray: stay available while the main window is hidden.
fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let show_i = MenuItem::with_id(app, TRAY_SHOW, "Show Window", true, None::<&str>)?;
    let hide_i = MenuItem::with_id(app, TRAY_HIDE, "Hide Window", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit_i = MenuItem::with_id(app, TRAY_QUIT, "Quit EagleScribe", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_i, &hide_i, &sep, &quit_i])?;

    // App-level listener: tray menu events are delivered as global MenuEvents.
    // Relying only on TrayIconBuilder::on_menu_event has been flaky on macOS.
    app.on_menu_event(|app, event| {
        handle_tray_menu_id(app, event.id().as_ref());
    });

    let mut builder = TrayIconBuilder::with_id("eaglescribe-tray")
        .menu(&menu)
        // Left-click: show window. Right-click: open menu (macOS/Windows).
        // Linux: tray click events unsupported — use the menu.
        .show_menu_on_left_click(false)
        .tooltip("EagleScribe — local dictation")
        .on_menu_event(|app, event| {
            handle_tray_menu_id(app, event.id().as_ref());
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });

    // Dedicated monochrome glyph — not the full-color app icon (template would go invisible).
    let icon = tray_template_icon()?;
    builder = builder.icon(icon);
    // macOS tints template images for light/dark menu bars. No-op / harmless elsewhere.
    builder = builder.icon_as_template(true);
    // No tray title string — glyph is the sole menu-bar mark (was "ES" fallback).

    let _tray = builder.build(app)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let model_path = resolve_model_path(None);
    let app_state: SharedState = Arc::new(AppState::new(model_path));

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            get_status,
            set_model_path,
            set_polish_mode,
            set_hotkey_mode,
            set_hotkeys,
            reset_hotkeys,
            set_llm_settings,
            set_history_enabled,
            set_clipboard_restore,
            set_silence_trim,
            set_menu_bar_only,
            set_onboarding_dismissed,
            open_macos_privacy_pane,
            list_mic_devices,
            set_input_device,
            clear_history,
            dictionary_add,
            dictionary_remove,
            dictionary_edit,
            dictionary_remove_entry,
            dictionary_resolve_migration_conflict,
            snippet_add,
            snippet_remove,
            load_model,
            toggle_dictation,
            toggle_command_mode,
            cancel_dictation,
        ])
        // Closing the window hides to tray so hotkeys keep working.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                // Invalidate deferred Show re-focus (same race as Hide Window).
                invalidate_window_restore_gen();
                let _ = window.hide();
            }
        })
        .setup(|app| {
            let state = Arc::clone(app.state::<SharedState>().inner());

            // Menu-bar-only (hide Dock) applies only at launch from settings.json.
            #[cfg(target_os = "macos")]
            {
                apply_menu_bar_only_activation_policy(app, state.menu_bar_only());
            }

            setup_tray(app)?;

            // Global hotkey registration must never abort startup (Linux Wayland / no X11).
            let hotkey_report = register_app_hotkeys(app.handle(), &state);
            apply_hotkey_register_report(&state, &hotkey_report);
            // Push status so the webview never sticks on the pre-registration default
            // (global_hotkeys_ok=false) if it loaded a snapshot early.
            let _ = app.emit("dictation-status", state.snapshot());

            let mode = state.hotkey_mode();
            // Always log configured bindings; "active" is only claimed via apply_hotkey_register_report.
            state.push_log(format!(
                "Dictation (configured): {} ({})",
                state.dictation_hotkey(),
                mode.as_str()
            ));
            state.push_log(format!(
                "Command Mode (configured): {} (hold)",
                state.command_hotkey()
            ));
            state.push_log(format!("Hotkey mode: {}", mode.label()));
            if cfg!(target_os = "linux") {
                state.push_log(format!(
                    "Linux session: {} ($XDG_SESSION_TYPE)",
                    detect_linux_session().as_str()
                ));
            }
            state.push_log(format!("Model path: {}", state.snapshot().model_path));
            state.push_log(format!(
                "LLM: {} / {}",
                state.snapshot().llm_base_url,
                state.snapshot().llm_model
            ));
            if state.menu_bar_only() {
                state.push_log(
                    "Tray: close hides · click menu-bar icon / Show Window · menu bar only (no Dock) · Quit from tray",
                );
            } else {
                state.push_log(
                    "Tray: close hides · click menu-bar icon / Show Window · Dock click also restores · Quit from tray",
                );
            }

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building EagleScribe")
        .run(|app, event| match event {
            // Backup path for tray/window menu activation.
            RunEvent::MenuEvent(e) => {
                handle_tray_menu_id(app, e.id().as_ref());
            }
            // Dock icon click while all windows are hidden.
            #[cfg(target_os = "macos")]
            RunEvent::Reopen {
                has_visible_windows: false,
                ..
            } => {
                show_main_window(app);
            }
            _ => {}
        });
}

#[cfg(test)]
mod macos_privacy_pane_tests {
    use super::{
        macos_privacy_pane_urls, MACOS_AX_PRIVACY_URL, MACOS_AX_PRIVACY_URL_LEGACY,
        MACOS_MIC_PRIVACY_URL, MACOS_MIC_PRIVACY_URL_LEGACY,
    };

    #[test]
    fn maps_microphone_and_accessibility_tokens() {
        let (p, l) = macos_privacy_pane_urls("microphone").expect("mic");
        assert_eq!(p, MACOS_MIC_PRIVACY_URL);
        assert_eq!(l, MACOS_MIC_PRIVACY_URL_LEGACY);
        assert!(p.contains("Privacy_Microphone"));
        assert!(p.starts_with("x-apple.systempreferences:"));

        let (p, l) = macos_privacy_pane_urls("Accessibility").expect("ax");
        assert_eq!(p, MACOS_AX_PRIVACY_URL);
        assert_eq!(l, MACOS_AX_PRIVACY_URL_LEGACY);
        assert!(p.contains("Privacy_Accessibility"));

        // Short aliases used by the frontend.
        assert!(macos_privacy_pane_urls("mic").is_ok());
        assert!(macos_privacy_pane_urls("ax").is_ok());
    }

    #[test]
    fn rejects_unknown_and_dangerous_panes() {
        assert!(macos_privacy_pane_urls("screen_recording").is_err());
        assert!(macos_privacy_pane_urls("full_disk").is_err());
        assert!(macos_privacy_pane_urls("").is_err());
        assert!(macos_privacy_pane_urls("http://evil.example").is_err());
    }

    #[test]
    fn urls_are_systempreferences_scheme_only() {
        for pane in ["microphone", "accessibility"] {
            let (p, l) = macos_privacy_pane_urls(pane).expect(pane);
            assert!(
                p.starts_with("x-apple.systempreferences:")
                    && l.starts_with("x-apple.systempreferences:"),
                "pane {pane} must use systempreferences scheme"
            );
            // Never open arbitrary https / file paths from this helper.
            assert!(!p.contains("://http") && !p.starts_with("file:"));
        }
    }
}

#[cfg(test)]
mod tray_template_tests {
    use super::{tray_template_icon, TRAY_TEMPLATE_PNG};
    use std::path::Path;

    #[test]
    fn tray_template_png_asset_exists_on_disk() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("icons/tray-template.png");
        assert!(
            path.is_file(),
            "dedicated tray template asset missing at {}",
            path.display()
        );
        let bytes = std::fs::read(&path).expect("read tray-template.png");
        assert!(
            bytes.starts_with(&[0x89, b'P', b'N', b'G']),
            "tray-template.png must be a PNG"
        );
        assert!(bytes.len() > 64, "tray template PNG looks empty");
    }

    #[test]
    fn tray_template_is_embedded_and_decodes() {
        assert!(
            TRAY_TEMPLATE_PNG.starts_with(&[0x89, b'P', b'N', b'G']),
            "embedded tray template must be PNG bytes"
        );
        let icon = tray_template_icon().expect("decode tray-template.png");
        assert!(icon.width() >= 16 && icon.height() >= 16);
        assert_eq!(icon.width(), icon.height(), "tray glyph should be square");
        // Non-empty alpha somewhere (silhouette present).
        let rgba = icon.rgba();
        let has_ink = rgba.chunks(4).any(|p| p[3] > 0);
        assert!(has_ink, "tray template has no visible pixels");
        // Template-style: ink is black (R=G=B=0) where opaque.
        let non_black_ink = rgba
            .chunks(4)
            .any(|p| p[3] > 200 && (p[0] | p[1] | p[2]) != 0);
        assert!(
            !non_black_ink,
            "tray template should be black silhouette for macOS template tinting"
        );
    }

    #[test]
    fn setup_tray_source_uses_template_not_color_icon_or_es_title() {
        let lib = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs"))
            .expect("read lib.rs");
        assert!(
            lib.contains("tray-template.png"),
            "setup_tray must reference icons/tray-template.png"
        );
        assert!(
            lib.contains("icon_as_template(true)"),
            "macOS tray must set icon_as_template(true) for the monochrome glyph"
        );
        assert!(
            lib.contains("TRAY_TEMPLATE_PNG") && lib.contains("tray_template_icon"),
            "tray setup should load the dedicated template asset helper"
        );
        assert!(
            !lib.contains(".title(\"ES\")"),
            "macOS tray title \"ES\" must be removed once the template glyph ships"
        );
        // Guard against regressing to full-color default icon as the tray mark.
        let setup = lib
            .split("fn setup_tray")
            .nth(1)
            .and_then(|s| s.split("fn run").next())
            .expect("setup_tray function body");
        assert!(
            !setup.contains("default_window_icon"),
            "setup_tray must not use default_window_icon as the tray glyph"
        );
    }
}

#[cfg(test)]
mod tray_restore_tests {
    use super::{
        invalidate_window_restore_gen, is_current_window_restore_gen, next_window_restore_gen,
        WINDOW_RESTORE_GEN,
    };
    use std::path::Path;
    use std::sync::atomic::Ordering;

    fn lib_src() -> String {
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs"))
            .expect("read lib.rs")
    }

    /// Extract a top-level `fn name` body (brace-matched) from `lib.rs`.
    fn fn_body<'a>(src: &'a str, name: &str) -> &'a str {
        let marker = format!("fn {name}");
        let start = src
            .find(&marker)
            .unwrap_or_else(|| panic!("missing {marker}"));
        let after = &src[start..];
        let brace = after
            .find('{')
            .unwrap_or_else(|| panic!("{marker} has no body"));
        let bytes = after.as_bytes();
        let mut depth = 0i32;
        let mut end = brace;
        for (i, &b) in bytes[brace..].iter().enumerate() {
            match b {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = brace + i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        &after[..end]
    }

    #[test]
    fn restore_generation_invalidates_stale_deferred_show() {
        // Isolate from other tests that may bump the counter.
        WINDOW_RESTORE_GEN.store(0, Ordering::SeqCst);

        let show_gen = next_window_restore_gen();
        assert!(
            is_current_window_restore_gen(show_gen),
            "fresh show gen must be current"
        );

        // Simulate Hide Window / close-to-tray after Show.
        invalidate_window_restore_gen();
        assert!(
            !is_current_window_restore_gen(show_gen),
            "hide must invalidate prior show gen so delayed re-show is skipped"
        );

        // A newer show is current; older remains stale.
        let show_gen2 = next_window_restore_gen();
        assert!(is_current_window_restore_gen(show_gen2));
        assert!(!is_current_window_restore_gen(show_gen));

        // Triple Show (menu triple-delivery) keeps only the latest gen current.
        let g1 = next_window_restore_gen();
        let g2 = next_window_restore_gen();
        let g3 = next_window_restore_gen();
        assert!(!is_current_window_restore_gen(g1));
        assert!(!is_current_window_restore_gen(g2));
        assert!(is_current_window_restore_gen(g3));
    }

    #[test]
    fn show_main_window_helper_exists_with_focus_stack() {
        let lib = lib_src();
        assert!(
            lib.contains("fn show_main_window"),
            "shared show_main_window helper must exist"
        );
        assert!(
            lib.contains("fn apply_show_main_window"),
            "core apply_show_main_window restore steps must exist"
        );

        let apply = fn_body(&lib, "apply_show_main_window");
        for step in ["unminimize", ".show()", "set_focus"] {
            assert!(
                apply.contains(step),
                "apply_show_main_window must use {step} (show + focus / order front)"
            );
        }
        // macOS app-level unhide before window show (Dock reopen + close-to-tray).
        assert!(
            apply.contains("app.show()") || apply.contains("app .show()"),
            "macOS path should call app.show() to unhide the application"
        );

        let show = fn_body(&lib, "show_main_window");
        assert!(
            show.contains("apply_show_main_window"),
            "show_main_window must call apply_show_main_window"
        );
        assert!(
            show.contains("run_on_main_thread"),
            "show_main_window must re-assert focus on the main thread (tray menu steal)"
        );
        // macOS delayed re-focus after menu dismissal (regression gate for flaky Show).
        assert!(
            show.contains("from_millis(80)") && show.contains("thread::spawn"),
            "show_main_window must schedule macOS delayed re-focus (~80ms)"
        );
        assert!(
            show.contains("apply_show_main_window_if_current")
                || show.contains("is_current_window_restore_gen"),
            "deferred passes must gate on restore generation"
        );
        assert!(
            show.contains("next_window_restore_gen"),
            "show must mint a restore generation for deferred work"
        );
    }

    #[test]
    fn show_main_window_is_used_by_menu_click_and_reopen() {
        let lib = lib_src();

        // Tray menu Show Window id → helper.
        let handle = fn_body(&lib, "handle_tray_menu_id");
        assert!(
            handle.contains("TRAY_SHOW") && handle.contains("show_main_window"),
            "handle_tray_menu_id must route TRAY_SHOW to show_main_window"
        );
        assert!(
            handle.contains("TRAY_HIDE") && handle.contains("hide_main_window"),
            "Hide Window semantics must remain"
        );
        assert!(
            handle.contains("TRAY_QUIT") && handle.contains("unregister_all"),
            "Quit must unregister global hotkeys before exit"
        );

        // Left-click tray icon → helper.
        let setup = fn_body(&lib, "setup_tray");
        assert!(
            setup.contains("MouseButton::Left") && setup.contains("show_main_window"),
            "setup_tray left-click must call show_main_window"
        );

        // Triple menu delivery (historical flaky-Show fix): app-level, tray, RunEvent.
        let app_menu_hits = setup.matches("on_menu_event").count();
        assert!(
            app_menu_hits >= 2,
            "setup_tray must wire app.on_menu_event and tray on_menu_event (got {app_menu_hits})"
        );
        assert!(
            setup.matches("handle_tray_menu_id").count() >= 2,
            "both setup_tray menu listeners must call handle_tray_menu_id"
        );
        // RunEvent::MenuEvent backup path lives in run(), not setup_tray.
        assert!(
            lib.contains("RunEvent::MenuEvent") && lib.contains("handle_tray_menu_id(app, e.id()"),
            "RunEvent::MenuEvent must also call handle_tray_menu_id"
        );

        // Hide / close-to-tray must invalidate deferred restore.
        let hide = fn_body(&lib, "hide_main_window");
        assert!(
            hide.contains("invalidate_window_restore_gen"),
            "hide_main_window must invalidate deferred restore generation"
        );
        assert!(
            lib.contains("invalidate_window_restore_gen()") && lib.contains("CloseRequested"),
            "close-to-tray must invalidate deferred restore generation"
        );

        // Dock reopen while hidden → helper.
        assert!(
            lib.contains("RunEvent::Reopen") && lib.contains("show_main_window(app)"),
            "Dock Reopen path must call show_main_window"
        );
        assert!(
            lib.contains("has_visible_windows: false"),
            "Reopen should only restore when all windows are hidden"
        );
    }

    #[test]
    fn tray_menu_ids_and_labels_unchanged() {
        let lib = lib_src();
        assert!(lib.contains("const TRAY_SHOW: &str = \"tray-show\""));
        assert!(lib.contains("const TRAY_HIDE: &str = \"tray-hide\""));
        assert!(lib.contains("const TRAY_QUIT: &str = \"tray-quit\""));

        // Only Show / Hide / Quit tray actions (no new items this slice).
        let setup = fn_body(&lib, "setup_tray");
        assert!(setup.contains("\"Show Window\""));
        assert!(setup.contains("\"Hide Window\""));
        assert!(setup.contains("\"Quit EagleScribe\""));
        let menu_item_labels: Vec<_> = setup
            .match_indices("MenuItem::with_id")
            .map(|(i, _)| &setup[i..])
            .collect();
        assert_eq!(
            menu_item_labels.len(),
            3,
            "tray menu must have exactly Show, Hide, Quit items"
        );
    }

    /// Regression: dictation hotkey → arm Escape must not call plugin
    /// register/unregister synchronously. That deadlocks macOS because the
    /// global-shortcut handler holds the plugin mutex for the whole callback
    /// (sample: arm_escape_cancel → GlobalShortcut::unregister → mutex wait).
    #[test]
    fn escape_arm_disarm_deferred_from_hotkey_paths() {
        let lib = lib_src();

        assert!(
            lib.contains("fn schedule_arm_escape_cancel")
                && lib.contains("fn schedule_disarm_escape_cancel"),
            "deferred Escape arm/disarm helpers must exist"
        );

        let start_d = fn_body(&lib, "start_dictation");
        assert!(
            start_d.contains("schedule_arm_escape_cancel"),
            "start_dictation must defer Escape arm (hotkey Pressed path)"
        );
        // Reject bare `arm_escape_cancel(` but allow `schedule_arm_escape_cancel(`.
        assert!(
            !start_d
                .replace("schedule_arm_escape_cancel", "SCHEDULED")
                .contains("arm_escape_cancel"),
            "start_dictation must not call arm_escape_cancel synchronously"
        );

        let start_c = fn_body(&lib, "start_command");
        assert!(
            start_c.contains("schedule_arm_escape_cancel"),
            "start_command must defer Escape arm"
        );
        assert!(
            !start_c
                .replace("schedule_arm_escape_cancel", "SCHEDULED")
                .contains("arm_escape_cancel"),
            "start_command must not call arm_escape_cancel synchronously"
        );

        let stop = fn_body(&lib, "stop_session");
        assert!(
            stop.contains("schedule_disarm_escape_cancel"),
            "stop_session must defer Escape disarm (hotkey Released path)"
        );
        assert!(
            !stop
                .replace("schedule_disarm_escape_cancel", "SCHEDULED")
                .contains("disarm_escape_cancel"),
            "stop_session must not call disarm_escape_cancel synchronously"
        );

        // Escape handler itself must defer disarm (re-entrancy on same plugin).
        let arm = fn_body(&lib, "arm_escape_cancel");
        assert!(
            arm.contains("schedule_disarm_escape_cancel"),
            "Escape callback must schedule_disarm, not sync unregister"
        );
        assert!(
            !arm
                .replace("schedule_disarm_escape_cancel", "SCHEDULED")
                .contains("disarm_escape_cancel"),
            "Escape callback must not call disarm_escape_cancel synchronously"
        );

        // Deferred arm: hop off main thread (run_on_main_thread is inline when
        // already on main), then post back; re-check recording so quick release
        // does not leave Escape grabbed.
        let sched_arm = fn_body(&lib, "schedule_arm_escape_cancel");
        assert!(
            sched_arm.contains("thread::spawn")
                && sched_arm.contains("run_on_main_thread")
                && sched_arm.contains("is_recording"),
            "schedule_arm must worker-hop, then main-thread arm only while still recording"
        );
        let sched_disarm = fn_body(&lib, "schedule_disarm_escape_cancel");
        assert!(
            sched_disarm.contains("thread::spawn") && sched_disarm.contains("run_on_main_thread"),
            "schedule_disarm must worker-hop then main-thread unregister"
        );
    }
}

#[cfg(test)]
mod menu_bar_only_tests {
    use super::*;
    use std::path::Path;

    fn lib_src() -> String {
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs"))
            .expect("read lib.rs")
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn should_use_accessory_policy_matches_preference() {
        assert!(!should_use_accessory_policy(false));
        assert!(should_use_accessory_policy(true));
    }

    #[test]
    fn setup_applies_accessory_policy_from_settings_on_macos() {
        let lib = lib_src();
        assert!(
            lib.contains("apply_menu_bar_only_activation_policy"),
            "setup must call apply_menu_bar_only_activation_policy on macOS"
        );
        assert!(
            lib.contains("ActivationPolicy::Accessory"),
            "menu-bar-only must set ActivationPolicy::Accessory"
        );
        assert!(
            lib.contains("state.menu_bar_only()"),
            "activation policy must read persisted menu_bar_only"
        );
        // Command + handler registration.
        assert!(
            lib.contains("fn set_menu_bar_only") && lib.contains("set_menu_bar_only,"),
            "set_menu_bar_only command must be registered"
        );
        // Must not flip policy live when the toggle changes — only at launch.
        let set_cmd = {
            let start = lib
                .find("fn set_menu_bar_only")
                .expect("set_menu_bar_only command");
            &lib[start..start + 400]
        };
        assert!(
            !set_cmd.contains("set_activation_policy") && !set_cmd.contains("ActivationPolicy"),
            "set_menu_bar_only must not change activation policy live (next launch only)"
        );
        // Restore paths from #15 must remain (Show / left-click still work without Dock).
        assert!(
            lib.contains("fn show_main_window")
                && lib.contains("TRAY_SHOW")
                && lib.contains("MouseButton::Left"),
            "menu-bar-only must not remove tray Show / left-click restore"
        );
    }

    #[test]
    fn apply_helper_sets_accessory_only_when_on() {
        let lib = lib_src();
        // Helper body: only Accessory when preference is on.
        let start = lib
            .find("fn apply_menu_bar_only_activation_policy")
            .expect("apply helper");
        let body = &lib[start..start + 350];
        assert!(
            body.contains("should_use_accessory_policy(menu_bar_only)")
                || body.contains("if menu_bar_only")
                || body.contains("if should_use_accessory_policy"),
            "apply helper must gate Accessory on menu_bar_only"
        );
        assert!(
            body.contains("ActivationPolicy::Accessory"),
            "apply helper must set Accessory"
        );
        // Off path: no forced Regular flip required (default Dock).
        assert!(
            !body.contains("ActivationPolicy::Regular"),
            "leave Regular as default when menu_bar_only is off"
        );
    }
}

#[cfg(test)]
mod hotkey_registration_honesty_tests {
    use super::hotkey::{
        classify_linux_session, hotkey_registration_failure_log, HotkeyRegisterReport,
        LinuxSession, HOTKEYS_UNAVAILABLE_USER_MSG,
    };
    use std::path::Path;

    fn lib_src() -> String {
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs"))
            .expect("read lib.rs")
    }

    #[test]
    fn setup_does_not_abort_on_hotkey_registration_failure() {
        let lib = lib_src();
        // Historical bug: setup used `.map_err(|e| e.to_string())?` and killed startup.
        assert!(
            !lib.contains("register_app_hotkeys(app.handle(), &state)\n                .map_err"),
            "setup must not fail the app when register_app_hotkeys errors"
        );
        assert!(
            lib.contains("apply_hotkey_register_report"),
            "setup must apply registration report (flag + log) instead of aborting"
        );
        assert!(
            lib.contains("HOTKEYS_UNAVAILABLE_USER_MSG"),
            "must surface window-controls guidance constant"
        );
    }

    #[test]
    fn register_app_hotkeys_returns_report_not_result_err() {
        let lib = lib_src();
        let start = lib
            .find("fn register_app_hotkeys")
            .expect("register_app_hotkeys");
        let sig = &lib[start..start + 120];
        assert!(
            sig.contains("-> HotkeyRegisterReport"),
            "register_app_hotkeys must return a soft report, not AppResult: {sig}"
        );
        assert!(
            !sig.contains("-> AppResult"),
            "register_app_hotkeys must not be hard-fail AppResult"
        );
    }

    #[test]
    fn quit_still_unregisters_safely() {
        let lib = lib_src();
        let handle = {
            let start = lib
                .find("fn handle_tray_menu_id")
                .expect("handle_tray_menu_id");
            &lib[start..start + 500]
        };
        assert!(
            handle.contains("unregister_all"),
            "Quit must still call unregister_all (safe when nothing registered)"
        );
    }

    #[test]
    fn failure_message_helpers_are_honest() {
        let msg = hotkey_registration_failure_log("no display", LinuxSession::Wayland);
        assert!(msg.contains("window controls") || msg.contains(HOTKEYS_UNAVAILABLE_USER_MSG));
        assert_eq!(
            classify_linux_session(Some("wayland")),
            LinuxSession::Wayland
        );

        let partial = HotkeyRegisterReport {
            dictation_ok: true,
            command_ok: false,
            errors: vec!["grab failed".into()],
        };
        assert!(!partial.all_ok());
        assert!(partial.any_ok());
    }
}
