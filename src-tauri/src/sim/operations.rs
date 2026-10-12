//! Operation handling (PRD §9, firmware-core-v2.md §4, floorplan-plugin.md §5).
//!
//! Rules:
//! - Operations arrive via realtime, possibly more than once (PENDING list on every (re)connect,
//!   CREATE + UPDATE echoes). Each id is handled **once** ([`OperationTracker::seen`]).
//! - Operations already FAILED / SUCCESSFUL are ignored (the plugin cancels PENDING ops after
//!   10 s and supersedes older group-mode ops); EXECUTING ones are ours or another device's.
//!   A prompt whose operation turns FAILED / SUCCESSFUL elsewhere is withdrawn
//!   (`operation-resolved` with `withdrawn`); when it was FAILED (the plugin's cancel) the
//!   simulator re-asserts FAILED after its own EXECUTING, so a late EXECUTING can never leave
//!   the operation hanging.
//! - PENDING operations from the list fetched on (re)connect that are older than 60 s on the
//!   **server's** clock (the client measures the skew from the `Date` header) → FAILED
//!   "Simulator was offline". Live operations are never "offline".
//! - Config operations → SUCCESSFUL at once (`c8y_MeasurementRequestOperation` also sends all
//!   values).
//! - Visible operations → EXECUTING at once, prompt (Confirm / Abort, 60 s → "Timed out"), or
//!   applied directly with auto-accept. Invalid payloads fail with the firmware's texts.
//! - Unknown operations → FAILED "Operation command not found or timeout" (firmware text).
//!
//! Status `PUT`s: EXECUTING goes to the concurrent priority lane at once; the final status is
//! queued on the normal lane after the operation's effects (readback, group event), and only
//! once EXECUTING has been answered (`SessionCore::gated`).

use std::collections::HashMap;
use std::time::Duration;

use serde_json::Value;

use crate::model::{
    GroupMode, LocalizedText, OperationDetail, OperationStatus, VisibleOperationKind,
};

/// Operations answered SUCCESSFUL without a prompt (prefix `lw_fetch` also matches).
pub const AUTO_ANSWERED: &[&str] = &[
    "c8y_Restart",
    "c8y_MeasurementRequestOperation",
    "c8y_Firmware",
    "lw_daliScan",
    "lw_daliVendorScan",
    "lw_daliReset",
    "lw_setGroupColor",
];

/// Operations that need presenter confirmation.
pub const VISIBLE: &[&str] = &[
    "lw_switchGroupMode",
    "lw_switchConverterState",
    "lw_modbusWrite",
];

/// Prompt timeout; also the age after which PENDING operations are failed as offline.
pub const PROMPT_TIMEOUT: Duration = Duration::from_secs(60);

/// `failureReason` texts.
pub const REASON_TIMED_OUT: &str = "Timed out";
pub const REASON_OFFLINE: &str = "Simulator was offline";
pub const REASON_ABORTED: &str = "Aborted by user";
pub const REASON_STOPPED: &str = "Simulator stopped";
pub const REASON_UNKNOWN: &str = "Operation command not found or timeout";

/// How an incoming operation is handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationClass {
    AutoAnswer,
    Visible,
    Unsupported,
}

/// Classify an operation by its fragment names.
pub fn classify(fragment: &str) -> OperationClass {
    if fragment.starts_with("lw_fetch") || AUTO_ANSWERED.contains(&fragment) {
        OperationClass::AutoAnswer
    } else if VISIBLE.contains(&fragment) {
        OperationClass::Visible
    } else {
        OperationClass::Unsupported
    }
}

/// Standard operation fields that are not the command fragment.
const STANDARD_KEYS: &[&str] = &[
    "id",
    "self",
    "deviceId",
    "deviceName",
    "status",
    "creationTime",
    "description",
    "failureReason",
    "bulkOperationId",
    "delivery",
    "deviceExternalIDs",
];

/// The command fragment of an operation: the first known one, else the first non-standard key.
pub fn fragment_of(op: &Value) -> String {
    let Some(obj) = op.as_object() else {
        return "unknown".into();
    };
    if let Some(k) = obj
        .keys()
        .find(|k| classify(k) != OperationClass::Unsupported)
    {
        return k.clone();
    }
    obj.keys()
        .find(|k| !STANDARD_KEYS.contains(&k.as_str()) && !k.starts_with("com_cumulocity"))
        .cloned()
        .unwrap_or_else(|| "unknown".into())
}

