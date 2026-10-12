//! All `#[tauri::command]`s — the complete IPC contract (PRD features ↔ frontend).
//!
//! One file per area, so parallel agents never edit the same file:
//! [`app`], [`auth`], [`library`], [`devices`], [`session`], [`raw`], [`activity`].
//!
//! Rules:
//! - Commands are thin adapters: resolve state (`AppState`), call into the owning
//!   module (`c8y`, `library`, `device`, `sim`, `raw`), return `AppResult<T>`.
//!   Business logic belongs in the module, not here.
//! - Every command is annotated with `#[specta::specta]`.
//! - **Registration is per file**: each command file ends with a `<area>_commands!` macro
//!   listing its own commands. A new command is added there — nowhere else. [`collect`] chains
//!   the macros of all files (add a new *file* to the chain in this module only).
//! - Run `npm run gen:types` after changes.
//! - Every command returns `AppResult<T>` (even infallible ones) so the frontend
//!   wrapper in `src/lib/api.ts` can unwrap uniformly.
//! - Argument names are snake_case here and camelCase in TS (Tauri converts).
//! - **No `f64` (or `u64`/`i64`/`usize`) as a direct argument or return type**: specta cannot
//!   annotate it there. Wrap it in a request struct with
//!   `#[specta(type = specta_typescript::Number)]` (e.g. `SetItemValueRequest`).

/// Chains the per-file command macros into one `tauri_specta::collect_commands!` call.
/// Each per-file macro receives the paths collected so far plus the macros still to visit,
/// appends its own commands and continues the chain.
macro_rules! collect_all {
    ([$($acc:tt)*]) => {
        tauri_specta::collect_commands![$($acc)*]
    };
    ([$($acc:tt)*] $next:ident $($rest:ident)*) => {
        $next!([$($acc)*] $($rest)*)
    };
}
pub(crate) use collect_all;

pub mod activity;
pub mod app;
pub mod auth;
pub mod devices;
pub mod library;
pub mod raw;
pub mod session;

use activity::activity_commands;
use app::app_commands;
use auth::auth_commands;
use devices::devices_commands;
use library::library_commands;
use raw::raw_commands;
use session::session_commands;

/// All commands, for `specta_builder()` (invoke handler + generated bindings).
pub fn collect() -> tauri_specta::Commands<tauri::Wry> {
    collect_all!([]
        app_commands
        auth_commands
        library_commands
        devices_commands
        session_commands
        raw_commands
        activity_commands
    )
}
