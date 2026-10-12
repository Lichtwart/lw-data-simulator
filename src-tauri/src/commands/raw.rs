//! Raw fire commands (PRD §10).

use tauri::{AppHandle, State};
use tauri_specta::Event;

use crate::error::{AppError, AppResult};
use crate::events::RawFired;
use crate::model::{FireRequest, FiredRecord};
use crate::raw;
use crate::state::AppState;

/// Fire a raw measurement / alarm / event / JSON body at a device (`time` and `source.id`
/// are filled in); stored in history and emitted as `raw-fired`. A failed POST is an error
/// and is not stored (it is in the activity log).
#[tauri::command]
#[specta::specta]
pub async fn fire_raw(
    app: AppHandle,
    state: State<'_, AppState>,
    device_id: String,
    request: FireRequest,
) -> AppResult<FiredRecord> {
    fire(&app, &state, device_id, request).await
}

async fn fire(
    app: &AppHandle,
    state: &AppState,
    device_id: String,
    request: FireRequest,
) -> AppResult<FiredRecord> {
    let record = raw::fire(&state.client().await?, &device_id, &request).await?;
    let record = state.raw_history.lock().await.push(record);
    let _ = RawFired(record.clone()).emit(app);
    Ok(record)
}

/// Recent raw-fire history, oldest first.
#[tauri::command]
#[specta::specta]
pub async fn raw_history(state: State<'_, AppState>) -> AppResult<Vec<FiredRecord>> {
    Ok(state.raw_history.lock().await.entries())
}

/// Resend a history entry to the same device (fresh `time`; stored as a new entry).
#[tauri::command]
#[specta::specta]
pub async fn resend_raw(
    app: AppHandle,
    state: State<'_, AppState>,
    record_id: u32,
) -> AppResult<FiredRecord> {
    let (device_id, request) = {
        let hist = state.raw_history.lock().await;
        let r = hist
            .get(record_id)
            .ok_or_else(|| AppError::NotFound(format!("raw record {record_id}")))?;
        (r.device_id.clone(), r.request.clone())
    };
    fire(&app, &state, device_id, request).await
}

/// This file's commands (see `commands/mod.rs`).
macro_rules! raw_commands {
    ([$($acc:tt)*] $($rest:ident)*) => {
        $crate::commands::collect_all!(
            [$($acc)*
                $crate::commands::raw::fire_raw,
                $crate::commands::raw::raw_history,
                $crate::commands::raw::resend_raw,
            ]
            $($rest)*
        )
    };
}
pub(crate) use raw_commands;
