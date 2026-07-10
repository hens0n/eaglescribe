mod audio;
mod dictionary;
mod error;
mod inject;
mod polish;
mod settings;
mod snippets;
mod state;
mod stt;

use error::{AppError, AppResult};
use polish::PolishMode;
use settings::HotkeyMode;
use state::{AppState, SharedState, StatusSnapshot};
use stt::resolve_model_path;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

const DEFAULT_HOTKEY_COMBO: &str = "Ctrl+Shift+Space";

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
        state::DictationStatus::Recording => {
            // Already holding / recording — ignore repeat Pressed events.
            Ok(())
        }
        state::DictationStatus::Transcribing => {
            Err(AppError::from("Already transcribing — please wait"))
        }
    }
}

fn stop_dictation(app: &AppHandle, state: &SharedState) -> AppResult<()> {
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
        state::DictationStatus::Idle | state::DictationStatus::Error => {
            // Release without a prior press — ignore.
            Ok(())
        }
        state::DictationStatus::Transcribing => {
            Err(AppError::from("Already transcribing — please wait"))
        }
    }
}

fn toggle_dictation_inner(app: &AppHandle, state: &SharedState) -> AppResult<()> {
    let status = state.snapshot().status;
    match status {
        state::DictationStatus::Idle | state::DictationStatus::Error => start_dictation(app, state),
        state::DictationStatus::Recording => stop_dictation(app, state),
        state::DictationStatus::Transcribing => {
            Err(AppError::from("Already transcribing — please wait"))
        }
    }
}

fn handle_hotkey(app: &AppHandle, state: &SharedState, key_state: ShortcutState) {
    let result = match state.hotkey_mode() {
        HotkeyMode::Hold => match key_state {
            ShortcutState::Pressed => start_dictation(app, state),
            ShortcutState::Released => stop_dictation(app, state),
        },
        HotkeyMode::Toggle => match key_state {
            // Only act on press — ignore release so it behaves like the old toggle.
            ShortcutState::Pressed => toggle_dictation_inner(app, state),
            ShortcutState::Released => Ok(()),
        },
    };
    if let Err(e) = result {
        state.push_log(format!("Hotkey error: {e}"));
        let _ = app.emit("dictation-status", state.snapshot());
    }
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
            dictionary_add,
            dictionary_remove,
            snippet_add,
            snippet_remove,
            load_model,
            toggle_dictation,
            cancel_dictation,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let state = Arc::clone(app.state::<SharedState>().inner());

            let shortcut = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::Space);

            app.global_shortcut()
                .on_shortcut(shortcut, move |_app, _sc, event| {
                    handle_hotkey(&handle, &state, event.state);
                })?;

            let state = app.state::<SharedState>().inner();
            let mode = state.hotkey_mode();
            state.push_log(format!(
                "Global hotkey registered: {DEFAULT_HOTKEY_COMBO} ({})",
                mode.as_str()
            ));
            state.push_log(format!("Hotkey: {}", mode.label()));
            state.push_log("Change hold vs toggle in the UI; choice is saved.");
            state.push_log(format!("Model path: {}", state.snapshot().model_path));
            state.push_log("Polish: smart (toggle in UI for verbatim)");

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running TalonType");
}
