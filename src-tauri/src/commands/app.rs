//! App-level commands.

use tauri::AppHandle;

use crate::error::AppResult;
use crate::model::AppInfo;

/// App + library version and platform.
#[tauri::command]
#[specta::specta]
pub fn app_info(app: AppHandle) -> AppResult<AppInfo> {
    let version = app.package_info().version.to_string();
    Ok(AppInfo {
        library_version: version.clone(),
        version,
        platform: std::env::consts::OS.to_owned(),
    })
}

/// This file's commands (see `commands/mod.rs`).
macro_rules! app_commands {
    ([$($acc:tt)*] $($rest:ident)*) => {
        $crate::commands::collect_all!(
            [$($acc)* $crate::commands::app::app_info,]
            $($rest)*
        )
    };
}
pub(crate) use app_commands;
