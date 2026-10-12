//! The single error type that crosses the IPC boundary.
//!
//! Every `#[tauri::command]` returns `Result<T, AppError>`. The frontend receives the
//! serialized form `{ code: "...", details?: ... }` and maps `code` to an i18n key
//! (`errors.<code>` in `src/i18n/*.json`). Never put user-facing (translated) text in
//! here — `details` is diagnostic context, not UI copy.
//!
//! Owned by: foundation. Feature modules may add variants, but must keep the
//! `code` names stable and add matching i18n keys.

use serde::Serialize;
use specta::Type;

/// Result alias used throughout the backend.
pub type AppResult<T> = Result<T, AppError>;

/// Serializable application error. Adjacently tagged: `{ "code": "...", "details": ... }`.
#[derive(Debug, Clone, thiserror::Error, Serialize, Type)]
#[serde(tag = "code", content = "details", rename_all = "camelCase")]
pub enum AppError {
    /// No user is logged in (call `login` first).
    #[error("not authenticated")]
    NotAuthenticated,

    /// Cumulocity rejected the credentials (401 on `GET /user/currentUser`). `details` is the
    /// server's reason when it gave one ("User account is locked", "password expired" …).
    #[error("authentication failed ({0:?})")]
    AuthFailed(Option<String>),

    /// Non-2xx response from Cumulocity.
    #[error("HTTP {status}: {message}")]
    Http { status: u16, message: String },

    /// Transport-level failure (DNS, TLS, timeout, connection reset).
    #[error("network error: {0}")]
    Network(String),

    /// A device, item, demo, scenario, operation … does not exist.
    #[error("not found: {0}")]
    NotFound(String),

    /// The request from the frontend is malformed or violates a rule
    /// (e.g. slave name > 19 chars, invalid raw JSON).
    #[error("invalid input: {0}")]
    InvalidInput(String),

    /// The bundled library failed to load or validate.
    #[error("library error: {0}")]
    Library(String),

    /// OS keychain access failed.
    #[error("keychain error: {0}")]
    Keychain(String),

    /// A session command was issued while no session is running.
    #[error("no active session")]
    NoSession,

    /// `start_session` was called while another session is running.
    /// `details` is the device id of the running session.
    #[error("a session is already running for device {0}")]
    SessionActive(String),

    /// The device carries a fresh advisory lock from another user
    /// (`lw_demo.lock`, younger than 10 min). Retry with `force: true` to take over.
    #[error("device in use by {user} since {since}")]
    DeviceLocked { user: String, since: String },

    /// File system error (export, library resources).
    #[error("io error: {0}")]
    Io(String),

    /// Anything else. Should be rare; prefer a specific variant.
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        Self::Internal(format!("json: {e}"))
    }
}

impl From<reqwest::Error> for AppError {
    fn from(e: reqwest::Error) -> Self {
        match e.status() {
            Some(s) => Self::Http {
                status: s.as_u16(),
                message: e.to_string(),
            },
            None => Self::Network(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_adjacently_tagged() {
        let v = serde_json::to_value(AppError::NoSession).unwrap();
        assert_eq!(v, serde_json::json!({ "code": "noSession" }));

        let v = serde_json::to_value(AppError::Http {
            status: 404,
            message: "x".into(),
        })
        .unwrap();
        assert_eq!(
            v,
            serde_json::json!({ "code": "http", "details": { "status": 404, "message": "x" } })
        );
    }
}
