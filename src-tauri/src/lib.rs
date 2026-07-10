mod audio;
mod error;
mod inject;
mod state;
mod stt;

use error::{AppError, AppResult};
use state::{AppState, SharedState, StatusSnapshot};
use stt::resolve_model_path;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

const DEFAULT_HOTKEY_LABEL: &str = "Ctrl+Shift+Space";

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
fn load_model(state: tauri::State<'_, SharedState>) -> AppResult<StatusSnapshot> {
    state.inner().ensure_engine()?;
    Ok(state.inner().snapshot())
}

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

fn toggle_dictation_inner(app: &AppHandle, state: &SharedState) -> AppResult<()> {
    let status = state.snapshot().status;
    match status {
        state::DictationStatus::Idle | state::DictationStatus::Error => {
            state.start_recording()?;
            let _ = app.emit("dictation-status", state.snapshot());
            Ok(())
        }
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
        state::DictationStatus::Transcribing => {
            Err(AppError::from("Already transcribing — please wait"))
        }
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
            load_model,
            toggle_dictation,
            cancel_dictation,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let state = Arc::clone(app.state::<SharedState>().inner());

            let shortcut = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::Space);

            app.global_shortcut().on_shortcut(shortcut, move |_app, _sc, event| {
                if event.state != ShortcutState::Pressed {
                    return;
                }
                if let Err(e) = toggle_dictation_inner(&handle, &state) {
                    state.push_log(format!("Hotkey error: {e}"));
                    let _ = handle.emit("dictation-status", state.snapshot());
                }
            })?;

            let state = app.state::<SharedState>().inner();
            state.push_log(format!("Global hotkey registered: {DEFAULT_HOTKEY_LABEL}"));
            state.push_log(format!("Model path: {}", state.snapshot().model_path));

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running TalonType");
}
