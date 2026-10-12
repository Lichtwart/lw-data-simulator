//! IPC domain types — **the** place for every serde type that crosses the
//! Rust ⇄ frontend boundary (command arguments, command results, event payloads)
//! and for the on-disk library file format.
//!
//! Rules:
//! - Every type here derives `Serialize`/`Deserialize` + `specta::Type` and uses
//!   `#[serde(rename_all = "camelCase")]`, so the generated TypeScript in
//!   `src/lib/bindings.ts` is idiomatic.
//! - Timestamps are ISO-8601 strings (UTC). Cumulocity ids are strings.
//! - No `u64`/`i64`/`usize` (they become `bigint` in TS) — use `u32`/`f64`.
//! - Module-internal types (HTTP DTOs, task messages) stay in their own module.
//!
//! After changing anything here run `npm run gen:types`.

pub mod activity;
pub mod app;
pub mod auth;
pub mod device;
pub mod library;
pub mod ops;
pub mod raw;
pub mod session;

pub use activity::*;
pub use app::*;
pub use auth::*;
pub use device::*;
pub use library::*;
pub use ops::*;
pub use raw::*;
pub use session::*;