/// A parsed visible operation.
#[derive(Debug, Clone, PartialEq)]
pub enum VisibleOp {
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
        /// Raw value (an integer), echoed exactly in the readback measurement.
        value: f64,
        coil: bool,
    },
}

impl VisibleOp {
    pub fn kind(&self) -> VisibleOperationKind {
        match self {
            Self::GroupMode { .. } => VisibleOperationKind::SwitchGroupMode,
            Self::ConverterState { .. } => VisibleOperationKind::SwitchConverterState,
            Self::ModbusWrite { .. } => VisibleOperationKind::ModbusWrite,
        }
    }
}

fn int_field(v: &Value, key: &str) -> Option<u64> {
    let x = v.get(key)?;
    x.as_u64().or_else(|| {
        x.as_f64()
            .filter(|f| *f >= 0.0 && f.fract() == 0.0)
            .map(|f| f as u64)
            .or_else(|| x.as_str().and_then(|s| s.trim().parse().ok()))
    })
}

/// Parse a visible operation's fragment; `Err` carries the firmware's failure text.
pub fn parse_visible(fragment: &str, body: &Value) -> Result<VisibleOp, String> {
    match fragment {
        "lw_switchGroupMode" => {
            let group = int_field(body, "group")
                .filter(|g| (1..=16).contains(g))
                .ok_or("The specified lw_switchGroupMode.group is incorrect")?;
            // The firmware matches the lowercase tokens case-sensitively.
            let mode = match body.get("value").and_then(Value::as_str) {
                Some("on") => GroupMode::On,
                Some("off") => GroupMode::Off,
                Some("auto") => GroupMode::Auto,
                _ => return Err("The specified lw_switchGroupMode.value is incorrect".into()),
            };
            Ok(VisibleOp::GroupMode {
                group: group as u32,
                mode,
            })
        }
        "lw_switchConverterState" => {
            let converter = int_field(body, "converter")
                .filter(|c| (1..=64).contains(c))
                .ok_or("The specified lw_switchConverterState.converter is incorrect")?;
            let on = match body.get("value") {
                Some(Value::String(s)) if s == "true" || s == "on" => true,
                Some(Value::String(s)) if s == "false" || s == "off" => false,
                Some(Value::Bool(b)) => *b,
                _ => {
                    return Err("The specified lw_switchConverterState.value is incorrect".into());
                }
            };
            Ok(VisibleOp::ConverterState {
                converter: converter as u32,
                on,
            })
        }
        "lw_modbusWrite" => {
            let slave_address = int_field(body, "slaveAddress")
                .filter(|a| (1..=247).contains(a))
                .ok_or("Slave address out of range (1-247)")?;
            let register_address = int_field(body, "registerAddress")
                .filter(|a| *a <= 65535)
                .ok_or("The specified lw_modbusWrite.registerAddress is incorrect")?;
            let coil = match body.get("type").and_then(Value::as_str) {
                Some("coil") => true,
                Some("register") => false,
                _ => return Err("The specified lw_modbusWrite.type is incorrect".into()),
            };
            let value = body
                .get("value")
                .and_then(|v| {
                    v.as_f64()
                        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                })
                .ok_or("The specified lw_modbusWrite.value is incorrect")?;
            if value.fract() != 0.0 {
                // The plugin always rounds; a hand-made operation must be an integer too.
                return Err("The specified lw_modbusWrite.value must be an integer".into());
            }
            if coil && value != 0.0 && value != 1.0 {
                return Err("Coil value must be 0 or 1".into());
            }
            if !coil && !(0.0..=65535.0).contains(&value) {
                return Err("Register value out of range (0-65535)".into());
            }
            Ok(VisibleOp::ModbusWrite {
                slave_address: slave_address as u32,
                register_address: register_address as u32,
                value,
                coil,
            })
        }
        other => Err(format!("{other} is not a visible operation")),
    }
}

