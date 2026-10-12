//! Simulation session types (PRD §7–§8).

use serde::{Deserialize, Serialize};
use specta::Type;

use super::library::{GeneratorConfig, Severity};
use super::ops::OperationPrompt;

/// Lifecycle of the (single) session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum SessionStatus {
    Starting,
    Running,
    Stopping,
    Stopped,
    /// Stopped because of an unrecoverable error (see `SessionError` event).
    Failed,
}

/// Arguments of `start_session`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StartSessionRequest {
    pub device_id: String,
    /// Take over a fresh advisory lock held by someone else.
    pub force: bool,
    /// Auto-accept visible operations (off by default).
    pub auto_accept: bool,
}

/// Arguments of `set_item_value`. A struct because specta cannot annotate a bare `f64`
/// command argument (it would become `number | null`).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SetItemValueRequest {
    pub item_id: String,
    #[specta(type = specta_typescript::Number)]
    pub value: f64,
}

/// DALI group mode (`lw_switchGroupMode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum GroupMode {
    On,
    Off,
    Auto,
}

/// Where a fault applies. Alarm types/texts/severities come from the fault catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FaultTarget {
    /// Per-lamp native alarm, e.g. `lw_lampFailure_1_NN_openCircuit`.
    DaliLamp {
        lamp: u32,
        #[serde(rename = "alarmType")]
        alarm_type: String,
    },
    /// Bus-level alarm, e.g. `lw_daliBusPower*`.
    DaliBus {
        #[serde(rename = "alarmType")]
        alarm_type: String,
    },
    /// Slave timeout: stops measurements, raises `lw_modbus_<id:02>_timeoutError`.
    SlaveTimeout {
        #[serde(rename = "slaveKey")]
        slave_key: String,
    },
    /// One custom alarm of a slave: `lw_demo_<id:02>_<alarmName>` (D2).
    SlaveAlarm {
        #[serde(rename = "slaveKey")]
        slave_key: String,
        #[serde(rename = "alarmName")]
        alarm_name: String,
    },
    /// A device-level `customAlarm` item: `lw_demo_<name>` (D2).
    CustomAlarm {
        #[serde(rename = "itemId")]
        item_id: String,
    },
    /// Native alarm of a DALI brightness / motion sensor (`lw_brightness_1_NN_lost`, …).
    /// `sensorKind` because `kind` is the tag.
    Sensor {
        /// `brightness` | `motion`.
        #[serde(rename = "sensorKind")]
        sensor_kind: String,
        /// Sensor 1–10.
        index: u8,
        /// Alarm type.
        alarm: String,
    },
}

/// One selectable fault (firmware text + severity), from `fault_catalog`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FaultDefinition {
    pub target: FaultTarget,
    /// Resolved Cumulocity alarm type.
    pub alarm_type: String,
    pub text: String,
    pub severity: Severity,
}

/// Status of an open Cumulocity alarm (DECISIONS D9). A CLEARED alarm is no longer open: its
/// fault ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OpenAlarmStatus {
    #[default]
    Active,
    Acknowledged,
}

impl OpenAlarmStatus {
    /// `ACTIVE` / `ACKNOWLEDGED` (anything else, e.g. `CLEARED`, is not an open status).
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "ACTIVE" => Some(Self::Active),
            "ACKNOWLEDGED" => Some(Self::Acknowledged),
            _ => None,
        }
    }
}

/// A fault currently in effect: fired by the presenter (or a scenario, or restored at session
/// start) and not yet cleared in Cumulocity (D9).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ActiveFault {
    pub target: FaultTarget,
    pub alarm_type: String,
    /// Cumulocity alarm id, once created.
    pub alarm_id: Option<String>,
    pub raised_at: String,
    /// Status of the alarm in Cumulocity (acknowledging changes only this; clearing ends the
    /// fault).
    pub status: OpenAlarmStatus,
}

/// One series of a measurement: name plus the unit the firmware sends (`None` = omit `unit`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SeriesNaming {
    /// e.g. `Brightness`, `Value03`, `freezer_temp`.
    pub name: String,
    pub unit: Option<String>,
}

/// Resolved Cumulocity naming of an item's measurement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MeasurementNaming {
    /// e.g. `daliControlGear`, `digitalInput`, `modbus`, `c8y_SignalStrength`.
    #[serde(rename = "type")]
    pub measurement_type: String,
    /// e.g. `LW Dali Control Gear 1-03`, `LW_Modbus_Slave_20_240`, `LW Digital Input`.
    pub fragment: String,
    /// All series under the fragment. One entry for most items; a DALI lamp has four
    /// (`Converter Status`, `Lamp Status`, `Converter Runtime`, `Brightness`). The item's value
    /// is the single series, or `Brightness` for a DALI lamp.
    pub series: Vec<SeriesNaming>,
}

/// Live state of one item in the session.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ItemRuntimeState {
    pub item_id: String,
    #[specta(type = specta_typescript::Number)]
    pub value: f64,
    /// Manual movement overrides the generator until `resume_generator`.
    pub manual_override: bool,
    /// Effective generator (library default or `lw_demo.generators` override).
    pub generator: Option<GeneratorConfig>,
    /// `None` for items without a measurement (`customAlarm`).
    pub naming: Option<MeasurementNaming>,
    /// Measurements paused (e.g. slave timeout fault active).
    pub paused: bool,
}

/// Value update for one item (`item-values-changed` event).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ItemValue {
    pub item_id: String,
    #[specta(type = specta_typescript::Number)]
    pub value: f64,
    pub manual_override: bool,
}

/// Live state of a DALI group.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DaliGroupState {
    pub id: u32,
    pub name: String,
    pub mode: GroupMode,
}

/// Complete session state. Returned by `start_session` / `get_session` and
/// emitted as `session-changed` on lifecycle changes.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SessionSnapshot {
    /// Monotonic per session; incremented on every state change the UI can observe (every
    /// `session-changed`, `operation-prompt`, `operation-resolved`, `fault-changed`,
    /// `item-values-changed` batch, `realtime-status`). Events carry the same counter.
    pub revision: u32,
    pub device_id: String,
    pub device_name: String,
    pub status: SessionStatus,
    pub started_at: String,
    pub auto_accept: bool,
    pub realistic_mode: bool,
    pub send_interval_sec: u32,
    /// Last successful measurement send (ISO-8601); a `session-changed` follows every
    /// periodic send.
    pub last_sent_at: Option<String>,
    pub realtime_connected: bool,
    /// The device's setup was edited during the session in a way the running session cannot
    /// take over (item selection, custom items / slaves, digital input type): restart to apply.
    pub restart_needed: bool,
    pub items: Vec<ItemRuntimeState>,
    pub groups: Vec<DaliGroupState>,
    pub active_faults: Vec<ActiveFault>,
    pub pending_operations: Vec<OperationPrompt>,
}
