//! Activity log commands (PRD §16).

use tauri::State;

use crate::c8y::activity::to_curl;
use crate::error::{AppError, AppResult};
use crate::model::ActivityEntry;
use crate::state::AppState;

/// Last 500 requests, oldest first.
#[tauri::command]
#[specta::specta]
pub fn activity_log(state: State<'_, AppState>) -> AppResult<Vec<ActivityEntry>> {
    Ok(state.activity.entries())
}

#[tauri::command]
#[specta::specta]
pub fn clear_activity_log(state: State<'_, AppState>) -> AppResult<()> {
    state.activity.clear();
    Ok(())
}

/// The entry as a `curl` command (password masked). Needs no login: the entry carries the
/// base URL and user.
#[tauri::command]
#[specta::specta]
pub fn activity_as_curl(state: State<'_, AppState>, entry_id: u32) -> AppResult<String> {
    let entry = state
        .activity
        .get(entry_id)
        .ok_or_else(|| AppError::NotFound(format!("activity entry {entry_id}")))?;
    Ok(to_curl(&entry))
}

/// This file's commands (see `commands/mod.rs`).
macro_rules! activity_commands {
    ([$($acc:tt)*] $($rest:ident)*) => {
        $crate::commands::collect_all!(
            [$($acc)*
                $crate::commands::activity::activity_log,
                $crate::commands::activity::clear_activity_log,
                $crate::commands::activity::activity_as_curl,
            ]
            $($rest)*
        )
    };
}
pub(crate) use activity_commands;