/// Human-readable, technical summary (not translated).
pub fn summary(op: &VisibleOp, item_title: Option<&str>) -> String {
    match op {
        VisibleOp::GroupMode { group, mode } => {
            format!(
                "Group {group} → {}",
                crate::payload::event::group_mode_label(*mode)
            )
        }
        VisibleOp::ConverterState { converter, on } => {
            format!("Lamp {converter} → {}", if *on { "on" } else { "off" })
        }
        VisibleOp::ModbusWrite {
            slave_address,
            register_address,
            value,
            coil,
        } => {
            let kind = if *coil { "coil" } else { "register" };
            let v = crate::payload::json_number(*value);
            match item_title {
                Some(t) => format!("{t}: Modbus {slave_address}:{register_address} ({kind}) ← {v}"),
                None => format!("Modbus {slave_address}:{register_address} ({kind}) ← {v}"),
            }
        }
    }
}

/// The structured parameters of a visible operation (the UI words them per language).
pub fn detail(op: &VisibleOp, item_title: Option<&LocalizedText>) -> OperationDetail {
    match op {
        VisibleOp::GroupMode { group, mode } => OperationDetail::GroupMode {
            group: *group,
            mode: *mode,
        },
        VisibleOp::ConverterState { converter, on } => OperationDetail::ConverterState {
            converter: *converter,
            on: *on,
        },
        VisibleOp::ModbusWrite {
            slave_address,
            register_address,
            value,
            coil,
        } => OperationDetail::ModbusWrite {
            slave_address: *slave_address,
            register_address: *register_address,
            value: *value,
            coil: *coil,
            item_title: item_title.cloned(),
        },
    }
}

/// What to do with an incoming operation (pure triage).
#[derive(Debug, Clone, PartialEq)]
pub enum Triage {
    /// Duplicate, already final, EXECUTING, or malformed.
    Ignore,
    /// Final elsewhere while we prompt for it: withdraw the prompt. `status` is what was seen
    /// (`FAILED` / `SUCCESSFUL`), `reason` its `failureReason`.
    Withdraw {
        id: String,
        status: String,
        reason: Option<String>,
    },
    /// Stale PENDING → FAILED "Simulator was offline".
    Offline { id: String, fragment: String },
    /// Config op → SUCCESSFUL.
    Auto { id: String, fragment: String },
    /// Visible op → EXECUTING + prompt / auto-accept (or FAILED with the parse error).
    Visible {
        id: String,
        fragment: String,
        by: Option<String>,
        parsed: Result<VisibleOp, String>,
    },
    /// Unknown op → FAILED.
    Unsupported { id: String, fragment: String },
}

/// Operations seen by the session.
#[derive(Debug, Default)]
pub struct OperationTracker {
    /// Ids handled (once each).
    seen: HashMap<String, ()>,
}

impl OperationTracker {
    /// Triage one realtime operation. `prompting(id)` tells whether a prompt is open for it;
    /// `now` is the **server's** wall-clock UTC (compared with `creationTime`); `initial`: the
    /// operation comes from the PENDING list fetched on (re)connect (only those can be stale).
    pub fn triage(
        &mut self,
        op: &Value,
        prompting: impl Fn(&str) -> bool,
        now: chrono::DateTime<chrono::Utc>,
        initial: bool,
    ) -> Triage {
        let Some(id) = op.get("id").and_then(|v| {
            v.as_str()
                .map(str::to_owned)
                .or_else(|| v.as_u64().map(|n| n.to_string()))
        }) else {
            return Triage::Ignore;
        };
        let status = op
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("PENDING");
        match status {
            "FAILED" | "SUCCESSFUL" => {
                self.seen.insert(id.clone(), ());
                return if prompting(&id) {
                    Triage::Withdraw {
                        id,
                        status: status.to_owned(),
                        reason: op
                            .get("failureReason")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                    }
                } else {
                    Triage::Ignore
                };
            }
            "PENDING" => {}
            _ => return Triage::Ignore,
        }
        if self.seen.insert(id.clone(), ()).is_some() {
            return Triage::Ignore;
        }
        let fragment = fragment_of(op);
        let stale = initial
            && op
                .get("creationTime")
                .and_then(Value::as_str)
                .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                .is_some_and(|t| {
                    now - t.with_timezone(&chrono::Utc)
                        > chrono::Duration::from_std(PROMPT_TIMEOUT).unwrap_or_default()
                });
        if stale {
            return Triage::Offline { id, fragment };
        }
        match classify(&fragment) {
            OperationClass::AutoAnswer => Triage::Auto { id, fragment },
            OperationClass::Unsupported => Triage::Unsupported { id, fragment },
            OperationClass::Visible => {
                let body = op.get(&fragment).cloned().unwrap_or(Value::Null);
                let by = body
                    .get("by")
                    .and_then(Value::as_str)
                    .or_else(|| op.pointer("/creationRequest/user").and_then(Value::as_str))
                    .or_else(|| op.get("user").and_then(Value::as_str))
                    .map(str::to_owned);
                let parsed = parse_visible(&fragment, &body);
                Triage::Visible {
                    id,
                    fragment,
                    by,
                    parsed,
                }
            }
        }
    }
}

