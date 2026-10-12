//! How the Cumulocity client reports things to the rest of the app.
//!
//! `C8yClient` does not know Tauri: it talks to an [`EventSink`]. The app installs
//! [`TauriSink`] (emits `activity-logged` / `auth-expired` and cleans up the backend on 401);
//! tests use [`NoopSink`].

use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use crate::events::{ActivityLogged, AuthExpired};
use crate::model::ActivityEntry;
use crate::state::AppState;

/// Receiver of client-side notifications. Must be cheap and must not block.
pub trait EventSink: Send + Sync + 'static {
    /// A request was added to the activity log.
    fn activity_logged(&self, entry: &ActivityEntry);
    /// Cumulocity answered 401 on an authenticated call for this tenant.
    fn auth_expired(&self, tenant_key: &str);
}

/// Does nothing (unit tests, tools).
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopSink;

impl EventSink for NoopSink {
    fn activity_logged(&self, _entry: &ActivityEntry) {}
    fn auth_expired(&self, _tenant_key: &str) {}
}

/// Emits the Tauri events and, on 401, clears `AppState.auth` and stops the session in the
/// backend (the UI only has to show the login screen).
pub struct TauriSink {
    app: AppHandle,
}

impl TauriSink {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl EventSink for TauriSink {
    fn activity_logged(&self, entry: &ActivityEntry) {
        let _ = ActivityLogged(entry.clone()).emit(&self.app);
    }

    fn auth_expired(&self, tenant_key: &str) {
        let app = self.app.clone();
        let tenant_key = tenant_key.to_owned();
        tauri::async_runtime::spawn(async move {
            let state = app.state::<AppState>();
            // Only if this 401 actually ended the login: a stale client's 401 (old session,
            // other tenant, already logged out) must not send the UI to the login screen.
            if state.expire_auth(&app, &tenant_key).await {
                let _ = AuthExpired { tenant_key }.emit(&app);
            }
        });
    }
}
