//! Raw fire panel types (PRD §10).

use serde::{Deserialize, Serialize};
use specta::Type;

use super::library::Severity;

/// A single-series measurement to fire.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RawMeasurement {
    #[serde(rename = "type")]
    pub measurement_type: String,
    pub fragment: String,
    pub series: String,
    #[specta(type = specta_typescript::Number)]
    pub value: f64,
    pub unit: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RawAlarm {
    #[serde(rename = "type")]
    pub alarm_type: String,
    pub severity: Severity,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RawEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    pub text: String,
    /// Optional JSON object (as text) merged into the event body.
    pub extra_json: Option<String>,
}

/// Target API for the raw-JSON tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum RawTarget {
    Measurement,
    Alarm,
    Event,
}

/// Raw JSON body to POST. `source.id` is filled with the selected device if missing.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RawJsonRequest {
    pub target: RawTarget,
    /// JSON object as text, exactly as typed by the user.
    pub body: String,
}

/// Anything the raw fire panel can send; stored in the history for "resend".
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FireRequest {
    Measurement(RawMeasurement),
    Alarm(RawAlarm),
    Event(RawEvent),
    Json(RawJsonRequest),
}

/// One entry of the raw-fire history.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FiredRecord {
    /// Monotonic id within this app run.
    pub id: u32,
    pub device_id: String,
    pub fired_at: String,
    pub request: FireRequest,
    /// HTTP status of the POST.
    pub status: u16,
    /// Id of the created measurement / alarm / event.
    pub created_id: Option<String>,
}
