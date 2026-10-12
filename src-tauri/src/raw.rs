//! Raw fire panel backend (PRD §10): POST arbitrary measurements / alarms / events
//! to the selected demo device, keep a recent history for "resend".

use std::collections::VecDeque;

use reqwest::Method;
use serde_json::{Map, Value, json};

use crate::c8y::{C8yClient, id_of, now_iso};
use crate::error::{AppError, AppResult};
use crate::model::{FireRequest, FiredRecord, RawTarget, Severity};

/// History length.
pub const HISTORY_CAPACITY: usize = 50;

/// In-memory raw-fire history, newest last. Held in `AppState::raw_history`.
#[derive(Debug, Default)]
pub struct RawHistory {
    next_id: u32,
    entries: VecDeque<FiredRecord>,
}

impl RawHistory {
    /// Store a record, assigning its id.
    pub fn push(&mut self, mut record: FiredRecord) -> FiredRecord {
        self.next_id = self.next_id.wrapping_add(1);
        record.id = self.next_id;
        if self.entries.len() >= HISTORY_CAPACITY {
            self.entries.pop_front();
        }
        self.entries.push_back(record.clone());
        record
    }

    pub fn entries(&self) -> Vec<FiredRecord> {
        self.entries.iter().cloned().collect()
    }

    pub fn get(&self, id: u32) -> Option<&FiredRecord> {
        self.entries.iter().find(|r| r.id == id)
    }
}

/// Collection path for a raw target.
pub fn target_path(target: RawTarget) -> &'static str {
    match target {
        RawTarget::Measurement => "/measurement/measurements",
        RawTarget::Alarm => "/alarm/alarms",
        RawTarget::Event => "/event/events",
    }
}

fn severity_str(s: Severity) -> AppResult<String> {
    match serde_json::to_value(s)? {
        Value::String(v) => Ok(v),
        _ => Err(AppError::Internal("severity".into())),
    }
}

fn parse_object(text: &str, what: &str) -> AppResult<Map<String, Value>> {
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(o)) => Ok(o),
        Ok(_) => Err(AppError::InvalidInput(format!("{what}: not a JSON object"))),
        Err(e) => Err(AppError::InvalidInput(format!("{what}: {e}"))),
    }
}

/// Build path + body for `request` (pure). Always sets `source.id` (raw JSON: only if
/// missing) and `time` = `now` (raw JSON: only if missing; in a `measurements` collection,
/// per element).
pub fn build(
    device_id: &str,
    request: &FireRequest,
    now: &str,
) -> AppResult<(&'static str, Value)> {
    if device_id.trim().is_empty() {
        return Err(AppError::InvalidInput("device id".into()));
    }
    let source = json!({ "id": device_id });
    match request {
        FireRequest::Measurement(m) => {
            if m.measurement_type.trim().is_empty()
                || m.fragment.trim().is_empty()
                || m.series.trim().is_empty()
            {
                return Err(AppError::InvalidInput(
                    "measurement type, fragment and series are required".into(),
                ));
            }
            if !m.value.is_finite() {
                return Err(AppError::InvalidInput("measurement value".into()));
            }
            let mut series = json!({ "value": m.value });
            if let Some(unit) = m.unit.as_deref().filter(|u| !u.is_empty()) {
                series["unit"] = json!(unit);
            }
            let mut body = json!({ "type": m.measurement_type, "time": now, "source": source });
            body[m.fragment.as_str()] = json!({ m.series.as_str(): series });
            Ok((target_path(RawTarget::Measurement), body))
        }
        FireRequest::Alarm(a) => {
            if a.alarm_type.trim().is_empty() {
                return Err(AppError::InvalidInput("alarm type is required".into()));
            }
            let body = json!({
                "type": a.alarm_type,
                "text": a.text,
                "severity": severity_str(a.severity)?,
                "status": "ACTIVE",
                "time": now,
                "source": source,
            });
            Ok((target_path(RawTarget::Alarm), body))
        }
        FireRequest::Event(e) => {
            if e.event_type.trim().is_empty() {
                return Err(AppError::InvalidInput("event type is required".into()));
            }
            let mut body = Map::new();
            if let Some(extra) = e.extra_json.as_deref().filter(|x| !x.trim().is_empty()) {
                body = parse_object(extra, "extra JSON")?;
            }
            // Set after merging: the extras must not override the checked fields.
            body.insert("type".into(), json!(e.event_type));
            body.insert("text".into(), json!(e.text));
            body.insert("time".into(), json!(now));
            body.insert("source".into(), source);
            Ok((target_path(RawTarget::Event), Value::Object(body)))
        }
        FireRequest::Json(j) => {
            let mut body = parse_object(&j.body, "JSON body")?;
            let fill = |o: &mut Map<String, Value>| {
                o.entry("source").or_insert_with(|| source.clone());
                o.entry("time").or_insert_with(|| json!(now));
            };
            let is_collection =
                j.target == RawTarget::Measurement && body.contains_key("measurements");
            if is_collection {
                let Some(Value::Array(items)) = body.get_mut("measurements") else {
                    return Err(AppError::InvalidInput(
                        "measurements must be an array".into(),
                    ));
                };
                for item in items {
                    match item.as_object_mut() {
                        Some(o) => fill(o),
                        None => {
                            return Err(AppError::InvalidInput(
                                "measurements[]: not a JSON object".into(),
                            ));
                        }
                    }
                }
            } else {
                fill(&mut body);
            }
            Ok((target_path(j.target), Value::Object(body)))
        }
    }
}