/// A visible operation waiting for an answer.
#[derive(Debug, Clone)]
pub struct OpenPrompt {
    pub prompt: crate::model::OperationPrompt,
    pub fragment: String,
    pub op: VisibleOp,
    pub deadline: tokio::time::Instant,
    /// The same moment on the wall clock: a sleeping laptop stops `deadline`.
    pub expires_wall: chrono::DateTime<chrono::Utc>,
}

/// Final status for a resolution: `(status, failureReason)`.
pub fn final_status(result: &Result<(), String>) -> (OperationStatus, Option<String>) {
    match result {
        Ok(()) => (OperationStatus::Successful, None),
        Err(r) => (OperationStatus::Failed, Some(r.clone())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn now() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339("2026-10-12T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc)
    }

    #[test]
    fn detail_carries_the_title_in_both_languages() {
        let op = VisibleOp::ModbusWrite {
            slave_address: 20,
            register_address: 40001,
            value: 7.0,
            coil: false,
        };
        let title = LocalizedText {
            de: "Sollwert".into(),
            en: "Setpoint".into(),
        };
        assert_eq!(
            detail(&op, Some(&title)),
            OperationDetail::ModbusWrite {
                slave_address: 20,
                register_address: 40001,
                value: 7.0,
                coil: false,
                item_title: Some(title),
            }
        );
        let json = serde_json::to_value(detail(&op, None)).unwrap();
        assert_eq!(json["kind"], "modbusWrite");
        assert_eq!(json["slaveAddress"], 20);
    }

    #[test]
    fn classifies() {
        assert_eq!(classify("lw_fetchCore"), OperationClass::AutoAnswer);
        assert_eq!(classify("c8y_Restart"), OperationClass::AutoAnswer);
        assert_eq!(classify("lw_modbusWrite"), OperationClass::Visible);
        assert_eq!(classify("c8y_Command"), OperationClass::Unsupported);
    }

    #[test]
    fn fragment_detection_skips_standard_keys() {
        let op = json!({"id": "1", "status": "PENDING", "description": "x", "deviceId": "7",
                        "lw_switchGroupMode": {"group": 1, "value": "on"}});
        assert_eq!(fragment_of(&op), "lw_switchGroupMode");
        let op = json!({"id": "1", "status": "PENDING", "c8y_Command": {"text": "x"}});
        assert_eq!(fragment_of(&op), "c8y_Command");
    }

    #[test]
    fn triage_dedups_and_ignores_final() {
        let mut t = OperationTracker::default();
        let op = json!({"id": "5", "status": "PENDING", "creationTime": "2026-10-12T11:59:55Z",
                        "lw_fetchCore": true});
        assert!(matches!(
            t.triage(&op, |_| false, now(), true),
            Triage::Auto { .. }
        ));
        assert_eq!(t.triage(&op, |_| false, now(), true), Triage::Ignore);
        let failed = json!({"id": "6", "status": "FAILED", "lw_modbusWrite": {}});
        assert_eq!(t.triage(&failed, |_| false, now(), false), Triage::Ignore);
        // A later PENDING echo of a FAILED op is ignored too.
        let pending = json!({"id": "6", "status": "PENDING", "lw_modbusWrite": {}});
        assert_eq!(t.triage(&pending, |_| false, now(), false), Triage::Ignore);
        let executing = json!({"id": "7", "status": "EXECUTING", "lw_fetchCore": true});
        assert_eq!(
            t.triage(&executing, |_| false, now(), false),
            Triage::Ignore
        );
    }

    #[test]
    fn triage_withdraws_open_prompt_and_fails_stale() {
        let mut t = OperationTracker::default();
        let failed = json!({"id": "9", "status": "FAILED", "lw_switchGroupMode": {}});
        assert_eq!(
            t.triage(&failed, |id| id == "9", now(), false),
            Triage::Withdraw {
                id: "9".into(),
                status: "FAILED".into(),
                reason: None
            }
        );
        let old = json!({"id": "10", "status": "PENDING", "creationTime": "2026-10-12T11:58:00Z",
                         "lw_switchGroupMode": {"group": 1, "value": "on"}});
        assert!(matches!(
            t.triage(&old, |_| false, now(), true),
            Triage::Offline { .. }
        ));
        let unknown = json!({"id": "11", "status": "PENDING", "c8y_Command": {}});
        assert!(matches!(
            t.triage(&unknown, |_| false, now(), false),
            Triage::Unsupported { .. }
        ));
    }

    #[test]
    fn by_falls_back_to_creation_request_user() {
        let mut t = OperationTracker::default();
        let op = |id: &str, extra: serde_json::Value| {
            let mut o = json!({ "id": id, "status": "PENDING", "creationTime": "2026-10-12T11:59:55Z",
                                "lw_switchGroupMode": { "group": 1, "value": "on" } });
            for (k, v) in extra.as_object().unwrap() {
                o[k] = v.clone();
            }
            o
        };
        let by = |tr: Triage| match tr {
            Triage::Visible { by, .. } => by,
            other => panic!("{other:?}"),
        };
        assert_eq!(
            by(t.triage(
                &op("1", json!({ "creationRequest": { "user": "anna" } })),
                |_| false,
                now(),
                false
            )),
            Some("anna".into())
        );
        assert_eq!(
            by(t.triage(&op("2", json!({ "user": "bob" })), |_| false, now(), false)),
            Some("bob".into())
        );
        let mut with_by = op("3", json!({ "user": "bob" }));
        with_by["lw_switchGroupMode"]["by"] = json!("carl");
        assert_eq!(
            by(t.triage(&with_by, |_| false, now(), false)),
            Some("carl".into())
        );
    }

    #[test]
    fn only_the_initial_list_can_be_stale() {
        let mut t = OperationTracker::default();
        let old = json!({ "id": "1", "status": "PENDING", "creationTime": "2026-10-12T11:50:00Z",
                          "lw_switchGroupMode": { "group": 1, "value": "on" } });
        assert!(matches!(
            t.triage(&old, |_| false, now(), false),
            Triage::Visible { .. }
        ));
        let old2 = json!({ "id": "2", "status": "PENDING", "creationTime": "2026-10-12T11:50:00Z",
                           "lw_switchGroupMode": { "group": 1, "value": "on" } });
        assert!(matches!(
            t.triage(&old2, |_| false, now(), true),
            Triage::Offline { .. }
        ));
    }

    #[test]
    fn modbus_write_values_must_be_integers() {
        let w = |v: serde_json::Value| {
            parse_visible(
                "lw_modbusWrite",
                &json!({"slaveAddress": 240, "registerAddress": 1, "value": v, "type": "register"}),
            )
        };
        assert!(w(json!(22)).is_ok());
        assert!(w(json!(22.0)).is_ok());
        assert!(w(json!(22.5)).is_err());
        assert!(w(json!("22.5")).is_err());
    }

    #[test]
    fn parses_visible_ops_like_the_firmware() {
        assert_eq!(
            parse_visible(
                "lw_switchGroupMode",
                &json!({"group": 4, "value": "on", "by": "x"})
            ),
            Ok(VisibleOp::GroupMode {
                group: 4,
                mode: GroupMode::On
            })
        );
        assert!(parse_visible("lw_switchGroupMode", &json!({"group": 4, "value": "On"})).is_err());
        assert!(parse_visible("lw_switchGroupMode", &json!({"group": 17, "value": "on"})).is_err());
        assert_eq!(
            parse_visible(
                "lw_switchConverterState",
                &json!({"converter": 3, "value": "false"})
            ),
            Ok(VisibleOp::ConverterState {
                converter: 3,
                on: false
            })
        );
        assert_eq!(
            parse_visible(
                "lw_modbusWrite",
                &json!({"slaveAddress": 240, "registerAddress": 40001, "value": 21, "type": "register"})
            ),
            Ok(VisibleOp::ModbusWrite {
                slave_address: 240,
                register_address: 40001,
                value: 21.0,
                coil: false
            })
        );
        assert_eq!(
            parse_visible(
                "lw_modbusWrite",
                &json!({"slaveAddress": 240, "registerAddress": 1, "value": 2, "type": "coil"})
            ),
            Err("Coil value must be 0 or 1".into())
        );
        assert!(
            parse_visible(
                "lw_modbusWrite",
                &json!({"slaveAddress": 0, "registerAddress": 1, "value": 1, "type": "coil"})
            )
            .is_err()
        );
    }
}

// ───────────────────────────── session integration ─────────────────────────────

mod engine {
    use tokio::time::Instant;

    use super::*;
    use crate::error::{AppError, AppResult};
    use crate::events::{OperationPromptEvent, OperationResolvedEvent};
    use crate::model::{
        OperationDecision, OperationPrompt, OperationResolution, OperationResolved,
    };
    use crate::sim::core::{GatedFinal, Purpose, SessionCore, Source};
    use crate::sim::emit::SessionEvent;
    use crate::sim::outbound::{Lane, OutboundPayload};

    impl SessionCore {
        /// Queue a status. EXECUTING goes out at once (priority lane); a final status waits
        /// for the answer to EXECUTING (else a slow EXECUTING could land after FAILED and
        /// leave the operation hanging).
        pub(in crate::sim) fn op_status(
            &mut self,
            id: &str,
            status: OperationStatus,
            reason: Option<String>,
        ) {
            if status == OperationStatus::Executing {
                self.exec_inflight.insert(id.to_owned());
                self.push_on(
                    OutboundPayload::OperationStatus {
                        operation_id: id.to_owned(),
                        status,
                        failure_reason: None,
                    },
                    Purpose::OperationStatus {
                        operation_id: id.to_owned(),
                        executing: true,
                    },
                    Lane::Priority,
                );
            } else if self.exec_inflight.contains(id) {
                self.gated
                    .insert(id.to_owned(), GatedFinal { status, reason });
            } else {
                self.op_status_now(id, status, reason);
            }
        }

        /// Queue a final status now: on the normal lane, behind the operation's effects (the
        /// readback, the group event). At stop it takes the concurrent lane.
        pub(in crate::sim) fn op_status_now(
            &mut self,
            id: &str,
            status: OperationStatus,
            reason: Option<String>,
        ) {
            let lane = if self.status == crate::model::SessionStatus::Stopping {
                Lane::Priority
            } else {
                Lane::Normal
            };
            self.push_on(
                OutboundPayload::OperationStatus {
                    operation_id: id.to_owned(),
                    status,
                    failure_reason: reason,
                },
                Purpose::OperationStatus {
                    operation_id: id.to_owned(),
                    executing: false,
                },
                lane,
            );
        }

        /// Final status + `operation-resolved`.
        fn resolve_op(
            &mut self,
            id: &str,
            fragment: &str,
            result: Result<(), String>,
            resolution: OperationResolution,
        ) {
            let (status, reason) = final_status(&result);
            self.op_status(id, status, reason.clone());
            let revision = self.bump();
            self.events
                .push(SessionEvent::Resolved(OperationResolvedEvent(
                    OperationResolved {
                        revision,
                        device_id: self.device_id.clone(),
                        operation_id: id.to_owned(),
                        operation_type: fragment.to_owned(),
                        status,
                        resolution,
                        reason,
                    },
                )));
        }

        /// Check a visible op against the device (group exists, register is writable, slave
        /// answers) and return the matching item for Modbus writes.
        fn check_visible(&self, op: &VisibleOp) -> Result<Option<usize>, String> {
            match op {
                VisibleOp::GroupMode { group, .. } => {
                    if self.groups.iter().any(|(g, _)| g.id == *group) {
                        Ok(None)
                    } else {
                        Err("The specified lw_switchGroupMode.group is incorrect".into())
                    }
                }
                VisibleOp::ConverterState { converter, .. } => {
                    if self.lamps.contains_key(converter) {
                        Ok(None)
                    } else {
                        Err("The specified lw_switchConverterState.converter is incorrect".into())
                    }
                }
                VisibleOp::ModbusWrite {
                    slave_address,
                    register_address,
                    coil,
                    ..
                } => {
                    if !self.rs485_active {
                        return Err("RS485 interface inactive".into());
                    }
                    if self.slave_address_timed_out(*slave_address) {
                        return Err("Modbus write failed: Timeout - slave not responding".into());
                    }
                    let want = if *coil { "coil" } else { "register" };
                    for (def, a) in &self.slaves {
                        if a.address != *slave_address {
                            continue;
                        }
                        for r in &a.registers {
                            if r.address == *register_address
                                && r.register_type.write_operation_type() == Some(want)
                                && let Some(idx) = self.index_of(&r.item_id)
                                && matches!(
                                    self.items[idx].def,
                                    crate::model::LibraryItem::Modbus { writable: true, .. }
                                )
                            {
                                let _ = def;
                                return Ok(Some(idx));
                            }
                        }
                    }
                    Err(format!(
                        "Modbus write failed: no writable {want} {register_address} on slave {slave_address}"
                    ))
                }
            }
        }

        /// Apply a confirmed visible operation.
        fn apply_visible(&mut self, op: &VisibleOp) -> Result<(), String> {
            let item = self.check_visible(op)?;
            match op {
                // The plugin reads the mode from the newest event: always report it.
                VisibleOp::GroupMode { group, mode } => self
                    .set_group_mode(*group, *mode, SessionCore::group_reason(), true)
                    .map_err(|e| e.to_string()),
                VisibleOp::ConverterState { converter, on } => self
                    .switch_converter(*converter, *on)
                    .map_err(|e| e.to_string()),
                VisibleOp::ModbusWrite { value, .. } => {
                    let idx = item.ok_or("Modbus write failed")?;
                    // The readback echoes exactly the operation's value (plugin compares `===`);
                    // the control range is the plugin's business, not checked here.
                    self.set_value(idx, *value, Source::Operation);
                    Ok(())
                }
            }
        }

        /// A realtime operation; `initial`: from the PENDING list fetched on (re)connect.
        pub fn on_operation(&mut self, op: &serde_json::Value, now: Instant, initial: bool) {
            let open: Vec<String> = self
                .prompts
                .iter()
                .map(|p| p.prompt.operation_id.clone())
                .collect();
            let server_now = self.server_now();
            match self
                .ops
                .triage(op, |id| open.iter().any(|o| o == id), server_now, initial)
            {
                Triage::Ignore => {}
                Triage::Withdraw { id, status, reason } => {
                    let fragment = self
                        .prompts
                        .iter()
                        .find(|p| p.prompt.operation_id == id)
                        .map(|p| p.fragment.clone())
                        .unwrap_or_default();
                    self.prompts.retain(|p| p.prompt.operation_id != id);
                    let failed = status == "FAILED";
                    let seen = if failed {
                        OperationStatus::Failed
                    } else {
                        OperationStatus::Successful
                    };
                    let revision = self.bump();
                    self.events
                        .push(SessionEvent::Resolved(OperationResolvedEvent(
                            OperationResolved {
                                revision,
                                device_id: self.device_id.clone(),
                                operation_id: id.clone(),
                                operation_type: fragment,
                                status: seen,
                                resolution: OperationResolution::Withdrawn,
                                reason: reason.clone(),
                            },
                        )));
                    if failed {
                        // The plugin cancelled it (10 s PENDING timeout). Our EXECUTING may
                        // have landed after that and left the operation EXECUTING: say FAILED
                        // again, behind EXECUTING (idempotent when it already is FAILED).
                        self.op_status(
                            &id,
                            OperationStatus::Failed,
                            Some(reason.unwrap_or_else(|| "Cancelled".into())),
                        );
                    }
                    self.emit_changed = true;
                }
                Triage::Offline { id, fragment } => self.resolve_op(
                    &id,
                    &fragment,
                    Err(REASON_OFFLINE.into()),
                    OperationResolution::SimulatorOffline,
                ),
                Triage::Unsupported { id, fragment } => self.resolve_op(
                    &id,
                    &fragment,
                    Err(REASON_UNKNOWN.into()),
                    OperationResolution::Error,
                ),
                Triage::Auto { id, fragment } => {
                    if fragment == "c8y_MeasurementRequestOperation" {
                        self.send_all_now();
                    }
                    self.resolve_op(&id, &fragment, Ok(()), OperationResolution::AutoAnswered);
                }
                Triage::Visible {
                    id,
                    fragment,
                    by,
                    parsed,
                } => {
                    self.op_status(&id, OperationStatus::Executing, None);
                    let op = match parsed.and_then(|op| self.check_visible(&op).map(|_| op)) {
                        Ok(op) => op,
                        Err(reason) => {
                            self.resolve_op(
                                &id,
                                &fragment,
                                Err(reason),
                                OperationResolution::Error,
                            );
                            return;
                        }
                    };
                    if self.auto_accept {
                        let res = self.apply_visible(&op);
                        let resolution = if res.is_ok() {
                            OperationResolution::AutoAccepted
                        } else {
                            OperationResolution::Error
                        };
                        self.resolve_op(&id, &fragment, res, resolution);
                        return;
                    }
                    let title = match &op {
                        VisibleOp::ModbusWrite { .. } => self
                            .check_visible(&op)
                            .ok()
                            .flatten()
                            .map(|i| self.items[i].def.title().clone()),
                        _ => None,
                    };
                    let received = chrono::Utc::now();
                    let expires_wall =
                        received + chrono::Duration::from_std(PROMPT_TIMEOUT).unwrap_or_default();
                    let iso = |t: chrono::DateTime<chrono::Utc>| {
                        t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
                    };
                    let revision = self.bump();
                    let prompt = OperationPrompt {
                        revision,
                        device_id: self.device_id.clone(),
                        operation_id: id,
                        kind: op.kind(),
                        summary: summary(&op, title.as_ref().map(|t| t.de.as_str())),
                        detail: detail(&op, title.as_ref()),
                        by,
                        received_at: iso(received),
                        expires_at: iso(received
                            + chrono::Duration::from_std(PROMPT_TIMEOUT).unwrap_or_default()),
                    };
                    self.events
                        .push(SessionEvent::Prompt(OperationPromptEvent(prompt.clone())));
                    self.prompts.push(OpenPrompt {
                        prompt,
                        fragment,
                        op,
                        deadline: now + PROMPT_TIMEOUT,
                        expires_wall,
                    });
                    self.emit_changed = true;
                }
            }
        }

        /// Confirm / Abort from the presenter.
        pub fn answer_operation(
            &mut self,
            operation_id: &str,
            decision: &OperationDecision,
        ) -> AppResult<()> {
            let pos = self
                .prompts
                .iter()
                .position(|p| p.prompt.operation_id == operation_id)
                .ok_or_else(|| AppError::NotFound(format!("operation {operation_id}")))?;
            let p = self.prompts.remove(pos);
            match decision {
                OperationDecision::Confirm => {
                    let res = self.apply_visible(&p.op);
                    let resolution = if res.is_ok() {
                        OperationResolution::Confirmed
                    } else {
                        OperationResolution::Error
                    };
                    self.resolve_op(operation_id, &p.fragment, res, resolution);
                }
                OperationDecision::Abort { reason } => {
                    let reason = reason
                        .as_deref()
                        .map(str::trim)
                        .filter(|r| !r.is_empty())
                        .unwrap_or(REASON_ABORTED)
                        .to_owned();
                    self.resolve_op(
                        operation_id,
                        &p.fragment,
                        Err(reason),
                        OperationResolution::Aborted,
                    );
                }
            }
            self.emit_changed = true;
            Ok(())
        }

        /// Prompts past their deadline → FAILED "Timed out".
        pub(in crate::sim) fn expire_prompts(&mut self, now: Instant) {
            // The monotonic deadline stops while a laptop sleeps: the wall clock counts too.
            let wall = chrono::Utc::now();
            let (expired, open): (Vec<OpenPrompt>, Vec<OpenPrompt>) =
                std::mem::take(&mut self.prompts)
                    .into_iter()
                    .partition(|p| p.deadline <= now || p.expires_wall <= wall);
            self.prompts = open;
            for p in expired {
                self.resolve_op(
                    &p.prompt.operation_id,
                    &p.fragment,
                    Err(REASON_TIMED_OUT.into()),
                    OperationResolution::TimedOut,
                );
                self.emit_changed = true;
            }
        }

        /// Stop: open prompts → FAILED "Simulator stopped" (the plugin has no timeout once an
        /// operation is EXECUTING).
        pub fn fail_open_prompts(&mut self) {
            for p in std::mem::take(&mut self.prompts) {
                self.resolve_op(
                    &p.prompt.operation_id,
                    &p.fragment,
                    Err(REASON_STOPPED.into()),
                    OperationResolution::Stopped,
                );
            }
        }
    }
}
