//! Every event the backend emits to the frontend.
//!
//! Each event is a struct deriving [`tauri_specta::Event`]; the Tauri event name is
//! the kebab-case struct name (e.g. [`SessionChanged`] → `session-changed`).
//! The frontend subscribes through the generated `events.*` in
//! `src/lib/bindings.ts` (wrapped by `src/lib/api.ts`). Emit with
//! `SessionChanged(snapshot).emit(&app_handle)` (trait `tauri_specta::Event`).
//!
//! Adding an event: define it here, add it to [`collect`], run `npm run gen:types`,
//! and list it in `docs/ARCHITECTURE.md`.

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri_specta::{Event, Events, collect_events};

use crate::model::{
    ActiveFault, ActivityEntry, FaultTarget, FiredRecord, ItemValue, OperationPrompt,
    OperationResolved, SessionSnapshot,
};

/// `session-changed` — session lifecycle or configuration changed
/// (start, stop, auto-accept, interval, group modes). `None` = no session.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct SessionChanged(pub Option<SessionSnapshot>);

/// `item-values-changed` — one or more item values changed (generator tick,
/// manual change, scenario, operation effect). Batched per tick.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct ItemValuesChanged {
    /// Session revision of this batch (`SessionSnapshot::revision`).
    pub revision: u32,
    pub device_id: String,
    pub values: Vec<ItemValue>,
}

/// `fault-changed` — a fault was raised or cleared.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct FaultChanged {
    /// Session revision after the change (`SessionSnapshot::revision`).
    pub revision: u32,
    pub device_id: String,
    pub target: FaultTarget,
    /// Present while raised, `None` once cleared.
    pub fault: Option<ActiveFault>,
}

/// `operation-prompt` — a visible operation needs Confirm / Abort (60 s).
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[tauri_specta(event_name = "operation-prompt")]
pub struct OperationPromptEvent(pub OperationPrompt);

/// `operation-resolved` — an operation reached SUCCESSFUL / FAILED.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[tauri_specta(event_name = "operation-resolved")]
pub struct OperationResolvedEvent(pub OperationResolved);

/// `realtime-status` — Cumulocity realtime (operations) connection state.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct RealtimeStatus {
    /// Session revision (see `SessionSnapshot.revision`).
    pub revision: u32,
    pub device_id: String,
    pub connected: bool,
    pub error: Option<String>,
}

/// `session-error` — non-fatal or fatal problem inside the session task.
/// `code` matches an `AppError` code for i18n.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct SessionError {
    pub code: String,
    pub message: String,
    /// The session stopped because of it.
    pub fatal: bool,
}

/// `activity-logged` — a request was added to the activity log.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct ActivityLogged(pub ActivityEntry);

/// `raw-fired` — a raw-fire request completed (history entry).
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct RawFired(pub FiredRecord);

/// `auth-expired` — Cumulocity answered 401 on an authenticated call;
/// the frontend returns to the login screen.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct AuthExpired {
    pub tenant_key: String,
}

/// All events, registered with the tauri-specta builder.
pub fn collect() -> Events {
    collect_events![
        SessionChanged,
        ItemValuesChanged,
        FaultChanged,
        OperationPromptEvent,
        OperationResolvedEvent,
        RealtimeStatus,
        SessionError,
        ActivityLogged,
        RawFired,
        AuthExpired,
    ]
}