/// Build the body for `request` and POST it (headers by `c8y::media`).
/// Returns the record (id 0 — the caller stores it in [`RawHistory`]). Non-2xx is an error
/// (the request still appears in the activity log).
pub async fn fire(
    client: &C8yClient,
    device_id: &str,
    request: &FireRequest,
) -> AppResult<FiredRecord> {
    let now = now_iso();
    let (path, body) = build(device_id, request, &now)?;
    let resp = client.request_json(Method::POST, path, Some(body)).await?;
    let created_id = id_of(&resp.body).ok().or_else(|| {
        resp.body
            .pointer("/measurements/0")
            .and_then(|m| id_of(m).ok())
    });
    Ok(FiredRecord {
        id: 0,
        device_id: device_id.to_owned(),
        fired_at: now,
        request: request.clone(),
        status: resp.status,
        created_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::c8y::fake::FakeC8y;
    use crate::model::{RawAlarm, RawEvent, RawJsonRequest, RawMeasurement};

    const NOW: &str = "2026-10-12T12:00:00.000Z";

    fn meas() -> FireRequest {
        FireRequest::Measurement(RawMeasurement {
            measurement_type: "modbus".into(),
            fragment: "LW_Modbus_Slave_20_240".into(),
            series: "freezer_temp".into(),
            value: -18.5,
            unit: Some("°C".into()),
        })
    }

    #[test]
    fn builds_measurement() {
        let (path, body) = build("42", &meas(), NOW).unwrap();
        assert_eq!(path, "/measurement/measurements");
        assert_eq!(
            body,
            json!({ "type": "modbus", "time": NOW, "source": { "id": "42" },
                    "LW_Modbus_Slave_20_240": { "freezer_temp": { "value": -18.5, "unit": "°C" } } })
        );
        let FireRequest::Measurement(mut m) = meas() else {
            unreachable!()
        };
        m.unit = Some(String::new());
        let (_, body) = build("42", &FireRequest::Measurement(m), NOW).unwrap();
        assert!(
            body["LW_Modbus_Slave_20_240"]["freezer_temp"]
                .get("unit")
                .is_none()
        );
    }

    #[test]
    fn builds_alarm_and_event() {
        let (path, body) = build(
            "42",
            &FireRequest::Alarm(RawAlarm {
                alarm_type: "lw_demo_x".into(),
                severity: Severity::Major,
                text: "boom".into(),
            }),
            NOW,
        )
        .unwrap();
        assert_eq!(path, "/alarm/alarms");
        assert_eq!(body["severity"], "MAJOR");
        assert_eq!(body["time"], NOW);
        assert_eq!(body["status"], "ACTIVE");

        let (path, body) = build(
            "42",
            &FireRequest::Event(RawEvent {
                event_type: "lw_alarm".into(),
                text: "t".into(),
                extra_json: Some(r#"{"source":{"id":"evil"},"x":{"y":1}}"#.into()),
            }),
            NOW,
        )
        .unwrap();
        assert_eq!(path, "/event/events");
        assert_eq!(body["source"]["id"], "42");
        assert_eq!(body["x"]["y"], 1);

        // Extras cannot override type / text / time.
        let (_, body) = build(
            "42",
            &FireRequest::Event(RawEvent {
                event_type: "real".into(),
                text: "t".into(),
                extra_json: Some(r#"{"type":"","text":"x","time":"T"}"#.into()),
            }),
            NOW,
        )
        .unwrap();
        assert_eq!(
            (
                body["type"].as_str(),
                body["text"].as_str(),
                body["time"].as_str()
            ),
            (Some("real"), Some("t"), Some(NOW))
        );

        let bad = FireRequest::Event(RawEvent {
            event_type: "t".into(),
            text: "t".into(),
            extra_json: Some("[1]".into()),
        });
        assert!(matches!(
            build("42", &bad, NOW),
            Err(AppError::InvalidInput(_))
        ));
    }

    #[test]
    fn raw_json_fills_missing_source_and_time() {
        let req = FireRequest::Json(RawJsonRequest {
            target: RawTarget::Measurement,
            body: r#"{"measurements":[{"type":"a"},{"type":"b","time":"T","source":{"id":"7"}}]}"#
                .into(),
        });
        let (_, body) = build("42", &req, NOW).unwrap();
        assert_eq!(body["measurements"][0]["source"]["id"], "42");
        assert_eq!(body["measurements"][0]["time"], NOW);
        assert_eq!(body["measurements"][1]["source"]["id"], "7");
        assert_eq!(body["measurements"][1]["time"], "T");

        let req = FireRequest::Json(RawJsonRequest {
            target: RawTarget::Measurement,
            body: r#"{"measurements":{"type":"a"}}"#.into(),
        });
        assert!(matches!(
            build("42", &req, NOW),
            Err(AppError::InvalidInput(m)) if m.contains("array")
        ));

        let req = FireRequest::Json(RawJsonRequest {
            target: RawTarget::Alarm,
            body: "not json".into(),
        });
        assert!(matches!(
            build("42", &req, NOW),
            Err(AppError::InvalidInput(_))
        ));
    }

    #[tokio::test]
    async fn fires_against_fake() {
        let fake = FakeC8y::start().await;
        let client = fake.client();
        let rec = fire(&client, "42", &meas()).await.unwrap();
        assert_eq!(rec.status, 201);
        assert!(rec.created_id.is_some());
        let req = fake.requests().pop().unwrap();
        assert_eq!(
            req.content_type.as_deref(),
            Some("application/vnd.com.nsn.cumulocity.measurement+json")
        );
        assert_eq!(fake.state().measurements.len(), 1);

        let coll = FireRequest::Json(RawJsonRequest {
            target: RawTarget::Measurement,
            body: r#"{"measurements":[{"type":"a","f":{"s":{"value":1}}}]}"#.into(),
        });
        let rec = fire(&client, "42", &coll).await.unwrap();
        assert!(rec.created_id.is_some());
        let req = fake.requests().pop().unwrap();
        assert_eq!(
            req.content_type.as_deref(),
            Some("application/vnd.com.nsn.cumulocity.measurementcollection+json")
        );

        let mut history = RawHistory::default();
        let stored = history.push(rec);
        assert_eq!(stored.id, 1);
        assert_eq!(history.get(1).unwrap().device_id, "42");
    }
}
