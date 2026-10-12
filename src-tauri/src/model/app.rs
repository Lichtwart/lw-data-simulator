//! App-level info types.

use serde::{Deserialize, Serialize};
use specta::Type;

/// Static information about the running app and its bundled library.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    /// App version (== library version, SemVer).
    pub version: String,
    /// Version of the bundled `library/` (from the app version, see PRD §11.1).
    pub library_version: String,
    /// `macos` / `windows` / ….
    pub platform: String,
}
