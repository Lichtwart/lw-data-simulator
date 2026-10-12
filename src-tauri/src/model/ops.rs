//! Cumulocity operation handling types (PRD §9).

use serde::{Deserialize, Serialize};
use specta::Type;

use super::library::LocalizedText;
use super::session::GroupMode;

/// Operations that need presenter confirmation ("visible operations").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum VisibleOperationKind {
    /// `lw_switchGroupMode`
    SwitchGroupMode,
    /// `lw_switchConverterState`
    SwitchConverterState,
    /// `lw_modbusWrite`
    ModbusWrite,
}

/// The parameters of a visible operation, for the UI to word in the current language.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum OperationDetail {
    GroupMode {
        group: u32,
        mode: GroupMode,
    },
    ConverterState {
        converter: u32,
        on: bool,
    },
    ModbusWrite {
        slave_address: u32,
        register_address: u32,
        #[specta(type = specta_typescript::Number)]
        value: f64,
        coil: bool,
        /// Title of the library item behind the register, if known.
        item_title: Option<LocalizedText>,
    },
}

/// A visible operation waiting for Confirm / Abort. Emitted as `operation-prompt`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OperationPrompt {
    /// Session revision when the prompt was raised (see `SessionSnapshot::revision`).
    pub revision: u32,
    pub device_id: String,
    pub operation_id: String,
    pub kind: VisibleOperationKind,
    /// Technical English summary built in Rust (e.g. `Group 3 → On`): for logs and as a
    /// fallback. The UI words the prompt from `detail` in the current language.
    pub summary: String,
    pub detail: OperationDetail,
    /// `lw_*.by` of the operation, else `creationRequest.user` / `user`, if present.
    pub by: Option<String>,
    pub received_at: String,
    /// `received_at + 60 s`; after this the backend fails it with "Timed out".
    pub expires_at: String,
}

/// Presenter decision for a visible operation.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(tag = "decision", rename_all = "camelCase")]
pub enum OperationDecision {
    Confirm,
    /// `reason` becomes the `failureReason` (default `Aborted by user`).
    Abort {
        #[serde(default)]
        reason: Option<String>,
    },
}

/// Final operation status reported back to Cumulocity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OperationStatus {
    Pending,
    Executing,
    Successful,
    Failed,
}

/// Why an operation reached its final state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum OperationResolution {
    /// Config operation, answered without a prompt.
    AutoAnswered,
    /// Auto-accept switch was on.
    AutoAccepted,
    Confirmed,
    Aborted,
    TimedOut,
    /// Stale PENDING operation found at session start.
    SimulatorOffline,
    /// Applying the effect failed.
    Error,
    /// The operation became final elsewhere while its prompt was open (the plugin cancelled it
    /// or another client answered); the prompt was withdrawn. `status` / `reason` are what was
    /// observed.
    Withdrawn,
    /// The presenter stopped the session while the prompt was open (the operation is FAILED
    /// "Simulator stopped"). Not an error: the UI shows nothing.
    Stopped,
}

/// An operation reached SUCCESSFUL / FAILED. Emitted as `operation-resolved`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OperationResolved {
    /// Session revision after the resolution (see `SessionSnapshot::revision`).
    pub revision: u32,
    pub device_id: String,
    pub operation_id: String,
    /// Cumulocity fragment name, e.g. `lw_fetchCore`.
    pub operation_type: String,
    pub status: OperationStatus,
    pub resolution: OperationResolution,
    /// `failureReason` sent to Cumulocity, if failed.
    pub reason: Option<String>,
}
