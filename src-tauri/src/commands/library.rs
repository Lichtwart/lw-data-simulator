//! Library commands (PRD §11).

use tauri::State;

use crate::error::{AppError, AppResult};
use crate::library as lib_mod;
use crate::model::{
    DemoPack, DemoSummary, GenerateSeriesRequest, LibraryInfo, LibraryItem, SlaveDefinition,
};
use crate::state::AppState;

/// Library version, demo summaries, counts, warnings.
#[tauri::command]
#[specta::specta]
pub fn library_info(state: State<'_, AppState>) -> AppResult<LibraryInfo> {
    let lib = state.library()?;
    let demos = lib
        .demos
        .iter()
        .map(|d| DemoSummary {
            id: d.id.clone(),
            name: d.name.clone(),
            description: d.description.clone(),
            tags: d.tags.clone(),
            item_count: count(lib_mod::select::select_items(&lib.items, d).len()),
            floorplan_count: count(d.floorplans.len()),
        })
        .collect();
    Ok(LibraryInfo {
        version: lib.version.clone(),
        demos,
        item_count: count(lib.items.len()),
        slave_count: count(lib.slaves.len()),
        warnings: lib.warnings.clone(),
    })
}

/// All demo packs.
#[tauri::command]
#[specta::specta]
pub fn list_demos(state: State<'_, AppState>) -> AppResult<Vec<DemoPack>> {
    Ok(state.library()?.demos.clone())
}

/// One demo pack.
#[tauri::command]
#[specta::specta]
pub fn get_demo(state: State<'_, AppState>, demo_id: String) -> AppResult<DemoPack> {
    state
        .library()?
        .demo(&demo_id)
        .cloned()
        .ok_or_else(|| AppError::NotFound(format!("demo {demo_id}")))
}

/// All library items (the frontend filters/searches; see `library::select`).
#[tauri::command]
#[specta::specta]
pub fn list_library_items(state: State<'_, AppState>) -> AppResult<Vec<LibraryItem>> {
    Ok(state.library()?.items.clone())
}

/// All slave definitions.
#[tauri::command]
#[specta::specta]
pub fn list_slaves(state: State<'_, AppState>) -> AppResult<Vec<SlaveDefinition>> {
    Ok(state.library()?.slaves.clone())
}

/// "Generate series" helper of the demo editor (PRD §11.3): clone an item template
/// `count` times with numbered ids / titles (and native indexes). Pure; nothing is saved.
#[tauri::command]
#[specta::specta]
pub fn generate_item_series(request: GenerateSeriesRequest) -> AppResult<Vec<LibraryItem>> {
    lib_mod::series::generate(&request)
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// This file's commands (see `commands/mod.rs`).
macro_rules! library_commands {
    ([$($acc:tt)*] $($rest:ident)*) => {
        $crate::commands::collect_all!(
            [$($acc)*
                $crate::commands::library::library_info,
                $crate::commands::library::list_demos,
                $crate::commands::library::get_demo,
                $crate::commands::library::list_library_items,
                $crate::commands::library::list_slaves,
                $crate::commands::library::generate_item_series,
            ]
            $($rest)*
        )
    };
}
pub(crate) use library_commands;
