//! How the session reports to the frontend. The task never knows Tauri: it emits
//! [`SessionEvent`]s through a [`SessionEmitter`] (`TauriEmitter` in the app, a recorder in
//! tests).

use std::sync::{Arc, Mutex};

use tauri::AppHandle;
use tauri_specta::Event;

use crate::events::{
    FaultChanged, ItemValuesChanged, OperationPromptEvent, OperationResolvedEvent, RealtimeStatus,
    SessionChanged, SessionError,
};
use crate::model::SessionSnapshot;

/// Everything a session emits.
#[derive(Debug, Clone)]
pub enum SessionEvent {
    Changed(Option<SessionSnapshot>),
    Values(ItemValuesChanged),
    Fault(FaultChanged),
    Prompt(OperationPromptEvent),
    Resolved(OperationResolvedEvent),
    Realtime(RealtimeStatus),
    Error(SessionError),
}

/// Sink for session events.
pub trait SessionEmitter: Send + Sync + 'static {
    fn emit(&self, event: SessionEvent);
}

/// Emits Tauri events (the app).
pub struct TauriEmitter(pub AppHandle);

impl SessionEmitter for TauriEmitter {
    fn emit(&self, event: SessionEvent) {
        let app = &self.0;
        let res = match event {
            SessionEvent::Changed(s) => SessionChanged(s).emit(app),
            SessionEvent::Values(e) => e.emit(app),
            SessionEvent::Fault(e) => e.emit(app),
            SessionEvent::Prompt(e) => e.emit(app),
            SessionEvent::Resolved(e) => e.emit(app),
            SessionEvent::Realtime(e) => e.emit(app),
            SessionEvent::Error(e) => e.emit(app),
        };
        if let Err(e) = res {
            log::warn!("emitting a session event failed: {e}");
        }
    }
}

/// Records events (tests).
#[derive(Default)]
pub struct RecordingEmitter(pub Mutex<Vec<SessionEvent>>);

impl RecordingEmitter {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn events(&self) -> Vec<SessionEvent> {
        self.0.lock().map(|g| g.clone()).unwrap_or_default()
    }
}

impl SessionEmitter for RecordingEmitter {
    fn emit(&self, event: SessionEvent) {
        if let Ok(mut g) = self.0.lock() {
            g.push(event);
        }
    }
}

/// `AppError` code of an error (`internal` if it cannot be read), for `session-error.code`.
pub fn error_code(e: &crate::error::AppError) -> String {
    serde_json::to_value(e)
        .ok()
        .and_then(|v| v.get("code").and_then(|c| c.as_str().map(str::to_owned)))
        .unwrap_or_else(|| "internal".to_owned())
}
