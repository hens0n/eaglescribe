mod audio;
mod dictionary;
mod error;
mod hotkey;
mod inject;
mod llm;
mod polish;
mod settings;
mod snippets;
mod state;
mod stt;

use error::{AppError, AppResult};
use hotkey::{parse_shortcut, DEFAULT_COMMAND_HOTKEY, DEFAULT_DICTATION_HOTKEY};
use polish::PolishMode;
use settings::HotkeyMode;
use state::{AppState, SharedState, StatusSnapshot};
use stt::resolve_model_path;
use std::sync::Arc;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, RunEvent, WindowEvent,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

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
    if let Err(e) = register_app_hotkeys(&app, state.inner()) {
        // Roll back settings + OS registration.
        let _ = state.inner().set_hotkey_bindings(&prev_d, &prev_c);
        let _ = register_app_hotkeys(&app, state.inner());
        return Err(AppError::from(format!(
            "Could not register hotkeys (maybe in use by another app?): {e}"
        )));
    }
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
fn snippet_add(
    cue: String,
    expansion: String,
    state: tauri::State<'_, SharedState>,
) -> AppResult<StatusSnapshot> {
    state.inner().snippet_add(&cue, &expansion)?;
    Ok(state.inner().snapshot())
}

#[tauri::command]
fn snippet_remove(
    cue: String,
    state: tauri::State<'_, SharedState>,
) -> AppResult<StatusSnapshot> {
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
fn cancel_dictation(state: tauri::State<'_, SharedState>) -> AppResult<StatusSnapshot> {
    state.inner().cancel_recording()?;
    Ok(state.inner().snapshot())
}

fn start_dictation(app: &AppHandle, state: &SharedState) -> AppResult<()> {
    let status = state.snapshot().status;
    match status {
        state::DictationStatus::Idle | state::DictationStatus::Error => {
            state.start_recording()?;
            let _ = app.emit("dictation-status", state.snapshot());
            Ok(())
        }
        state::DictationStatus::Recording => Ok(()),
        state::DictationStatus::Transcribing => {
            Err(AppError::from("Already transcribing — please wait"))
        }
    }
}

fn start_command(app: &AppHandle, state: &SharedState) -> AppResult<()> {
    let status = state.snapshot().status;
    match status {
        state::DictationStatus::Idle | state::DictationStatus::Error => {
            state.start_command_recording(app)?;
            let _ = app.emit("dictation-status", state.snapshot());
            Ok(())
        }
        state::DictationStatus::Recording => Ok(()),
        state::DictationStatus::Transcribing => {
            Err(AppError::from("Already transcribing — please wait"))
        }
    }
}

fn stop_session(app: &AppHandle, state: &SharedState) -> AppResult<()> {
    let status = state.snapshot().status;
    match status {
        state::DictationStatus::Recording => {
            let app_bg = app.clone();
            let state_bg = Arc::clone(state);
            std::thread::spawn(move || {
                let result = state_bg.stop_and_transcribe(&app_bg);
                if let Err(e) = &result {
                    state_bg.push_log(format!("Error: {e}"));
                }
                let _ = app_bg.emit("dictation-status", state_bg.snapshot());
            });
            let _ = app.emit("dictation-status", state.snapshot());
            Ok(())
        }
        state::DictationStatus::Idle | state::DictationStatus::Error => Ok(()),
        state::DictationStatus::Transcribing => {
            Err(AppError::from("Already transcribing — please wait"))
        }
    }
}

fn toggle_dictation_inner(app: &AppHandle, state: &SharedState) -> AppResult<()> {
    let status = state.snapshot().status;
    match status {
        state::DictationStatus::Idle | state::DictationStatus::Error => start_dictation(app, state),
        state::DictationStatus::Recording => stop_session(app, state),
        state::DictationStatus::Transcribing => {
            Err(AppError::from("Already transcribing — please wait"))
        }
    }
}

fn toggle_command_inner(app: &AppHandle, state: &SharedState) -> AppResult<()> {
    let status = state.snapshot().status;
    match status {
        state::DictationStatus::Idle | state::DictationStatus::Error => start_command(app, state),
        state::DictationStatus::Recording => stop_session(app, state),
        state::DictationStatus::Transcribing => {
            Err(AppError::from("Already transcribing — please wait"))
        }
    }
}

fn handle_dictation_hotkey(app: &AppHandle, state: &SharedState, key_state: ShortcutState) {
    let result = match state.hotkey_mode() {
        HotkeyMode::Hold => match key_state {
            ShortcutState::Pressed => start_dictation(app, state),
            ShortcutState::Released => stop_session(app, state),
        },
        HotkeyMode::Toggle => match key_state {
            ShortcutState::Pressed => toggle_dictation_inner(app, state),
            ShortcutState::Released => Ok(()),
        },
    };
    if let Err(e) = result {
        state.push_log(format!("Hotkey error: {e}"));
        let _ = app.emit("dictation-status", state.snapshot());
    }
}

/// Command Mode always uses hold semantics on its own hotkey for predictability.
fn handle_command_hotkey(app: &AppHandle, state: &SharedState, key_state: ShortcutState) {
    let result = match key_state {
        ShortcutState::Pressed => start_command(app, state),
        ShortcutState::Released => {
            if state.should_ignore_command_release() {
                // Synthetic Cmd/Ctrl+C during selection capture often looks like
                // a release of the command chord — ignore those.
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

/// Stable tray menu item ids (must match `handle_tray_menu_id`).
const TRAY_SHOW: &str = "tray-show";
const TRAY_HIDE: &str = "tray-hide";
const TRAY_QUIT: &str = "tray-quit";

fn main_window(app: &AppHandle) -> Option<tauri::WebviewWindow> {
    app.get_webview_window("main")
        .or_else(|| app.webview_windows().into_values().next())
}

fn show_main_window(app: &AppHandle) {
    // On macOS, unhide the app first so a previously hidden window can appear
    // after close-to-tray (and when the Dock icon is clicked).
    #[cfg(target_os = "macos")]
    {
        let _ = app.show();
    }
    if let Some(window) = main_window(app) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn hide_main_window(app: &AppHandle) {
    if let Some(window) = main_window(app) {
        let _ = window.hide();
    }
}

fn handle_tray_menu_id(app: &AppHandle, id: &str) {
    match id {
        TRAY_SHOW => show_main_window(app),
        TRAY_HIDE => hide_main_window(app),
        TRAY_QUIT => app.exit(0),
        _ => {}
    }
}

/// Unregister all global shortcuts, then register dictation + command from settings.
fn register_app_hotkeys(app: &AppHandle, state: &SharedState) -> AppResult<()> {
    let dictation_str = state.dictation_hotkey();
    let command_str = state.command_hotkey();
    let dictation = parse_shortcut(&dictation_str)?;
    let command = parse_shortcut(&command_str)?;

    // Clear previous bindings so rebind is clean.
    let _ = app.global_shortcut().unregister_all();

    {
        let handle = app.clone();
        let state = Arc::clone(state);
        app.global_shortcut()
            .on_shortcut(dictation, move |_app, _sc, event| {
                handle_dictation_hotkey(&handle, &state, event.state);
            })
            .map_err(|e| AppError::from(format!("Register dictation hotkey failed: {e}")))?;
    }

    {
        let handle = app.clone();
        let state = Arc::clone(state);
        app.global_shortcut()
            .on_shortcut(command, move |_app, _sc, event| {
                handle_command_hotkey(&handle, &state, event.state);
            })
            .map_err(|e| AppError::from(format!("Register command hotkey failed: {e}")))?;
    }

    Ok(())
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

    if let Some(icon) = app.default_window_icon() {
        // Do NOT set icon_as_template: the default app icon is full-color; as a
        // macOS template it often becomes invisible in the menu bar.
        builder = builder.icon(icon.clone());
    }

    // Text fallback so the tray entry is findable even if the glyph is subtle.
    #[cfg(target_os = "macos")]
    {
        builder = builder.title("ES");
    }

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
            dictionary_add,
            dictionary_remove,
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
                let _ = window.hide();
            }
        })
        .setup(|app| {
            let state = Arc::clone(app.state::<SharedState>().inner());

            setup_tray(app)?;
            register_app_hotkeys(app.handle(), &state)
                .map_err(|e| e.to_string())?;

            let mode = state.hotkey_mode();
            state.push_log(format!(
                "Dictation: {} ({})",
                state.dictation_hotkey(),
                mode.as_str()
            ));
            state.push_log(format!(
                "Command Mode: {} (hold)",
                state.command_hotkey()
            ));
            state.push_log(format!("Hotkey: {}", mode.label()));
            state.push_log(format!("Model path: {}", state.snapshot().model_path));
            state.push_log(format!(
                "LLM: {} / {}",
                state.snapshot().llm_base_url,
                state.snapshot().llm_model
            ));
            state.push_log(
                "Tray: close hides · click menu-bar “ES” / Show Window · Dock click also restores · Quit from tray",
            );

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
