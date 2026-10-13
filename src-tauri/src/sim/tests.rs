//! Session engine tests.
//!
//! - `paused::*`: the real task loop with paused tokio time, a channel instead of realtime and
//!   the outbound test double (no HTTP) — operation state machine incl. 60 s timeouts,
//!   scheduler, faults, scenarios, group modes, counters, revisions.
//! - `fake::*`: end to end against `c8y::fake::FakeC8y` (real client, realtime, outbound).

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::core::SessionParams;
use super::emit::{RecordingEmitter, SessionEmitter, SessionEvent};
use super::outbound::{self, OutboundItem, OutboundPayload, OutboundResult, TestOut};
use super::session::{Prepared, SessionCommand, SessionHandle, spawn_with};
use crate::c8y::realtime::{RealtimeMsg, RealtimeSubscription};
use crate::device::apply;
use crate::library::{Library, loader};
use crate::model::{
    FaultTarget, GroupMode, OperationDecision, OperationResolution, OperationStatus,
};

pub(crate) fn fixture() -> Library {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/library");
    loader::load(&root, "1.0.0").unwrap()
}

fn prepared(lib: &Library, auto_accept: bool, realistic: bool) -> Prepared {
    let demo = lib.demo("fixture").unwrap();
    let mut plan = apply::plan(lib, demo, None).unwrap();
    plan.state.realistic_mode = realistic;
    Prepared {
        params: SessionParams {
            device_id: "100".into(),
            device_name: "Demo".into(),
            user: "demo".into(),
            demo: plan.state,
            resolved: plan.resolved,
            groups: demo.dali_groups.clone(),
            inputs: plan.inputs,
            scenarios: demo.scenarios.clone(),
            auto_accept,
            counter_seeds: Default::default(),
            time_zone: "Europe/Berlin".into(),
            seed: 42,
            ..Default::default()
        },
    }
}

struct Rig {
    handle: SessionHandle,
    rt: mpsc::Sender<RealtimeMsg>,
    out: TestOut,
    results: mpsc::UnboundedSender<OutboundResult>,
    emitter: Arc<RecordingEmitter>,
    queue: outbound::OutboundQueue,
}

fn rig_with(p: Prepared) -> Rig {
    rig_with_emitter(p, RecordingEmitter::new())
}

fn rig_with_emitter(p: Prepared, emitter: Arc<RecordingEmitter>) -> Rig {
    rig_with_sink(p, emitter.clone(), emitter)
}

/// `sink` is what the session emits to (it may misbehave); `emitter` is what the test reads.
fn rig_with_sink(
    p: Prepared,
    sink: Arc<dyn SessionEmitter>,
    emitter: Arc<RecordingEmitter>,
) -> Rig {
    let (rt_tx, rt_rx) = mpsc::channel(16);
    let task = tauri::async_runtime::JoinHandle::Tokio(tokio::spawn(async {}));
    let sub = RealtimeSubscription::from_parts(rt_rx, Default::default(), task);
    let (worker, out, results) = outbound::test_pair();
    let queue = worker.queue.clone();
    let handle = spawn_with(p, Some(sub), worker, sink);
    Rig {
        handle,
        rt: rt_tx,
        out,
        results,
        emitter,
        queue,
    }
}

fn rig(auto_accept: bool) -> Rig {
    rig_with(prepared(&fixture(), auto_accept, false))
}

impl Rig {
    /// Let the task run (paused time: a tiny sleep advances nothing noticeable).
    async fn settle(&self) {
        for _ in 0..3 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }

    fn drain(&mut self) -> Vec<OutboundItem> {
        let mut v = Vec::new();
        while let Ok(i) = self.out.try_recv() {
            v.push(i);
        }
        v
    }

    /// Answer every drained item with `Ok(body)` (alarms get ids `a<tag>`).
    fn ack(&self, items: &[OutboundItem]) {
        for i in items {
            let body = match &i.payload {
                OutboundPayload::Alarm(_) => json!({ "id": format!("a{}", i.tag) }),
                _ => json!({}),
            };
            let _ = self.results.send(OutboundResult {
                tag: i.tag,
                result: Ok(body),
            });
        }
    }

    async fn op(&self, op: Value) {
        self.rt.send(RealtimeMsg::Operation(op)).await.unwrap();
        self.settle().await;
    }

    /// An operation from the PENDING list fetched on (re)connect.
    async fn pending_op(&self, op: Value) {
        self.rt
            .send(RealtimeMsg::PendingOperation(op))
            .await
            .unwrap();
        self.settle().await;
    }

    /// Drain everything queued, answer it, and let the loop release what waited for answers
    /// (final statuses wait for their EXECUTING answer). Returns what was drained.
    async fn flush(&mut self) -> Vec<OutboundItem> {
        self.settle().await;
        let items = self.drain();
        self.ack(&items);
        self.settle().await;
        items
    }

    async fn cmd(&self, c: SessionCommand) -> crate::error::AppResult<()> {
        self.handle.sender().send(c).await
    }
}

fn now_iso() -> String {
    crate::c8y::now_iso()
}

fn statuses(items: &[OutboundItem]) -> Vec<(String, OperationStatus, Option<String>)> {
    items
        .iter()
        .filter_map(|i| match &i.payload {
            OutboundPayload::OperationStatus {
                operation_id,
                status,
                failure_reason,
            } => Some((operation_id.clone(), *status, failure_reason.clone())),
            _ => None,
        })
        .collect()
}

fn measurements(items: &[OutboundItem]) -> Vec<Value> {
    items
        .iter()
        .filter_map(|i| match &i.payload {
            OutboundPayload::Measurements(b) => {
                Some(b["measurements"].as_array().cloned().unwrap_or_default())
            }
            _ => None,
        })
        .flatten()
        .collect()
}

fn events_of(items: &[OutboundItem]) -> Vec<Value> {
    items
        .iter()
        .filter_map(|i| match &i.payload {
            OutboundPayload::Event(b) => Some(b.clone()),
            _ => None,
        })
        .collect()
}

fn resolutions(e: &RecordingEmitter) -> Vec<(String, OperationResolution)> {
    e.events()
        .into_iter()
        .filter_map(|ev| match ev {
            SessionEvent::Resolved(r) => Some((r.0.operation_id, r.0.resolution)),
            _ => None,
        })
        .collect()
}

fn has_series(ms: &[Value], fragment: &str, series: &str) -> Option<f64> {
    ms.iter()
        .rev()
        .find_map(|m| m.get(fragment)?.get(series)?.get("value")?.as_f64())
}

fn group_op(id: &str, group: u32, value: &str) -> Value {
    json!({ "id": id, "status": "PENDING", "creationTime": now_iso(), "deviceId": "100",
            "lw_switchGroupMode": { "group": group, "value": value, "by": "anna@example.com" } })
}

mod paused {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn start_sends_everything_with_native_names() {
        let mut r = rig(false);
        r.settle().await;
        let items = r.drain();
        let ms = measurements(&items);
        assert_eq!(
            has_series(&ms, "LW Dali Control Gear 1-01", "Brightness"),
            Some(80.0)
        );
        assert_eq!(
            has_series(&ms, "LW Dali Control Gear 1-01", "Lamp Status"),
            Some(1.0)
        );
        // Lamp 3 has no item (excluded) but is in a group: still sent, at its group's max.
        assert_eq!(
            has_series(&ms, "LW Dali Control Gear 1-03", "Brightness"),
            Some(90.0)
        );
        // The start scenario set the freezer to -18.
        assert_eq!(
            has_series(&ms, "LW_Modbus_Slave_20_240", "freezer_temp"),
            Some(-18.0)
        );
        assert_eq!(
            has_series(&ms, "LW_Modbus_Slave_19_239", "ac_setpoint"),
            Some(21.0)
        );
        assert_eq!(has_series(&ms, "LW Digital Input", "Value02"), Some(0.0));
        assert_eq!(
            has_series(&ms, "LW Digital Input", "ImpulseCounter03"),
            Some(1000.0)
        );
        assert!(has_series(&ms, "c8y_SignalStrength", "rssi").is_some());
        // DALI lamps: ≤ 6 per collection; every object has time.
        assert!(
            ms.iter()
                .all(|m| m["time"].is_string() && m["source"]["id"] == "100")
        );
        let snap = r.handle.snapshot();
        assert!(
            snap.items
                .iter()
                .find(|i| i.item_id == "freezer_temp")
                .unwrap()
                .generator
                .is_some()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn visible_operation_times_out_after_60s() {
        let mut r = rig(false);
        r.settle().await;
        r.drain();
        r.op(group_op("op1", 1, "on")).await;
        let items = r.flush().await;
        assert_eq!(
            statuses(&items),
            vec![("op1".into(), OperationStatus::Executing, None)]
        );
        assert_eq!(r.handle.snapshot().pending_operations.len(), 1);
        let prompt = r
            .emitter
            .events()
            .into_iter()
            .find_map(|e| match e {
                SessionEvent::Prompt(p) => Some(p.0),
                _ => None,
            })
            .unwrap();
        assert_eq!(prompt.summary, "Group 1 → On");
        // F28: the UI words the prompt itself, from structured data.
        assert_eq!(
            prompt.detail,
            crate::model::OperationDetail::GroupMode {
                group: 1,
                mode: GroupMode::On
            }
        );
        assert_eq!(prompt.by.as_deref(), Some("anna@example.com"));

        tokio::time::sleep(Duration::from_secs(59)).await;
        assert!(statuses(&r.drain()).is_empty());
        tokio::time::sleep(Duration::from_secs(2)).await;
        let items = r.drain();
        assert_eq!(
            statuses(&items),
            vec![(
                "op1".into(),
                OperationStatus::Failed,
                Some("Timed out".into())
            )]
        );
        assert!(events_of(&items).is_empty(), "no group change on timeout");
        assert!(r.handle.snapshot().pending_operations.is_empty());
        assert_eq!(
            resolutions(&r.emitter),
            vec![("op1".into(), OperationResolution::TimedOut)]
        );
        // Answering afterwards fails.
        assert!(
            r.cmd(SessionCommand::AnswerOperation {
                operation_id: "op1".into(),
                decision: OperationDecision::Confirm
            })
            .await
            .is_err()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn confirm_applies_group_mode_and_sends_event() {
        let mut r = rig(false);
        r.settle().await;
        r.drain();
        r.op(group_op("op2", 1, "off")).await;
        r.flush().await;
        r.cmd(SessionCommand::AnswerOperation {
            operation_id: "op2".into(),
            decision: OperationDecision::Confirm,
        })
        .await
        .unwrap();
        r.settle().await;
        let items = r.drain();
        let ms = measurements(&items);
        assert_eq!(
            has_series(&ms, "LW Dali Control Gear 1-01", "Brightness"),
            Some(0.0)
        );
        assert_eq!(
            has_series(&ms, "LW Dali Control Gear 1-02", "Brightness"),
            Some(0.0)
        );
        let ev = events_of(&items);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0]["type"], "lw_daliGroupChange");
        assert_eq!(ev[0]["groupModeChange"]["newMode"], "Off");
        assert_eq!(ev[0]["groupModeChange"]["previousMode"], "Auto");
        assert_eq!(
            statuses(&items),
            vec![("op2".into(), OperationStatus::Successful, None)]
        );
        let snap = r.handle.snapshot();
        assert_eq!(snap.groups[0].mode, GroupMode::Off);

        // Same mode again (locally): no event.
        r.cmd(SessionCommand::SetGroupMode {
            group_id: 1,
            mode: GroupMode::Off,
        })
        .await
        .unwrap();
        r.settle().await;
        assert!(events_of(&r.drain()).is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn abort_uses_default_reason_and_dedups() {
        let mut r = rig(false);
        r.settle().await;
        r.drain();
        let op = group_op("op3", 2, "on");
        r.op(op.clone()).await;
        r.op(op).await; // duplicate delivery
        let items = r.flush().await;
        assert_eq!(statuses(&items).len(), 1, "one EXECUTING only");
        r.cmd(SessionCommand::AnswerOperation {
            operation_id: "op3".into(),
            decision: OperationDecision::Abort { reason: None },
        })
        .await
        .unwrap();
        r.settle().await;
        assert_eq!(
            statuses(&r.drain()),
            vec![(
                "op3".into(),
                OperationStatus::Failed,
                Some("Aborted by user".into())
            )]
        );
    }

    /// F19: the presenter's abort reason becomes the `failureReason`.
    #[tokio::test(start_paused = true)]
    async fn abort_reason_is_sent_as_failure_reason() {
        let mut r = rig(false);
        r.settle().await;
        r.op(group_op("op4", 2, "on")).await;
        r.flush().await;
        r.cmd(SessionCommand::AnswerOperation {
            operation_id: "op4".into(),
            decision: OperationDecision::Abort {
                reason: Some("  Kunde wartet ".into()),
            },
        })
        .await
        .unwrap();
        r.settle().await;
        assert_eq!(
            statuses(&r.drain()),
            vec![(
                "op4".into(),
                OperationStatus::Failed,
                Some("Kunde wartet".into())
            )]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn modbus_write_readback_echoes_exact_value() {
        let mut r = rig(true); // auto-accept
        r.settle().await;
        r.drain();
        r.op(
            json!({ "id": "op4", "status": "PENDING", "creationTime": now_iso(),
                     "lw_modbusWrite": { "slaveAddress": 239, "registerAddress": 40002,
                                         "value": 23, "type": "register", "by": "x" } }),
        )
        .await;
        let items = r.flush().await;
        let ms = measurements(&items);
        assert_eq!(ms.len(), 1);
        assert_eq!(
            ms[0]["LW_Modbus_Slave_19_239"]["ac_setpoint"]["value"],
            json!(23)
        );
        // EXECUTING went out at once; SUCCESSFUL waited for its answer and follows the readback.
        assert_eq!(
            statuses(&items),
            vec![("op4".into(), OperationStatus::Executing, None)]
        );
        let items = r.drain();
        assert_eq!(
            statuses(&items),
            vec![("op4".into(), OperationStatus::Successful, None)]
        );
        r.ack(&items);
        assert_eq!(
            resolutions(&r.emitter),
            vec![("op4".into(), OperationResolution::AutoAccepted)]
        );
        // Wrong register type → FAILED, nothing applied.
        r.op(
            json!({ "id": "op5", "status": "PENDING", "creationTime": now_iso(),
                     "lw_modbusWrite": { "slaveAddress": 239, "registerAddress": 40002,
                                         "value": 1, "type": "coil" } }),
        )
        .await;
        let mut items = r.flush().await;
        items.extend(r.drain());
        assert!(measurements(&items).is_empty());
        assert_eq!(statuses(&items)[1].1, OperationStatus::Failed);
    }

    #[tokio::test(start_paused = true)]
    async fn multi_effect_item_sends_event_and_inventory() {
        let mut r = rig(true);
        r.settle().await;
        r.drain();
        r.op(
            json!({ "id": "op6", "status": "PENDING", "creationTime": now_iso(),
                     "lw_modbusWrite": { "slaveAddress": 239, "registerAddress": 40001,
                                         "value": 2, "type": "register" } }),
        )
        .await;
        let items = r.drain();
        let ev = events_of(&items);
        assert_eq!(ev[0]["type"], "lw_demo_acMode");
        assert_eq!(ev[0]["text"], "Klima Modus: Heizen");
        assert!(items.iter().any(|i| matches!(&i.payload,
            OutboundPayload::InventoryPatch(b) if b["lw_demoAcMode"] == "Heizen")));
    }

    #[tokio::test(start_paused = true)]
    async fn config_ops_auto_answer_stale_offline_failed_ignored() {
        let mut r = rig(false);
        r.settle().await;
        r.drain();
        r.op(
            json!({ "id": "c1", "status": "PENDING", "creationTime": now_iso(),
                     "c8y_MeasurementRequestOperation": { "requestName": "LOG" } }),
        )
        .await;
        let items = r.drain();
        assert_eq!(
            statuses(&items),
            vec![("c1".into(), OperationStatus::Successful, None)]
        );
        assert!(
            measurements(&items).len() > 5,
            "measurement request sends all values"
        );

        let old = (chrono::Utc::now() - chrono::Duration::seconds(120))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        r.pending_op(
            json!({ "id": "c2", "status": "PENDING", "creationTime": old,
                     "lw_switchGroupMode": { "group": 1, "value": "on" } }),
        )
        .await;
        r.op(
            json!({ "id": "c3", "status": "FAILED", "creationTime": now_iso(),
                     "lw_switchGroupMode": { "group": 1, "value": "on" } }),
        )
        .await;
        r.op(
            json!({ "id": "c4", "status": "PENDING", "creationTime": now_iso(),
                     "c8y_Command": { "text": "x" } }),
        )
        .await;
        assert_eq!(
            statuses(&r.drain()),
            vec![
                (
                    "c2".into(),
                    OperationStatus::Failed,
                    Some("Simulator was offline".into())
                ),
                (
                    "c4".into(),
                    OperationStatus::Failed,
                    Some("Operation command not found or timeout".into())
                ),
            ]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn prompt_withdrawn_when_plugin_fails_it() {
        let mut r = rig(false);
        r.settle().await;
        r.op(group_op("w1", 1, "on")).await;
        assert_eq!(r.handle.snapshot().pending_operations.len(), 1);
        let mut failed = group_op("w1", 1, "on");
        failed["status"] = json!("FAILED");
        r.op(failed).await;
        assert!(r.handle.snapshot().pending_operations.is_empty());
        r.drain();
        tokio::time::sleep(Duration::from_secs(61)).await;
        assert!(statuses(&r.drain()).is_empty(), "no late Timed out");
    }

    #[tokio::test(start_paused = true)]
    async fn periodic_send_and_session_changed_after_it() {
        let mut r = rig(false);
        r.settle().await;
        let first = r.drain();
        r.ack(&first);
        r.settle().await;
        let before = r.handle.snapshot().revision;
        tokio::time::sleep(Duration::from_secs(25)).await;
        // Generator ticks only update the UI.
        assert!(measurements(&r.drain()).is_empty());
        tokio::time::sleep(Duration::from_secs(6)).await;
        let items = r.drain();
        assert!(measurements(&items).len() > 5, "periodic full send at 30 s");
        let n_changed = |e: &RecordingEmitter| {
            e.events()
                .iter()
                .filter(|e| matches!(e, SessionEvent::Changed(Some(_))))
                .count()
        };
        let changed_before = n_changed(&r.emitter);
        r.ack(&items);
        r.settle().await;
        assert!(
            n_changed(&r.emitter) > changed_before,
            "session-changed after periodic send"
        );
        let snap = r.handle.snapshot();
        assert!(snap.last_sent_at.is_some());
        assert!(snap.revision > before);

        r.cmd(SessionCommand::SetSendInterval { seconds: 900 })
            .await
            .unwrap_err();
        r.cmd(SessionCommand::SetSendInterval { seconds: 10 })
            .await
            .unwrap();
        r.drain();
        tokio::time::sleep(Duration::from_secs(11)).await;
        assert!(!measurements(&r.drain()).is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn slave_timeout_stops_measurements_and_recovers() {
        let mut r = rig(false);
        r.settle().await;
        r.drain();
        let target = FaultTarget::SlaveTimeout {
            slave_key: "kuehlung".into(),
        };
        r.cmd(SessionCommand::SetFault {
            target: target.clone(),
            active: true,
        })
        .await
        .unwrap();
        r.settle().await;
        let items = r.drain();
        let alarm = items
            .iter()
            .find_map(|i| match &i.payload {
                OutboundPayload::Alarm(a) => Some(a.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(alarm["type"], "lw_modbus_20_timeoutError");
        assert_eq!(alarm["text"], "Modbus 20-240 Timeout Error Kuehlung");
        assert_eq!(alarm["severity"], "MAJOR");
        let ev = events_of(&items);
        assert_eq!(ev[0]["type"], "lw_modbus");
        assert_eq!(
            ev[0]["text"],
            "Modbus slave 20-240 (Kuehlung): Timeout alarm: Timeout - slave not responding - registers 1-30001"
        );
        r.ack(&items);
        r.settle().await;
        assert_eq!(
            r.handle.snapshot().active_faults[0].alarm_id.as_deref(),
            Some(
                format!(
                    "a{}",
                    items
                        .iter()
                        .find(|i| matches!(i.payload, OutboundPayload::Alarm(_)))
                        .unwrap()
                        .tag
                )
                .as_str()
            )
        );

        // Periodic send skips the slave; writes to it fail.
        r.cmd(SessionCommand::SendAllNow).await.unwrap();
        r.settle().await;
        let ms = measurements(&r.drain());
        assert!(has_series(&ms, "LW_Modbus_Slave_20_240", "freezer_temp").is_none());
        assert!(has_series(&ms, "LW_Modbus_Slave_19_239", "ac_mode").is_some());
        assert!(
            r.handle
                .snapshot()
                .items
                .iter()
                .find(|i| i.item_id == "hood_fan")
                .unwrap()
                .paused
        );

        r.cmd(SessionCommand::SetFault {
            target,
            active: false,
        })
        .await
        .unwrap();
        r.settle().await;
        let items = r.drain();
        let ev = events_of(&items);
        assert_eq!(ev[0]["type"], "lw_alarm");
        assert_eq!(ev[0]["text"], "Alarm cleared: lw_modbus_20_timeoutError");
        assert!(items.iter().any(|i| matches!(&i.payload, OutboundPayload::AlarmClear { alarm_id } if alarm_id.starts_with('a'))));
        assert!(
            has_series(
                &measurements(&items),
                "LW_Modbus_Slave_20_240",
                "freezer_temp"
            )
            .is_some()
        );
        assert!(r.handle.snapshot().active_faults.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn lamp_faults_change_status_and_a_scenario_clear_clears_in_realistic_mode() {
        let mut r = rig_with(prepared(&fixture(), false, true));
        r.settle().await;
        r.drain();
        let lost = FaultTarget::DaliLamp {
            lamp: 2,
            alarm_type: "lw_converter_1_02_lost".into(),
        };
        let open = FaultTarget::DaliLamp {
            lamp: 1,
            alarm_type: "lw_lampFailure_1_01_openCircuit".into(),
        };
        for t in [&lost, &open] {
            r.cmd(SessionCommand::SetFault {
                target: t.clone(),
                active: true,
            })
            .await
            .unwrap();
        }
        r.settle().await;
        let items = r.drain();
        r.ack(&items);
        let ms = measurements(&items);
        assert_eq!(
            has_series(&ms, "LW Dali Control Gear 1-01", "Lamp Status"),
            Some(0.0)
        );
        assert!(has_series(&ms, "LW Dali Control Gear 1-02", "Brightness").is_none());
        r.cmd(SessionCommand::SetFault {
            target: open,
            active: false,
        })
        .await
        .unwrap();
        r.settle().await;
        let items = r.drain();
        assert_eq!(
            events_of(&items)[0]["text"],
            "Alarm cleared: lw_lampFailure_1_01_openCircuit"
        );
        // D9: a scenario `active: false` stands in for the presenter's clear in Cumulocity, so
        // it clears even in realistic mode (that mode only spares derived alarms).
        assert_eq!(clears(&items), 1);
        assert_eq!(
            has_series(
                &measurements(&items),
                "LW Dali Control Gear 1-01",
                "Lamp Status"
            ),
            Some(1.0)
        );
    }

    fn alarm_types(items: &[OutboundItem]) -> Vec<String> {
        items
            .iter()
            .filter_map(|i| match &i.payload {
                OutboundPayload::Alarm(b) => b["type"].as_str().map(str::to_owned),
                _ => None,
            })
            .collect()
    }

    fn clears(items: &[OutboundItem]) -> usize {
        items
            .iter()
            .filter(|i| matches!(i.payload, OutboundPayload::AlarmClear { .. }))
            .count()
    }

    fn cleared_texts(items: &[OutboundItem]) -> Vec<String> {
        events_of(items)
            .iter()
            .filter_map(|e| e["text"].as_str().map(str::to_owned))
            .filter(|t| t.starts_with("Alarm cleared: "))
            .collect()
    }

    fn bus(active: bool) -> SessionCommand {
        SessionCommand::SetFault {
            target: FaultTarget::DaliBus {
                alarm_type: "lw_daliBusPowerFail".into(),
            },
            active,
        }
    }

    /// Firmware §2.1: without bus power no gear answers. Every grouped lamp is lost
    /// (`lw_converter_1_NN_lost` + `lw_controlGearFailure_1_NN`, the floorplan turns it red) and
    /// stops sending, sensors are lost; recovery undoes it. Lamp faults of their own survive.
    #[tokio::test(start_paused = true)]
    async fn bus_power_failure_loses_every_lamp_and_recovers() {
        let mut r = rig(false);
        r.settle().await;
        r.drain();
        let open1 = FaultTarget::DaliLamp {
            lamp: 1,
            alarm_type: "lw_lampFailure_1_01_openCircuit".into(),
        };
        r.cmd(SessionCommand::SetFault {
            target: open1.clone(),
            active: true,
        })
        .await
        .unwrap();
        r.flush().await;

        r.cmd(bus(true)).await.unwrap();
        let items = r.flush().await;
        let raised = alarm_types(&items);
        assert_eq!(raised[0], "lw_daliBusPowerFail");
        for n in 1..=4 {
            assert!(
                raised.contains(&format!("lw_converter_1_{n:02}_lost")),
                "{raised:?}"
            );
            assert!(raised.contains(&format!("lw_controlGearFailure_1_{n:02}")));
        }
        assert!(raised.contains(&"lw_brightness_1_01_lost".to_owned()));
        assert!(raised.contains(&"lw_motion_1_01_lost".to_owned()));
        assert_eq!(raised.len(), 1 + 4 * 2 + 2, "{raised:?}");
        let snap = r.handle.snapshot();
        assert_eq!(snap.active_faults.len(), 2, "cascade alarms are no faults");
        assert!(
            snap.items
                .iter()
                .find(|i| i.item_id == "light-1")
                .unwrap()
                .paused
        );
        assert!(
            snap.items
                .iter()
                .find(|i| i.item_id == "lux-1")
                .unwrap()
                .paused
        );

        // Nothing on the bus is sent while it is down, also not with the next full send.
        r.cmd(SessionCommand::SendAllNow).await.unwrap();
        let ms = measurements(&r.flush().await);
        assert!(!ms.is_empty());
        for n in 1..=4 {
            let f = format!("LW Dali Control Gear 1-{n:02}");
            assert!(has_series(&ms, &f, "Brightness").is_none(), "{f}");
        }
        assert!(has_series(&ms, "LW Dali Brightness", "Lux1").is_none());

        // A lamp fault raised during the bus failure: the alarm is open already.
        let lost2 = FaultTarget::DaliLamp {
            lamp: 2,
            alarm_type: "lw_converter_1_02_lost".into(),
        };
        r.cmd(SessionCommand::SetFault {
            target: lost2.clone(),
            active: true,
        })
        .await
        .unwrap();
        let items = r.flush().await;
        assert!(alarm_types(&items).is_empty());

        // Recovery: bus first, then every lamp / sensor alarm except the one lamp 2 holds.
        r.cmd(bus(false)).await.unwrap();
        let items = r.flush().await;
        let texts = cleared_texts(&items);
        assert_eq!(texts[0], "Alarm cleared: lw_daliBusPowerFail");
        assert!(texts.contains(&"Alarm cleared: lw_converter_1_01_lost".to_owned()));
        assert!(texts.contains(&"Alarm cleared: lw_brightness_1_01_lost".to_owned()));
        // Lamp 2's own `_lost` fault still holds itself and its control gear failure.
        assert!(!texts.contains(&"Alarm cleared: lw_converter_1_02_lost".to_owned()));
        assert!(!texts.contains(&"Alarm cleared: lw_controlGearFailure_1_02".to_owned()));
        assert!(!texts.iter().any(|t| t.contains("openCircuit")));
        assert_eq!(texts.len(), 1 + 4 * 2 - 2 + 2, "{texts:?}");
        assert_eq!(clears(&items), texts.len());
        let ms = measurements(&items);
        // Lamp 1 is back with its own open-circuit fault, lamp 4 with its group level.
        assert_eq!(
            has_series(&ms, "LW Dali Control Gear 1-01", "Lamp Status"),
            Some(0.0)
        );
        assert_eq!(
            has_series(&ms, "LW Dali Control Gear 1-04", "Lamp Status"),
            Some(1.0)
        );
        assert!(has_series(&ms, "LW Dali Control Gear 1-04", "Brightness").is_some());
        assert!(has_series(&ms, "LW Dali Control Gear 1-02", "Brightness").is_none());
        assert!(has_series(&ms, "LW Dali Brightness", "Lux1").is_some());
        assert_eq!(r.handle.snapshot().active_faults.len(), 2);

        // Clearing lamp 2's own fault now recovers it.
        r.cmd(SessionCommand::SetFault {
            target: lost2,
            active: false,
        })
        .await
        .unwrap();
        let items = r.flush().await;
        assert_eq!(
            cleared_texts(&items),
            vec![
                "Alarm cleared: lw_converter_1_02_lost".to_owned(),
                "Alarm cleared: lw_controlGearFailure_1_02".to_owned()
            ]
        );
        assert!(
            has_series(
                &measurements(&items),
                "LW Dali Control Gear 1-02",
                "Brightness"
            )
            .is_some()
        );
        // The snapshot is no longer paused after the recovery.
        let snap = r.handle.snapshot();
        assert!(
            !snap
                .items
                .iter()
                .find(|i| i.item_id == "light-1")
                .unwrap()
                .paused
        );
        assert!(
            !snap
                .items
                .iter()
                .find(|i| i.item_id == "lux-1")
                .unwrap()
                .paused
        );
    }

    fn position(types: &[String], pred: impl Fn(&str) -> bool) -> Vec<usize> {
        types
            .iter()
            .enumerate()
            .filter(|(_, t)| pred(t))
            .map(|(i, _)| i)
            .collect()
    }

    fn bus_type(t: &str) -> SessionCommand {
        SessionCommand::SetFault {
            target: FaultTarget::DaliBus {
                alarm_type: t.into(),
            },
            active: true,
        }
    }

    fn bus_off_type(t: &str) -> SessionCommand {
        SessionCommand::SetFault {
            target: FaultTarget::DaliBus {
                alarm_type: t.into(),
            },
            active: false,
        }
    }

    fn fixture_groups() -> Vec<crate::model::DaliGroup> {
        fixture().demo("fixture").unwrap().dali_groups.clone()
    }

    /// All `_lost` alarms (lamps, then sensors) go out before any control gear failure, on
    /// raise and on recovery, so the floorplan lamps turn red / green first.
    #[tokio::test(start_paused = true)]
    async fn cascade_posts_lost_alarms_before_gear_failures() {
        let mut r = rig(false);
        r.settle().await;
        r.drain();
        r.cmd(bus(true)).await.unwrap();
        let items = r.flush().await;
        let raised = alarm_types(&items);
        let lost = position(&raised, |t| t.ends_with("_lost"));
        let gear = position(&raised, |t| t.starts_with("lw_controlGearFailure_"));
        assert_eq!((lost.len(), gear.len()), (6, 4), "{raised:?}");
        assert!(lost.iter().max() < gear.iter().min(), "{raised:?}");
        r.cmd(bus(false)).await.unwrap();
        let items = r.flush().await;
        let texts: Vec<String> = cleared_texts(&items)
            .into_iter()
            .map(|t| t.trim_start_matches("Alarm cleared: ").to_owned())
            .collect();
        let lost = position(&texts, |t| t.ends_with("_lost"));
        let gear = position(&texts, |t| t.starts_with("lw_controlGearFailure_"));
        assert_eq!((lost.len(), gear.len()), (6, 4), "{texts:?}");
        assert!(lost.iter().max() < gear.iter().min(), "{texts:?}");
    }

    /// Two bus power types at once: the cascade stays until the last one is cleared.
    #[tokio::test(start_paused = true)]
    async fn cascade_lasts_until_the_last_bus_power_fault_clears() {
        let mut r = rig(false);
        r.settle().await;
        r.drain();
        r.cmd(bus_type("lw_daliBusPowerFail")).await.unwrap();
        r.flush().await;
        r.cmd(bus_type("lw_daliBusPowerOff")).await.unwrap();
        let items = r.flush().await;
        assert_eq!(alarm_types(&items), vec!["lw_daliBusPowerOff".to_owned()]);
        r.cmd(bus_off_type("lw_daliBusPowerFail")).await.unwrap();
        let items = r.flush().await;
        assert_eq!(
            cleared_texts(&items),
            vec!["Alarm cleared: lw_daliBusPowerFail".to_owned()]
        );
        assert!(
            r.handle
                .snapshot()
                .items
                .iter()
                .find(|i| i.item_id == "light-1")
                .unwrap()
                .paused
        );
        r.cmd(bus_off_type("lw_daliBusPowerOff")).await.unwrap();
        let items = r.flush().await;
        assert_eq!(cleared_texts(&items).len(), 1 + 4 * 2 + 2);
    }

    /// Ungrouped lamps are not on the failed bus; lamps that join / leave a group during the
    /// failure are raised / cleared.
    #[tokio::test(start_paused = true)]
    async fn group_changes_join_and_leave_a_running_bus_failure() {
        let mut r = rig(false);
        r.settle().await;
        r.drain();
        let all = fixture_groups();
        // Group 2 (lamps 3, 4) is gone: those lamps are ungrouped.
        r.cmd(SessionCommand::ConfigChanged {
            groups: Some(all[..1].to_vec()),
            inputs: None,
        })
        .await
        .unwrap();
        r.flush().await;
        r.cmd(bus(true)).await.unwrap();
        let raised = alarm_types(&r.flush().await);
        assert!(raised.contains(&"lw_converter_1_02_lost".to_owned()));
        assert!(
            !raised
                .iter()
                .any(|t| t.contains("_03") || t.contains("_04"))
        );
        let ms = {
            r.cmd(SessionCommand::SendAllNow).await.unwrap();
            measurements(&r.flush().await)
        };
        assert!(has_series(&ms, "LW Dali Control Gear 1-02", "Brightness").is_none());

        // The group returns while the bus is down: lamps 3 and 4 are lost now.
        r.cmd(SessionCommand::ConfigChanged {
            groups: Some(all.clone()),
            inputs: None,
        })
        .await
        .unwrap();
        let raised = alarm_types(&r.flush().await);
        assert_eq!(
            raised,
            vec![
                "lw_converter_1_03_lost".to_owned(),
                "lw_converter_1_04_lost".to_owned(),
                "lw_controlGearFailure_1_03".to_owned(),
                "lw_controlGearFailure_1_04".to_owned()
            ]
        );
        // ... and leaves again: recovered.
        r.cmd(SessionCommand::ConfigChanged {
            groups: Some(all[..1].to_vec()),
            inputs: None,
        })
        .await
        .unwrap();
        let items = r.flush().await;
        assert_eq!(cleared_texts(&items).len(), 4);
        assert_eq!(clears(&items), 4);
    }

    /// A sensor's own `_lost` fault is not the bus's: it survives the recovery.
    #[tokio::test(start_paused = true)]
    async fn sensor_fault_of_its_own_survives_bus_recovery() {
        let mut r = rig(false);
        r.settle().await;
        r.drain();
        let target = FaultTarget::Sensor {
            sensor_kind: "brightness".into(),
            index: 1,
            alarm: "lw_brightness_1_01_lost".into(),
        };
        r.cmd(SessionCommand::SetFault {
            target: target.clone(),
            active: true,
        })
        .await
        .unwrap();
        r.flush().await;
        r.cmd(bus(true)).await.unwrap();
        let raised = alarm_types(&r.flush().await);
        assert!(!raised.contains(&"lw_brightness_1_01_lost".to_owned()));
        r.cmd(bus(false)).await.unwrap();
        let texts = cleared_texts(&r.flush().await);
        assert!(!texts.contains(&"Alarm cleared: lw_brightness_1_01_lost".to_owned()));
        assert!(texts.contains(&"Alarm cleared: lw_motion_1_01_lost".to_owned()));
        assert_eq!(r.handle.snapshot().active_faults.len(), 1);
    }

    /// The bus fails and recovers before Cumulocity has answered the alarm POSTs: the clears
    /// wait for the ids and go out afterwards.
    #[tokio::test(start_paused = true)]
    async fn bus_toggled_before_the_posts_are_answered() {
        let mut r = rig(false);
        r.settle().await;
        r.drain();
        r.cmd(bus(true)).await.unwrap();
        r.settle().await;
        let posts = r.drain();
        r.cmd(bus(false)).await.unwrap();
        r.settle().await;
        let early = r.drain();
        assert_eq!(clears(&early), 0, "no id yet");
        r.ack(&posts);
        r.settle().await;
        r.ack(&early);
        r.settle().await;
        let late = r.drain();
        assert_eq!(clears(&late), 1 + 4 * 2 + 2);
        assert!(r.handle.snapshot().active_faults.is_empty());
    }

    /// A cascade alarm whose POST failed does not exist: recovery sends no "Alarm cleared" for
    /// it, and the next sync raises it again.
    #[tokio::test(start_paused = true)]
    async fn failed_cascade_post_is_retried_and_not_recovered() {
        let mut r = rig(false);
        r.settle().await;
        r.drain();
        r.cmd(bus(true)).await.unwrap();
        r.settle().await;
        let items = r.drain();
        for i in &items {
            let result = match &i.payload {
                OutboundPayload::Alarm(b) if b["type"] == "lw_converter_1_01_lost" => {
                    Err(crate::error::AppError::Internal("down".into()))
                }
                OutboundPayload::Alarm(_) => Ok(json!({ "id": format!("a{}", i.tag) })),
                _ => Ok(json!({})),
            };
            let _ = r.results.send(OutboundResult { tag: i.tag, result });
        }
        r.settle().await;
        r.drain();
        // The next sync (here: a group change) raises it once more.
        r.cmd(SessionCommand::ConfigChanged {
            groups: Some(fixture_groups()),
            inputs: None,
        })
        .await
        .unwrap();
        r.settle().await;
        let retry = r.drain();
        assert_eq!(
            alarm_types(&retry),
            vec!["lw_converter_1_01_lost".to_owned()]
        );

        // Fail it again, then recover: nothing to clear for it.
        for i in &retry {
            let _ = r.results.send(OutboundResult {
                tag: i.tag,
                result: Err(crate::error::AppError::Internal("down".into())),
            });
        }
        r.settle().await;
        r.drain();
        r.cmd(bus(false)).await.unwrap();
        let items = r.flush().await;
        let texts = cleared_texts(&items);
        assert!(!texts.contains(&"Alarm cleared: lw_converter_1_01_lost".to_owned()));
        assert_eq!(texts.len(), 1 + 4 * 2 - 1 + 2, "{texts:?}");
        assert_eq!(clears(&items), texts.len());
    }

    /// A fault raised while its alarm is open already (held by the bus) carries the alarm id.
    #[tokio::test(start_paused = true)]
    async fn fault_on_a_held_alarm_takes_its_id() {
        let mut r = rig(false);
        r.settle().await;
        r.drain();
        r.cmd(bus(true)).await.unwrap();
        r.flush().await;
        let target = FaultTarget::DaliLamp {
            lamp: 2,
            alarm_type: "lw_converter_1_02_lost".into(),
        };
        r.cmd(SessionCommand::SetFault {
            target: target.clone(),
            active: true,
        })
        .await
        .unwrap();
        let faults = r.handle.snapshot().active_faults;
        let f = faults.iter().find(|f| f.target == target).unwrap();
        assert!(f.alarm_id.as_deref().is_some_and(|id| id.starts_with('a')));
    }

    /// Realistic mode: the firmware only sends "Alarm cleared" events for the cascade. Clearing
    /// a lamp's own fault while the bus is down clears its alarm, but the lamp stays lost (and
    /// the alarm is not raised again) until the bus recovers (D9).
    #[tokio::test(start_paused = true)]
    async fn bus_recovery_in_realistic_mode_sends_events_only() {
        let mut r = rig_with(prepared(&fixture(), false, true));
        r.settle().await;
        r.drain();
        let lost3 = FaultTarget::DaliLamp {
            lamp: 3,
            alarm_type: "lw_converter_1_03_lost".into(),
        };
        r.cmd(SessionCommand::SetFault {
            target: lost3.clone(),
            active: true,
        })
        .await
        .unwrap();
        // A lamp's own `_lost` fault also raises its control gear failure.
        let own = alarm_types(&r.flush().await);
        assert_eq!(
            own,
            vec![
                "lw_converter_1_03_lost".to_owned(),
                "lw_controlGearFailure_1_03".to_owned()
            ]
        );
        r.cmd(bus(true)).await.unwrap();
        let raised = alarm_types(&r.flush().await);
        assert!(
            !raised.contains(&"lw_converter_1_03_lost".to_owned())
                && !raised.contains(&"lw_controlGearFailure_1_03".to_owned()),
            "open already"
        );
        r.cmd(SessionCommand::SetFault {
            target: lost3,
            active: false,
        })
        .await
        .unwrap();
        let items = r.flush().await;
        assert!(
            cleared_texts(&items).is_empty(),
            "still lost: the bus is down"
        );
        assert_eq!(clears(&items), 1, "the scenario clear clears the alarm");
        assert!(alarm_types(&items).is_empty(), "not raised again");
        r.cmd(bus(false)).await.unwrap();
        let items = r.flush().await;
        let texts = cleared_texts(&items);
        assert!(texts.contains(&"Alarm cleared: lw_converter_1_03_lost".to_owned()));
        assert!(texts.contains(&"Alarm cleared: lw_controlGearFailure_1_03".to_owned()));
        assert_eq!(clears(&items), 1, "only the bus alarm itself");
        assert!(
            has_series(
                &measurements(&items),
                "LW Dali Control Gear 1-03",
                "Brightness"
            )
            .is_some()
        );
    }

    /// Open alarms at session start: lamp lost alarms next to an open bus power alarm belong to
    /// the bus failure and recover with it (no lamp faults of their own).
    #[tokio::test(start_paused = true)]
    async fn restored_bus_failure_owns_the_lamp_alarms() {
        let mut p = prepared(&fixture(), false, false);
        let restored = |t: &str, id: &str| super::super::core::RestoredAlarm {
            alarm_type: t.into(),
            id: Some(id.into()),
            raised_at: now_iso(),
            status: crate::model::OpenAlarmStatus::Active,
        };
        p.params.restored_alarms = vec![
            restored("lw_daliBusPowerFail", "B"),
            restored("lw_converter_1_01_lost", "L1"),
            restored("lw_controlGearFailure_1_01", "G1"),
        ];
        // No start scenario (it would clear the bus fault).
        p.params.scenarios.retain(|s| !s.start);
        let mut r = rig_with(p);
        r.settle().await;
        let items = r.drain();
        let raised = alarm_types(&items);
        assert!(!raised.iter().any(|t| t == "lw_converter_1_01_lost"
            || t == "lw_controlGearFailure_1_01"
            || t == "lw_daliBusPowerFail"));
        assert!(raised.contains(&"lw_converter_1_02_lost".to_owned()));
        r.ack(&items);
        let faults = r.handle.snapshot().active_faults;
        assert_eq!(faults.len(), 1, "{faults:?}");
        r.cmd(bus(false)).await.unwrap();
        let items = r.flush().await;
        assert!(
            cleared_texts(&items).contains(&"Alarm cleared: lw_converter_1_01_lost".to_owned())
        );
        assert!(items.iter().any(|i| matches!(&i.payload,
            OutboundPayload::AlarmClear { alarm_id } if alarm_id == "L1")));
    }

    #[tokio::test(start_paused = true)]
    async fn scenario_raises_all_its_faults() {
        let mut r = rig(false);
        r.settle().await;
        r.drain();
        r.cmd(SessionCommand::ApplyScenario {
            scenario_id: "trouble".into(),
        })
        .await
        .unwrap();
        r.settle().await;
        let types: Vec<String> = r
            .drain()
            .iter()
            .filter_map(|i| match &i.payload {
                OutboundPayload::Alarm(a) => a["type"].as_str().map(str::to_owned),
                _ => None,
            })
            .collect();
        assert_eq!(
            types,
            vec![
                "lw_modbus_20_timeoutError",
                "lw_demo_20_co2Leakage",
                "lw_demo_doorOpen",
                "lw_lampFailure_1_02_openCircuit"
            ]
        );
        assert_eq!(r.handle.snapshot().active_faults.len(), 4);
        assert!(
            r.cmd(SessionCommand::ApplyScenario {
                scenario_id: "nope".into()
            })
            .await
            .is_err()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn manual_override_counter_and_revisions() {
        let mut r = rig(false);
        r.settle().await;
        r.drain();
        let rev0 = r.handle.snapshot().revision;
        r.cmd(SessionCommand::SetValue {
            item_id: "freezer_temp".into(),
            value: -12.0,
        })
        .await
        .unwrap();
        r.settle().await;
        assert!(
            measurements(&r.drain()).is_empty(),
            "a manual change waits for the debounce window"
        );
        tokio::time::sleep(Duration::from_millis(400)).await;
        let ms = measurements(&r.drain());
        assert_eq!(ms.len(), 1, "only the changed item is sent");
        assert_eq!(
            has_series(&ms, "LW_Modbus_Slave_20_240", "freezer_temp"),
            Some(-12.0)
        );
        tokio::time::sleep(Duration::from_secs(6)).await;
        let snap = r.handle.snapshot();
        let f = snap
            .items
            .iter()
            .find(|i| i.item_id == "freezer_temp")
            .unwrap();
        assert!(f.manual_override);
        assert_eq!(f.value, -12.0, "generator does not move an overridden item");
        assert!(snap.revision > rev0);

        r.cmd(SessionCommand::ResumeGenerator {
            item_id: "freezer_temp".into(),
        })
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_secs(6)).await;
        let f = r
            .handle
            .snapshot()
            .items
            .into_iter()
            .find(|i| i.item_id == "freezer_temp")
            .unwrap();
        assert!(!f.manual_override);
        assert_ne!(f.value, -12.0);

        // Out of range / not a value / counter backwards.
        for (id, v) in [
            ("freezer_temp", 5.0),
            ("doorOpen", 1.0),
            ("gas-meter", 10.0),
        ] {
            assert!(
                r.cmd(SessionCommand::SetValue {
                    item_id: id.into(),
                    value: v
                })
                .await
                .is_err(),
                "{id}"
            );
        }
        r.cmd(SessionCommand::SetValue {
            item_id: "gas-meter".into(),
            value: 2000.0,
        })
        .await
        .unwrap();
        // Revisions on events increase.
        let revs: Vec<u32> = r
            .emitter
            .events()
            .iter()
            .filter_map(|e| match e {
                SessionEvent::Changed(Some(s)) => Some(s.revision),
                SessionEvent::Values(v) => Some(v.revision),
                SessionEvent::Fault(f) => Some(f.revision),
                _ => None,
            })
            .collect();
        assert!(revs.windows(2).all(|w| w[0] < w[1]), "{revs:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn counters_resume_from_stored_value() {
        let lib = fixture();
        let mut p = prepared(&lib, false, false);
        p.params.counter_seeds.insert("gas-meter".into(), 1240.0);
        let mut r = rig_with(p);
        r.settle().await;
        let ms = measurements(&r.drain());
        assert_eq!(
            has_series(&ms, "LW Digital Input", "ImpulseCounter03"),
            Some(1240.0)
        );
        tokio::time::sleep(Duration::from_secs(60)).await;
        let v = r
            .handle
            .snapshot()
            .items
            .into_iter()
            .find(|i| i.item_id == "gas-meter")
            .unwrap()
            .value;
        assert!(v >= 1240.0);
    }

    #[tokio::test(start_paused = true)]
    async fn digital_input_dali_action_switches_groups() {
        let mut r = rig(false);
        r.settle().await;
        r.drain();
        r.cmd(SessionCommand::SetValue {
            item_id: "di-pump-1".into(),
            value: 1.0,
        })
        .await
        .unwrap();
        r.settle().await;
        let ev = events_of(&r.drain());
        assert_eq!(ev[0]["groupModeChange"]["id"], 2);
        assert_eq!(ev[0]["groupModeChange"]["newMode"], "On");
        assert_eq!(ev[0]["groupModeChange"]["reason"], "digital-input-01");
    }

    #[tokio::test(start_paused = true)]
    async fn stop_fails_open_prompts_and_releases_lock() {
        let mut r = rig(false);
        r.settle().await;
        r.op(group_op("s1", 1, "on")).await;
        r.flush().await;
        let Rig {
            handle,
            mut out,
            results,
            emitter,
            ..
        } = r;
        let stopper = tokio::spawn(handle.stop());
        tokio::time::sleep(Duration::from_millis(10)).await;
        let mut items = Vec::new();
        while let Ok(i) = out.try_recv() {
            items.push(i);
        }
        for i in &items {
            let _ = results.send(OutboundResult {
                tag: i.tag,
                result: Ok(json!({})),
            });
        }
        stopper.await.unwrap().unwrap();
        assert!(statuses(&items).contains(&(
            "s1".into(),
            OperationStatus::Failed,
            Some("Simulator stopped".into())
        )));
        // A deliberate stop is not an error: the UI must not toast it as one (F15).
        assert!(resolutions(&emitter).contains(&("s1".into(), OperationResolution::Stopped)));
        // No lock was held by "demo" in this rig (plan has none), so no release patch is needed.
    }
}

mod fake {
    use super::*;
    use crate::c8y::fake::FakeC8y;
    use crate::device::{crud, demo_state};
    use crate::model::{ApplyDemoRequest, CreateDeviceRequest, StartSessionRequest};

    pub(super) async fn device(fake: &FakeC8y, lib: &Library) -> String {
        let client = fake.client();
        let dev = crud::create(
            &client,
            lib,
            &CreateDeviceRequest {
                name: "Testdemo".into(),
                description: None,
                position: None,
                parent_group_id: None,
                apply_demo: None,
            },
        )
        .await
        .unwrap();
        apply::apply(
            &client,
            lib,
            &dev.id,
            &ApplyDemoRequest {
                demo_id: "fixture".into(),
                apply_floorplans: false,
                replace_existing_floorplans: false,
                force: false,
            },
        )
        .await
        .unwrap();
        dev.id
    }

    /// Realistic mode leaves recovered alarms ACTIVE; the session start finds them by their
    /// "Alarm cleared" events. A busy recovery (more than 200 events) must not push an older
    /// one out of the window.
    #[tokio::test]
    async fn recovered_alarms_are_found_despite_many_later_events() {
        let fake = FakeC8y::start().await;
        let client = fake.client();
        let dev = "4711";
        let alarm = |t: &str, at: &str| json!({ "type": t, "text": t, "severity": "MAJOR", "time": at, "source": { "id": dev } });
        client
            .create_alarm(&alarm("lw_old", "2026-10-12T10:00:00.000Z"))
            .await
            .unwrap();
        client
            .create_alarm(&alarm("lw_open", "2026-10-12T10:30:00.000Z"))
            .await
            .unwrap();
        let event = |text: String, at: String| json!({ "type": "lw_alarm", "text": text, "time": at, "source": { "id": dev } });
        client
            .create_event(&event(
                "Alarm cleared: lw_old".into(),
                "2026-10-12T10:01:00.000Z".into(),
            ))
            .await
            .unwrap();
        for i in 0..250 {
            client
                .create_event(&event(
                    format!("Alarm cleared: lw_other_{i}"),
                    format!("2026-10-12T11:{:02}:{:02}.000Z", i / 60, i % 60),
                ))
                .await
                .unwrap();
        }
        let open = super::super::session::open_alarms(&client, dev).await;
        let types: Vec<&str> = open.iter().map(|a| a.alarm_type.as_str()).collect();
        assert_eq!(types, vec!["lw_open"]);
    }

    pub(super) async fn wait_for(what: &str, mut f: impl FnMut() -> bool) {
        for _ in 0..200 {
            if f() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("timed out waiting for {what}");
    }

    pub(super) async fn start(
        fake: &FakeC8y,
        lib: &Library,
        id: &str,
    ) -> (SessionHandle, Arc<RecordingEmitter>) {
        let emitter = RecordingEmitter::new();
        let h = SessionHandle::start_with(
            fake.client(),
            lib,
            StartSessionRequest {
                device_id: id.into(),
                force: false,
                auto_accept: false,
            },
            emitter.clone() as Arc<dyn SessionEmitter>,
        )
        .await
        .unwrap();
        (h, emitter)
    }

    /// A demo device from an older app version (no `com_cumulocity_model_Agent`) is migrated
    /// at session start, so operations for it are accepted by Cumulocity and reach the session.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn session_start_makes_a_legacy_device_its_own_agent() {
        let fake = FakeC8y::start().await;
        let lib = fixture();
        let id = device(&fake, &lib).await;
        {
            let mut s = fake.state();
            let mo = s
                .managed_objects
                .iter_mut()
                .find(|m| m["id"] == id.as_str())
                .unwrap();
            mo.as_object_mut()
                .unwrap()
                .remove("com_cumulocity_model_Agent");
        }
        assert!(
            fake.try_create_operation(&id, json!({ "c8y_Restart": {} }))
                .is_err()
        );
        let (h, _e) = start(&fake, &lib, &id).await;
        assert!(
            fake.state()
                .managed_objects
                .iter()
                .find(|m| m["id"] == id.as_str())
                .unwrap()
                .get("com_cumulocity_model_Agent")
                .is_some()
        );
        wait_for("realtime connected", || h.snapshot().realtime_connected).await;
        let op = fake
            .try_create_operation(
                &id,
                json!({ "lw_switchGroupMode": { "group": 1, "value": "on", "by": "x" } }),
            )
            .unwrap();
        assert_eq!(op["status"], "PENDING");
        wait_for("prompt", || !h.snapshot().pending_operations.is_empty()).await;
        h.stop().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn session_posts_native_measurements_and_holds_lock() {
        let fake = FakeC8y::start().await;
        let lib = fixture();
        let id = device(&fake, &lib).await;
        let (h, _e) = start(&fake, &lib, &id).await;
        wait_for("measurements", || fake.state().measurements.len() > 10).await;
        let ms = fake.state().measurements.clone();
        assert!(ms.iter().all(|m| m["source"]["id"] == id.as_str()));
        let dali = ms
            .iter()
            .find(|m| m.get("LW Dali Control Gear 1-01").is_some())
            .unwrap();
        assert_eq!(dali["type"], "daliControlGear");
        let s = &dali["LW Dali Control Gear 1-01"];
        assert_eq!(s["Brightness"]["unit"], "%");
        assert!(s["Lamp Status"].get("unit").is_none());
        assert_eq!(s["Converter Runtime"]["unit"], "hr");
        let modbus = ms
            .iter()
            .find(|m| m.get("LW_Modbus_Slave_20_240").is_some())
            .unwrap();
        assert_eq!(modbus["type"], "modbus");
        assert_eq!(
            modbus["LW_Modbus_Slave_20_240"]["freezer_temp"]["unit"],
            "°C"
        );
        assert!(
            ms.iter()
                .any(|m| m["LW Digital Input"]["Value01"].is_object())
        );
        assert!(
            ms.iter()
                .any(|m| m["c8y_TemperatureMeasurement"]["T"]["unit"] == "C")
        );
        // A collection request was used.
        assert!(fake.requests().iter().any(|r| {
            r.path == "/measurement/measurements"
                && r.content_type
                    .as_deref()
                    .is_some_and(|c| c.contains("measurementcollection"))
        }));

        // Lock is held; a second presenter is refused.
        let mo = fake.client().get_managed_object(&id).await.unwrap();
        let st = demo_state::read(&mo).unwrap().unwrap();
        assert_eq!(st.lock.as_ref().unwrap().user, "demo");
        let other = fake.client_with("other", crate::c8y::fake::PASSWORD);
        let _ = other;
        h.stop().await.unwrap();
        let mo = fake.client().get_managed_object(&id).await.unwrap();
        assert!(
            demo_state::read(&mo).unwrap().unwrap().lock.is_none(),
            "lock released"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn foreign_fresh_lock_blocks_start_unless_forced() {
        let fake = FakeC8y::start().await;
        let lib = fixture();
        let id = device(&fake, &lib).await;
        let client = fake.client();
        let mo = client.get_managed_object(&id).await.unwrap();
        client
            .update_managed_object(
                &id,
                &demo_state::lock_body(Some(&demo_state::new_lock("someone-else"))).unwrap(),
            )
            .await
            .unwrap();
        let _ = mo;
        let err = SessionHandle::start_with(
            client.clone(),
            &lib,
            StartSessionRequest {
                device_id: id.clone(),
                force: false,
                auto_accept: false,
            },
            RecordingEmitter::new() as Arc<dyn SessionEmitter>,
        )
        .await
        .err()
        .unwrap();
        assert!(matches!(err, crate::error::AppError::DeviceLocked { .. }));
        let h = SessionHandle::start_with(
            client,
            &lib,
            StartSessionRequest {
                device_id: id,
                force: true,
                auto_accept: false,
            },
            RecordingEmitter::new() as Arc<dyn SessionEmitter>,
        )
        .await
        .unwrap();
        h.stop().await.unwrap();
    }

    pub(super) fn op_status(fake: &FakeC8y, id: &str) -> (String, Option<String>) {
        let s = fake.state();
        let op = s.operations.iter().find(|o| o["id"] == id).unwrap();
        (
            op["status"].as_str().unwrap().to_owned(),
            op["failureReason"].as_str().map(str::to_owned),
        )
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn operations_prompt_confirm_abort_and_offline() {
        let fake = FakeC8y::start().await;
        let lib = fixture();
        let id = device(&fake, &lib).await;
        // A stale PENDING op from before the session.
        let stale = {
            let mut s = fake.state();
            let oid = "9001".to_owned();
            s.operations.push(json!({
                "id": oid, "deviceId": id, "status": "PENDING",
                "creationTime": (chrono::Utc::now() - chrono::Duration::seconds(300))
                    .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                "lw_switchGroupMode": { "group": 1, "value": "on" }
            }));
            oid
        };
        let (h, emitter) = start(&fake, &lib, &id).await;
        wait_for("offline FAILED", || op_status(&fake, &stale).0 == "FAILED").await;
        assert_eq!(
            op_status(&fake, &stale).1.as_deref(),
            Some("Simulator was offline")
        );
        wait_for("realtime connected", || h.snapshot().realtime_connected).await;

        let op = fake.create_operation(
            &id,
            json!({ "lw_switchGroupMode": { "group": 1, "value": "on", "by": "x" } }),
        );
        let oid = op["id"].as_str().unwrap().to_owned();
        wait_for("EXECUTING", || op_status(&fake, &oid).0 == "EXECUTING").await;
        wait_for("prompt", || !h.snapshot().pending_operations.is_empty()).await;
        h.send(SessionCommand::AnswerOperation {
            operation_id: oid.clone(),
            decision: OperationDecision::Confirm,
        })
        .await
        .unwrap();
        wait_for("SUCCESSFUL", || op_status(&fake, &oid).0 == "SUCCESSFUL").await;
        wait_for("group change event", || {
            fake.state()
                .events
                .iter()
                .any(|e| e["type"] == "lw_daliGroupChange")
        })
        .await;

        let op = fake.create_operation(&id, json!({ "lw_modbusWrite": { "slaveAddress": 239, "registerAddress": 40002, "value": 25, "type": "register" } }));
        let oid2 = op["id"].as_str().unwrap().to_owned();
        wait_for("prompt 2", || !h.snapshot().pending_operations.is_empty()).await;
        h.send(SessionCommand::AnswerOperation {
            operation_id: oid2.clone(),
            decision: OperationDecision::Abort {
                reason: Some("Nicht jetzt".into()),
            },
        })
        .await
        .unwrap();
        wait_for("FAILED", || op_status(&fake, &oid2).0 == "FAILED").await;
        assert_eq!(op_status(&fake, &oid2).1.as_deref(), Some("Nicht jetzt"));
        assert!(
            resolutions(&emitter)
                .iter()
                .any(|(o, r)| *o == oid && *r == OperationResolution::Confirmed)
        );
        h.stop().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn slave_timeout_and_recover_in_cumulocity() {
        let fake = FakeC8y::start().await;
        let lib = fixture();
        let id = device(&fake, &lib).await;
        let (h, _e) = start(&fake, &lib, &id).await;
        let target = FaultTarget::SlaveTimeout {
            slave_key: "kuehlung".into(),
        };
        h.send(SessionCommand::SetFault {
            target: target.clone(),
            active: true,
        })
        .await
        .unwrap();
        wait_for("alarm", || {
            fake.state()
                .alarms
                .iter()
                .any(|a| a["type"] == "lw_modbus_20_timeoutError" && a["status"] == "ACTIVE")
        })
        .await;
        wait_for("lw_modbus event", || {
            fake.state().events.iter().any(|e| e["type"] == "lw_modbus")
        })
        .await;
        wait_for("alarm id in snapshot", || {
            h.snapshot()
                .active_faults
                .first()
                .is_some_and(|f| f.alarm_id.is_some())
        })
        .await;
        h.send(SessionCommand::SetFault {
            target,
            active: false,
        })
        .await
        .unwrap();
        wait_for("cleared", || {
            fake.state()
                .alarms
                .iter()
                .any(|a| a["type"] == "lw_modbus_20_timeoutError" && a["status"] == "CLEARED")
        })
        .await;
        // The clear is queued before the "Alarm cleared" event (faults.rs: put_alarm_clear, then
        // end_fault), so the event may still be in flight when the alarm reads CLEARED.
        wait_for("alarm cleared event", || {
            fake.state().events.iter().any(|e| {
                e["type"] == "lw_alarm" && e["text"] == "Alarm cleared: lw_modbus_20_timeoutError"
            })
        })
        .await;
        h.stop().await.unwrap();
    }
}

// ───────────────────────────── phase 3 review fixes (docs/reviews/phase3-engine.md) ─────────────────────────────

mod review_paused {
    use super::*;
    use crate::model::{DaliGroup, DigitalInputEntry, GeneratorConfig};
    use crate::sim::core::SessionCore;
    use std::sync::atomic::AtomicI64;
    use tokio::time::Instant;

    /// Finding 1: the session starts in the mode the newest group event says, restoring is
    /// silent, and a cloud `lw_switchGroupMode` is always reported, even for the same mode.
    #[tokio::test(start_paused = true)]
    async fn group_mode_is_restored_and_cloud_switch_always_reports() {
        let lib = fixture();
        let mut p = prepared(&lib, false, false);
        p.params.group_modes.insert(1, GroupMode::On);
        let mut r = rig_with(p);
        r.settle().await;
        let items = r.drain();
        assert_eq!(r.handle.snapshot().groups[0].mode, GroupMode::On);
        assert_eq!(r.handle.snapshot().groups[1].mode, GroupMode::Auto);
        assert!(
            events_of(&items)
                .iter()
                .all(|e| e["type"] != "lw_daliGroupChange"),
            "restoring a mode sends no event"
        );
        let ms = measurements(&items);
        assert_eq!(
            has_series(&ms, "LW Dali Control Gear 1-01", "Brightness"),
            Some(100.0),
            "group 1 is on at its max level"
        );
        r.ack(&items);

        // The presenter clicks "On" in the floorplan again: the mode does not change, but the
        // plugin reads the newest event.
        r.op(group_op("g1", 1, "on")).await;
        r.flush().await;
        r.cmd(SessionCommand::AnswerOperation {
            operation_id: "g1".into(),
            decision: OperationDecision::Confirm,
        })
        .await
        .unwrap();
        r.settle().await;
        let items = r.drain();
        let ev = events_of(&items);
        assert_eq!(ev.len(), 1, "{ev:?}");
        assert_eq!(ev[0]["groupModeChange"]["previousMode"], "On");
        assert_eq!(ev[0]["groupModeChange"]["newMode"], "On");

        // A DI action keeps the "only on change" rule; so does the local command.
        r.ack(&items);
        r.cmd(SessionCommand::SetGroupMode {
            group_id: 1,
            mode: GroupMode::On,
        })
        .await
        .unwrap();
        r.settle().await;
        assert!(events_of(&r.drain()).is_empty());
    }

    /// Finding 16: the start scenario does not replace a generator the presenter persisted.
    #[tokio::test(start_paused = true)]
    async fn start_scenario_keeps_persisted_generator_overrides() {
        let lib = fixture();
        let mut p = prepared(&lib, false, false);
        let sine = GeneratorConfig::Sine {
            period: 600.0,
            amplitude: 1.0,
            center: Some(-18.0),
        };
        p.params
            .demo
            .generators
            .insert("freezer_temp".into(), sine.clone());
        let r = rig_with(p);
        r.settle().await;
        let f = r
            .handle
            .snapshot()
            .items
            .into_iter()
            .find(|i| i.item_id == "freezer_temp")
            .unwrap();
        assert_eq!(f.generator, Some(sine), "not the scenario's randomWalk");
        assert_eq!(f.value, -18.0, "its value step still applies");
        // A manually applied scenario keeps overriding as before.
        let normal = r.cmd(SessionCommand::ApplyScenario {
            scenario_id: "normal".into(),
        });
        normal.await.unwrap();
        assert!(matches!(
            r.handle
                .snapshot()
                .items
                .into_iter()
                .find(|i| i.item_id == "freezer_temp")
                .unwrap()
                .generator,
            Some(GeneratorConfig::RandomWalk { .. })
        ));
    }

    /// Finding 35: at stop, queued measurements are discarded; the lock release and the final
    /// statuses are not stuck behind them.
    #[tokio::test(start_paused = true)]
    async fn stop_discards_queued_measurements_and_sends_the_release() {
        let mut r = rig(false);
        r.settle().await;
        r.drain();
        r.op(group_op("s9", 1, "on")).await;
        r.flush().await;
        r.cmd(SessionCommand::SetValue {
            item_id: "freezer_temp".into(),
            value: -12.0,
        })
        .await
        .unwrap();
        r.cmd(SessionCommand::SendAllNow).await.unwrap();
        r.settle().await;
        assert!(r.queue.len().0 > 0, "measurements are queued, nobody sends");
        let Rig {
            handle,
            mut out,
            results,
            queue,
            ..
        } = r;
        let stopper = tokio::spawn(handle.stop());
        tokio::time::sleep(Duration::from_millis(10)).await;
        let mut items = Vec::new();
        while let Ok(i) = out.try_recv() {
            items.push(i);
        }
        assert!(
            measurements(&items).is_empty(),
            "stale measurements were discarded"
        );
        assert!(
            items
                .iter()
                .any(|i| matches!(i.payload, OutboundPayload::LockRelease { .. }))
        );
        assert!(statuses(&items).contains(&(
            "s9".into(),
            OperationStatus::Failed,
            Some("Simulator stopped".into())
        )));
        for i in &items {
            let _ = results.send(OutboundResult {
                tag: i.tag,
                result: Ok(json!({})),
            });
        }
        stopper.await.unwrap().unwrap();
        assert!(queue.is_empty());
    }

    /// Finding 18: a slider drag makes a handful of requests, not one per change.
    #[tokio::test(start_paused = true)]
    async fn manual_changes_are_coalesced() {
        let mut r = rig(false);
        r.settle().await;
        r.drain();
        for i in 0..20 {
            r.cmd(SessionCommand::SetValue {
                item_id: "freezer_temp".into(),
                value: -25.0 + f64::from(i),
            })
            .await
            .unwrap();
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
        let items = r.drain();
        let ms = measurements(&items);
        let sends = ms
            .iter()
            .filter(|m| m.get("LW_Modbus_Slave_20_240").is_some())
            .count();
        assert!((2..=5).contains(&sends), "{sends} sends for 20 changes");
        assert_eq!(
            has_series(&ms, "LW_Modbus_Slave_20_240", "freezer_temp"),
            Some(-6.0),
            "the latest value goes out"
        );
    }

    /// Finding 23: the final status follows the readback and waits for EXECUTING's answer.
    #[tokio::test(start_paused = true)]
    async fn final_status_waits_for_executing_and_follows_the_effects() {
        let mut r = rig(true);
        r.settle().await;
        r.drain();
        r.op(group_op("f1", 1, "off")).await;
        let first = r.drain();
        assert_eq!(
            statuses(&first),
            vec![("f1".into(), OperationStatus::Executing, None)]
        );
        // Effects are queued, SUCCESSFUL is not (EXECUTING is unanswered).
        assert!(!events_of(&first).is_empty());
        r.ack(&first);
        r.settle().await;
        let second = r.drain();
        assert_eq!(
            statuses(&second),
            vec![("f1".into(), OperationStatus::Successful, None)]
        );
    }

    /// Finding 6c / 27: the plugin cancels a PENDING op before our EXECUTING landed; the prompt
    /// is withdrawn (and reported), and FAILED is said again behind EXECUTING.
    #[tokio::test(start_paused = true)]
    async fn late_executing_after_plugin_cancel_is_failed_again() {
        let mut r = rig(false);
        r.settle().await;
        r.drain();
        r.op(group_op("w1", 1, "on")).await;
        let executing = r.drain();
        assert_eq!(statuses(&executing).len(), 1);
        let mut failed = group_op("w1", 1, "on");
        failed["status"] = json!("FAILED");
        failed["failureReason"] = json!("Operation was not executed in time");
        r.op(failed).await;
        assert!(r.handle.snapshot().pending_operations.is_empty());
        assert!(
            statuses(&r.drain()).is_empty(),
            "FAILED waits for the answer to EXECUTING"
        );
        let resolved: Vec<_> = r
            .emitter
            .events()
            .into_iter()
            .filter_map(|e| match e {
                SessionEvent::Resolved(x) => Some(x.0),
                _ => None,
            })
            .collect();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].resolution, OperationResolution::Withdrawn);
        assert_eq!(resolved[0].status, OperationStatus::Failed);
        assert_eq!(
            resolved[0].reason.as_deref(),
            Some("Operation was not executed in time")
        );
        // EXECUTING finally lands.
        r.ack(&executing);
        r.settle().await;
        assert_eq!(
            statuses(&r.drain()),
            vec![(
                "w1".into(),
                OperationStatus::Failed,
                Some("Operation was not executed in time".into())
            )]
        );
        // Our own FAILED echo is not a second withdrawal.
        let mut echo = group_op("w1", 1, "on");
        echo["status"] = json!("FAILED");
        r.op(echo).await;
        assert!(statuses(&r.drain()).is_empty());
    }

    /// Finding 7: only the PENDING list counts as "offline", measured on the server's clock.
    #[tokio::test(start_paused = true)]
    async fn offline_rule_uses_the_server_clock_and_only_the_initial_list() {
        let lib = fixture();
        let mut p = prepared(&lib, false, false);
        // The laptop is 2 minutes ahead of the server.
        p.params.clock_skew = Arc::new(AtomicI64::new(-120_000));
        let mut r = rig_with(p);
        r.settle().await;
        r.drain();
        let server_5s_ago = (chrono::Utc::now() - chrono::Duration::seconds(125))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let mut fresh = group_op("k1", 1, "on");
        fresh["creationTime"] = json!(server_5s_ago);
        r.pending_op(fresh).await;
        // A live operation is never "offline", whatever the timestamps say.
        let mut live = group_op("k2", 2, "on");
        live["creationTime"] = json!(
            (chrono::Utc::now() - chrono::Duration::seconds(600))
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        );
        r.op(live).await;
        let s = statuses(&r.drain());
        assert_eq!(
            s,
            vec![
                ("k1".into(), OperationStatus::Executing, None),
                ("k2".into(), OperationStatus::Executing, None)
            ]
        );
        assert_eq!(r.handle.snapshot().pending_operations.len(), 2);
    }

    /// Finding 26: a group that lists a lamp the device does not have sends nothing for it.
    #[tokio::test(start_paused = true)]
    async fn group_switch_sends_no_phantom_lamps() {
        let lib = fixture();
        let mut p = prepared(&lib, false, false);
        p.params.groups[0].converters.push(60);
        let mut r = rig_with(p);
        r.settle().await;
        r.drain();
        r.cmd(SessionCommand::SetGroupMode {
            group_id: 1,
            mode: GroupMode::On,
        })
        .await
        .unwrap();
        r.settle().await;
        let ms = measurements(&r.drain());
        assert!(
            ms.iter()
                .all(|m| m.get("LW Dali Control Gear 1-60").is_none()),
            "{ms:?}"
        );
        assert!(has_series(&ms, "LW Dali Control Gear 1-01", "Brightness").is_some());
    }

    /// Finding 28: an alarm `POST` that failed does not stay "active" forever.
    #[tokio::test(start_paused = true)]
    async fn failed_alarm_post_does_not_leave_a_dangling_fault() {
        let mut r = rig(false);
        r.settle().await;
        r.drain();
        let target = FaultTarget::DaliLamp {
            lamp: 1,
            alarm_type: "lw_lampFailure_1_01_openCircuit".into(),
        };
        r.cmd(SessionCommand::SetFault {
            target: target.clone(),
            active: true,
        })
        .await
        .unwrap();
        r.settle().await;
        let items = r.drain();
        let alarm = items
            .iter()
            .find(|i| matches!(i.payload, OutboundPayload::Alarm(_)))
            .unwrap();
        let faults = |r: &Rig| {
            r.emitter
                .events()
                .iter()
                .filter(|e| matches!(e, SessionEvent::Fault(_)))
                .count()
        };
        let before = faults(&r);
        r.results
            .send(OutboundResult {
                tag: alarm.tag,
                result: Err(crate::error::AppError::Http {
                    status: 500,
                    message: "boom".into(),
                }),
            })
            .unwrap();
        r.settle().await;
        assert!(faults(&r) > before, "fault-changed after the failure");
        // No alarm exists, so nothing can end the fault: it ends with the failed create (D9),
        // without a recovery event.
        assert!(r.handle.snapshot().active_faults.is_empty());
        assert!(
            r.handle.snapshot().items.iter().all(|i| !i.paused),
            "the lamp sends again"
        );
        // Clearing now must not wait for an id that never comes.
        r.cmd(SessionCommand::SetFault {
            target: target.clone(),
            active: false,
        })
        .await
        .unwrap();
        r.settle().await;
        let items = r.drain();
        assert!(events_of(&items).iter().all(|e| {
            !e["text"]
                .as_str()
                .is_some_and(|t| t.starts_with("Alarm cleared"))
        }));
        assert!(
            !items
                .iter()
                .any(|i| matches!(i.payload, OutboundPayload::AlarmClear { .. }))
        );
        // And a new raise works.
        r.cmd(SessionCommand::SetFault {
            target,
            active: true,
        })
        .await
        .unwrap();
        r.settle().await;
        assert!(
            r.drain()
                .iter()
                .any(|i| matches!(i.payload, OutboundPayload::Alarm(_)))
        );
    }

    /// Finding 34: Modbus writes are as strict as the firmware.
    #[tokio::test(start_paused = true)]
    async fn modbus_write_rejects_fractions_and_inactive_rs485() {
        let mut r = rig(true);
        r.settle().await;
        r.drain();
        r.op(
            json!({ "id": "m1", "status": "PENDING", "creationTime": now_iso(),
                     "lw_modbusWrite": { "slaveAddress": 239, "registerAddress": 40002,
                                         "value": 22.5, "type": "register" } }),
        )
        .await;
        let mut items = r.flush().await;
        items.extend(r.drain());
        assert!(measurements(&items).is_empty());
        assert_eq!(
            statuses(&items)[1],
            (
                "m1".into(),
                OperationStatus::Failed,
                Some("The specified lw_modbusWrite.value must be an integer".into())
            )
        );

        let lib = fixture();
        let mut p = prepared(&lib, true, false);
        p.params.rs485_active = false;
        let mut r = rig_with(p);
        r.settle().await;
        r.drain();
        r.op(
            json!({ "id": "m2", "status": "PENDING", "creationTime": now_iso(),
                     "lw_modbusWrite": { "slaveAddress": 239, "registerAddress": 40002,
                                         "value": 22, "type": "register" } }),
        )
        .await;
        let mut items = r.flush().await;
        items.extend(r.drain());
        assert_eq!(
            statuses(&items)[1].2.as_deref(),
            Some("RS485 interface inactive")
        );
    }

    /// Finding 10: a refresh that finds somebody else's lock ends the session (fatal).
    #[tokio::test(start_paused = true)]
    async fn lock_taken_over_ends_the_session() {
        let mut r = rig(false);
        r.settle().await;
        r.drain();
        tokio::time::sleep(Duration::from_secs(181)).await;
        let items = r.drain();
        let refresh = items
            .iter()
            .find(|i| matches!(i.payload, OutboundPayload::LockRefresh { .. }))
            .expect("lock refresh after 3 min");
        r.results
            .send(OutboundResult {
                tag: refresh.tag,
                result: Err(crate::error::AppError::DeviceLocked {
                    user: "someone-else".into(),
                    since: now_iso(),
                }),
            })
            .unwrap();
        r.settle().await;
        assert!(r.emitter.events().iter().any(|e| matches!(e,
            SessionEvent::Error(x) if x.fatal && x.code == "deviceLocked")));
        // The shutdown drain (lock release, 5 s at most) comes first.
        tokio::time::sleep(Duration::from_secs(6)).await;
        let events = r.emitter.events();
        assert!(matches!(events.last(), Some(SessionEvent::Changed(None))));
        assert!(r.handle.is_finished());
    }

    /// Finding 36: a dead outbound lane ends the session instead of silently sending nothing.
    #[tokio::test(start_paused = true)]
    async fn closed_outbound_lane_is_fatal() {
        let mut r = rig(false);
        r.settle().await;
        r.drain();
        r.queue.close_lane_for_test(outbound::Lane::Normal);
        r.cmd(SessionCommand::SendAllNow).await.unwrap();
        r.settle().await;
        assert!(
            r.emitter
                .events()
                .iter()
                .any(|e| matches!(e, SessionEvent::Error(x) if x.fatal)),
        );
        tokio::time::sleep(Duration::from_secs(6)).await;
        assert!(matches!(
            r.emitter.events().last(),
            Some(SessionEvent::Changed(None))
        ));
        assert!(r.handle.is_finished());
    }

    /// Finding 2b: a panic in the state loop is reported at once.
    #[tokio::test(start_paused = true)]
    async fn session_panic_is_reported() {
        struct PanicOnFault(Arc<RecordingEmitter>);
        impl SessionEmitter for PanicOnFault {
            fn emit(&self, event: SessionEvent) {
                if matches!(event, SessionEvent::Fault(_)) {
                    panic!("boom (test)");
                }
                self.0.emit(event);
            }
        }
        let lib = fixture();
        let rec = RecordingEmitter::new();
        let r = rig_with_sink(
            prepared(&lib, false, false),
            Arc::new(PanicOnFault(rec.clone())),
            rec.clone(),
        );
        r.settle().await;
        let _ = r
            .cmd(SessionCommand::SetFault {
                target: FaultTarget::SlaveTimeout {
                    slave_key: "kuehlung".into(),
                },
                active: true,
            })
            .await;
        r.settle().await;
        let events = rec.events();
        assert!(events.iter().any(|e| matches!(e,
            SessionEvent::Error(x) if x.fatal && x.message.contains("crashed"))));
        assert!(matches!(events.last(), Some(SessionEvent::Changed(None))));
        assert!(r.handle.is_finished());
    }

    /// Finding 22: `SetupChanged` while a session runs.
    #[tokio::test(start_paused = true)]
    async fn setup_changed_takes_interval_and_generators() {
        let lib = fixture();
        let p = prepared(&lib, false, false);
        let mut demo = p.params.demo.clone();
        let mut r = rig_with(p);
        r.settle().await;
        r.drain();
        demo.send_interval_sec = 10;
        demo.generators.insert(
            "freezer_temp".into(),
            GeneratorConfig::Sine {
                period: 600.0,
                amplitude: 1.0,
                center: Some(-18.0),
            },
        );
        r.cmd(SessionCommand::SetupChanged {
            demo: Box::new(demo),
        })
        .await
        .unwrap();
        let snap = r.handle.snapshot();
        assert_eq!(snap.send_interval_sec, 10);
        assert!(matches!(
            snap.items
                .iter()
                .find(|i| i.item_id == "freezer_temp")
                .unwrap()
                .generator,
            Some(GeneratorConfig::Sine { .. })
        ));
        r.drain();
        tokio::time::sleep(Duration::from_secs(11)).await;
        assert!(!measurements(&r.drain()).is_empty(), "new interval is used");
    }

    /// Finding 39: new groups / inputs reach a running session.
    #[tokio::test(start_paused = true)]
    async fn config_changed_reaches_the_running_session() {
        let lib = fixture();
        let p = prepared(&lib, false, false);
        let mut groups: Vec<DaliGroup> = p.params.groups.clone();
        let mut inputs: Vec<DigitalInputEntry> = p.params.inputs.clone();
        let mut r = rig_with(p);
        r.settle().await;
        r.drain();
        r.cmd(SessionCommand::SetGroupMode {
            group_id: 1,
            mode: GroupMode::Off,
        })
        .await
        .unwrap();
        groups[0].name = "Laden".into();
        groups.push(DaliGroup {
            id: 5,
            name: "Neu".into(),
            converters: vec![4],
            min_level: 0,
            max_level: 100,
            lux_threshold: None,
            slave_of: None,
            dynamic_control: false,
            automatic_switch: false,
        });
        inputs
            .iter_mut()
            .find(|e| e.index == 3)
            .unwrap()
            .settings
            .unit = Some("kWh".into());
        r.cmd(SessionCommand::ConfigChanged {
            groups: Some(groups),
            inputs: Some(inputs),
        })
        .await
        .unwrap();
        let snap = r.handle.snapshot();
        assert_eq!(snap.groups.len(), 3);
        assert_eq!(snap.groups[0].name, "Laden");
        assert_eq!(snap.groups[0].mode, GroupMode::Off, "mode of a kept group");
        assert_eq!(snap.groups[2].mode, GroupMode::Auto);
        let counter = snap
            .items
            .iter()
            .find(|i| i.item_id == "gas-meter")
            .unwrap();
        assert_eq!(
            counter.naming.as_ref().unwrap().series[0].unit.as_deref(),
            Some("kWh")
        );
    }

    /// Finding 31: prompts also expire by the wall clock (a sleeping laptop stops the timers).
    #[test]
    fn prompt_expires_by_wall_clock() {
        let lib = fixture();
        let now = Instant::now();
        let mut core = SessionCore::new(prepared(&lib, false, false).params, now);
        core.start(now);
        core.finish();
        core.on_operation(&group_op("e1", 1, "on"), now, false);
        core.finish();
        assert_eq!(core.prompts.len(), 1);
        core.prompts[0].expires_wall = chrono::Utc::now() - chrono::Duration::seconds(1);
        // The monotonic deadline (60 s) is far away.
        core.timers(now + Duration::from_secs(1));
        assert!(core.prompts.is_empty());
        // FAILED "Timed out" waits for the answer to EXECUTING.
        let g = core.gated.get("e1").expect("final status is queued");
        assert_eq!(g.reason.as_deref(), Some("Timed out"));
    }

    fn started_core() -> SessionCore {
        let lib = fixture();
        let now = Instant::now();
        let mut core = SessionCore::new(prepared(&lib, false, false).params, now);
        core.start(now);
        core.finish();
        core
    }

    /// The last `session-changed` snapshot of a flush.
    fn changed_snapshot(f: &super::super::core::Flush) -> Option<crate::model::SessionSnapshot> {
        f.events.iter().rev().find_map(|e| match e {
            SessionEvent::Changed(Some(s)) => Some(s.clone()),
            _ => None,
        })
    }

    /// F3: `item-values-changed` carries no generator, so a generator change sends a snapshot.
    #[test]
    fn generator_change_refreshes_the_snapshot() {
        let mut core = started_core();
        let sine = GeneratorConfig::Sine {
            period: 600.0,
            amplitude: 1.0,
            center: Some(-18.0),
        };
        core.set_generator("freezer_temp", Some(sine)).unwrap();
        let snap = changed_snapshot(&core.finish()).expect("a snapshot follows");
        let item = snap.items.iter().find(|i| i.item_id == "freezer_temp");
        assert!(matches!(
            item.unwrap().generator,
            Some(GeneratorConfig::Sine { .. })
        ));
    }

    /// F4: a fault that pauses items refreshes `paused`; a sensor fault counts as paused and the
    /// sensor is sent again on clear.
    #[test]
    fn faults_refresh_paused_and_sensors_come_back() {
        let mut core = started_core();
        let paused = |s: &crate::model::SessionSnapshot, id: &str| {
            s.items.iter().find(|i| i.item_id == id).unwrap().paused
        };
        let timeout = FaultTarget::SlaveTimeout {
            slave_key: "kuehlung".into(),
        };
        core.set_fault(&timeout, true).unwrap();
        let snap = changed_snapshot(&core.finish()).expect("snapshot on raise");
        assert!(paused(&snap, "freezer_temp"));
        core.set_fault(&timeout, false).unwrap();
        let snap = changed_snapshot(&core.finish()).expect("snapshot on clear");
        assert!(!paused(&snap, "freezer_temp"));

        let alarm = crate::payload::alarm::sensor_alarms("brightness", 1)
            .unwrap()
            .into_iter()
            .find(|a| a.alarm_type.ends_with("_lost"))
            .unwrap()
            .alarm_type;
        let lost = FaultTarget::Sensor {
            sensor_kind: "brightness".into(),
            index: 1,
            alarm,
        };
        core.set_fault(&lost, true).unwrap();
        let f = core.finish();
        assert!(paused(&changed_snapshot(&f).unwrap(), "lux-1"));
        assert!(
            measurements(&f.outbound).is_empty(),
            "a lost sensor sends nothing"
        );
        core.set_fault(&lost, false).unwrap();
        let f = core.finish();
        assert!(!paused(&changed_snapshot(&f).unwrap(), "lux-1"));
        assert!(
            has_series(&measurements(&f.outbound), "LW Dali Brightness", "Lux1").is_some(),
            "the sensor is sent again"
        );
    }

    /// F7: a setup save touches only the generators whose persisted entry changed.
    #[test]
    fn setup_changed_keeps_unsaved_generator_overrides_of_other_items() {
        let now = Instant::now();
        let mut core = started_core();
        let sine = GeneratorConfig::Sine {
            period: 600.0,
            amplitude: 1.0,
            center: Some(-18.0),
        };
        // Session-only override (not persisted).
        core.set_generator("freezer_temp", Some(sine)).unwrap();
        core.finish();
        let mut demo = core.demo.clone();
        demo.send_interval_sec = 20;
        core.setup_changed(demo, now);
        let snap = changed_snapshot(&core.finish()).unwrap();
        let g = |id: &str| {
            snap.items
                .iter()
                .find(|i| i.item_id == id)
                .unwrap()
                .generator
                .clone()
        };
        assert!(matches!(
            g("freezer_temp"),
            Some(GeneratorConfig::Sine { .. })
        ));
        assert_eq!(snap.send_interval_sec, 20);
    }

    /// F11 / F14: edits the running session cannot take over are flagged.
    #[test]
    fn setup_edits_that_need_a_restart_are_flagged() {
        let now = Instant::now();
        let mut core = started_core();
        assert!(!core.snapshot().restart_needed);

        let mut demo = core.demo.clone();
        demo.excludes.push("freezer_temp".into());
        core.setup_changed(demo.clone(), now);
        assert!(core.snapshot().restart_needed, "item selection changed");
        demo.excludes.pop();
        core.setup_changed(demo, now);
        assert!(!core.snapshot().restart_needed, "reverted selection");

        // A digital input switched to an impulse counter: other series, restart needed.
        let mut inputs = core.inputs.clone();
        let e = inputs.iter_mut().find(|e| e.index == 1).unwrap();
        e.settings.input_type = crate::model::DigitalInputType::ImpulseCounter;
        e.settings.unit = Some("x".into());
        core.config_changed(None, Some(inputs));
        let snap = core.snapshot();
        assert!(snap.restart_needed);
        let di = snap
            .items
            .iter()
            .find(|i| i.item_id == "di-pump-1")
            .unwrap();
        assert_eq!(
            di.naming.as_ref().unwrap().series[0].name,
            "Value01",
            "the running item keeps its series"
        );
    }
}

mod review_fake {
    use super::fake::{device, op_status, start, wait_for};
    use super::*;
    use crate::c8y::fake::FakeC8y;
    use crate::payload::event::build_group_change;

    fn group_event(id: &str, group: u32, new: GroupMode, time: &str) -> Value {
        build_group_change(id, time, group, GroupMode::Auto, new, "cloud-operation")
    }

    /// Findings 1, 8, 9: a restart finds the group modes, the open alarms and the written
    /// registers where the last session left them.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn restart_restores_group_modes_alarms_and_written_registers() {
        let fake = FakeC8y::start().await;
        let lib = fixture();
        let id = device(&fake, &lib).await;
        {
            let mut s = fake.state();
            // Newest per group wins, whatever the insertion order.
            s.events.push(group_event(
                &id,
                2,
                GroupMode::On,
                "2026-10-12T10:03:00.000Z",
            ));
            s.events.push(group_event(
                &id,
                2,
                GroupMode::Off,
                "2026-10-12T10:01:00.000Z",
            ));
            s.events.push(group_event(
                &id,
                1,
                GroupMode::Auto,
                "2026-10-12T10:04:00.000Z",
            ));
            s.events.push(group_event(
                &id,
                1,
                GroupMode::On,
                "2026-10-12T10:02:00.000Z",
            ));
            // Open alarms of this device (and noise).
            s.alarms.push(json!({ "id": "A1", "source": { "id": id }, "type": "lw_lampFailure_1_01_openCircuit",
                "status": "ACTIVE", "time": "2026-10-12T10:00:00.000Z", "severity": "MAJOR", "text": "t" }));
            s.alarms.push(json!({ "id": "A2", "source": { "id": id }, "type": "lw_lampFailure_1_02_openCircuit",
                "status": "CLEARED", "time": "2026-10-12T10:00:00.000Z", "severity": "MAJOR", "text": "t" }));
            s.alarms.push(json!({ "id": "A3", "source": { "id": "999" }, "type": "lw_lampFailure_1_01_openCircuit",
                "status": "ACTIVE", "time": "2026-10-12T10:00:00.000Z", "severity": "MAJOR", "text": "t" }));
            // Realistic mode left this alarm ACTIVE after the firmware recovered it (F5).
            s.alarms.push(json!({ "id": "A4", "source": { "id": id }, "type": "lw_lampFailure_1_02_openCircuit",
                "status": "ACTIVE", "time": "2026-10-12T10:00:00.000Z", "severity": "MAJOR", "text": "t" }));
            s.events.push(json!({ "id": "E9", "source": { "id": id }, "type": "lw_alarm",
                "text": "Alarm cleared: lw_lampFailure_1_02_openCircuit", "time": "2026-10-12T10:02:00.000Z" }));
            // Written registers: the newest SUCCESSFUL write counts (a FAILED one does not).
            let write = |oid: &str, status: &str, value: u32, at: &str| {
                json!({
                "id": oid, "deviceId": id, "status": status, "creationTime": at,
                "lw_modbusWrite": { "slaveAddress": 239, "registerAddress": 40002,
                                    "value": value, "type": "register" } })
            };
            s.operations
                .push(write("8001", "SUCCESSFUL", 25, "2026-10-12T10:05:00.000Z"));
            s.operations
                .push(write("8002", "SUCCESSFUL", 22, "2026-10-12T10:01:00.000Z"));
            s.operations
                .push(write("8003", "FAILED", 30, "2026-10-12T10:06:00.000Z"));
        }
        let (h, _e) = start(&fake, &lib, &id).await;
        let snap = h.snapshot();
        assert_eq!(
            snap.groups
                .iter()
                .map(|g| (g.id, g.mode))
                .collect::<Vec<_>>(),
            vec![(1, GroupMode::Auto), (2, GroupMode::On)]
        );
        assert_eq!(
            snap.active_faults.len(),
            1,
            "A4 was recovered by the firmware (realistic mode): not restored"
        );
        assert_eq!(snap.active_faults[0].alarm_id.as_deref(), Some("A1"));
        let ac = snap
            .items
            .iter()
            .find(|i| i.item_id == "ac_setpoint")
            .unwrap();
        assert_eq!(ac.value, 25.0);
        assert!(
            ac.manual_override,
            "a restored value is not overwritten by a generator"
        );
        wait_for("lamp 1 reports its failure", || {
            fake.state()
                .measurements
                .iter()
                .any(|m| m["LW Dali Control Gear 1-01"]["Lamp Status"]["value"] == json!(0))
        })
        .await;
        wait_for("readback of the restored value", || {
            fake.state()
                .measurements
                .iter()
                .any(|m| m["LW_Modbus_Slave_19_239"]["ac_setpoint"]["value"] == json!(25))
        })
        .await;
        // The restored fault can be cleared (before: "not active", nothing could).
        h.send(SessionCommand::SetFault {
            target: FaultTarget::DaliLamp {
                lamp: 1,
                alarm_type: "lw_lampFailure_1_01_openCircuit".into(),
            },
            active: false,
        })
        .await
        .unwrap();
        wait_for("A1 cleared", || {
            fake.state()
                .alarms
                .iter()
                .any(|a| a["id"] == "A1" && a["status"] == "CLEARED")
        })
        .await;
        assert!(
            fake.state()
                .alarms
                .iter()
                .all(|a| a["id"] == "A1" || a["id"] != "A3" || a["status"] == "ACTIVE")
        );

        // The presenter sets group 2 to "On" again: same mode, but the event is sent.
        wait_for("realtime connected", || h.snapshot().realtime_connected).await;
        let op = fake.create_operation(
            &id,
            json!({ "lw_switchGroupMode": { "group": 2, "value": "on", "by": "x" } }),
        );
        let oid = op["id"].as_str().unwrap().to_owned();
        wait_for("prompt", || !h.snapshot().pending_operations.is_empty()).await;
        h.send(SessionCommand::AnswerOperation {
            operation_id: oid.clone(),
            decision: OperationDecision::Confirm,
        })
        .await
        .unwrap();
        wait_for("SUCCESSFUL", || op_status(&fake, &oid).0 == "SUCCESSFUL").await;
        assert!(fake.state().events.iter().any(|e| {
            e["groupModeChange"]["id"] == 2
                && e["groupModeChange"]["previousMode"] == "On"
                && e["groupModeChange"]["newMode"] == "On"
                && e["time"].as_str().is_some_and(|t| t > "2026-10-12T11")
        }));
        h.stop().await.unwrap();
    }

    /// A start scenario that is a full reset clears restored faults right away.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn start_scenario_clears_restored_faults() {
        let fake = FakeC8y::start().await;
        let lib = fixture();
        let id = device(&fake, &lib).await;
        fake.state().alarms.push(json!({
            "id": "A5", "source": { "id": id }, "type": "lw_modbus_20_timeoutError",
            "status": "ACTIVE", "time": "2026-10-12T10:00:00.000Z", "severity": "MAJOR", "text": "t" }));
        let (h, _e) = start(&fake, &lib, &id).await;
        assert!(h.snapshot().active_faults.is_empty());
        wait_for("A5 cleared by the start scenario", || {
            fake.state()
                .alarms
                .iter()
                .any(|a| a["id"] == "A5" && a["status"] == "CLEARED")
        })
        .await;
        h.stop().await.unwrap();
    }

    /// Finding 2c: EXECUTING visible operations from an earlier run are failed at the next start.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn leftover_executing_operations_are_failed_at_start() {
        let fake = FakeC8y::start().await;
        let lib = fixture();
        let id = device(&fake, &lib).await;
        let ago = |s: i64| {
            (chrono::Utc::now() - chrono::Duration::seconds(s))
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        };
        {
            let mut s = fake.state();
            let mut op = |oid: &str, status: &str, at: String, frag: Value| {
                let mut o = frag;
                o["id"] = json!(oid);
                o["deviceId"] = json!(id);
                o["status"] = json!(status);
                o["creationTime"] = json!(at);
                s.operations.push(o);
            };
            let write = json!({ "lw_modbusWrite": { "slaveAddress": 239, "registerAddress": 40002,
                                                    "value": 22, "type": "register" } });
            op("7001", "EXECUTING", ago(300), write.clone());
            op("7002", "EXECUTING", ago(5), write); // a prompt of somebody else, still young
            op("7003", "EXECUTING", ago(300), json!({ "lw_fetchCore": {} }));
            op(
                "7004",
                "EXECUTING",
                ago(300),
                json!({ "lw_switchGroupMode": { "group": 1, "value": "on" } }),
            );
        }
        let (h, _e) = start(&fake, &lib, &id).await;
        assert_eq!(
            op_status(&fake, "7001"),
            ("FAILED".into(), Some("Simulator was offline".into()))
        );
        assert_eq!(
            op_status(&fake, "7004"),
            ("FAILED".into(), Some("Simulator was offline".into()))
        );
        assert_eq!(op_status(&fake, "7002").0, "EXECUTING");
        assert_eq!(op_status(&fake, "7003").0, "EXECUTING");
        h.stop().await.unwrap();
    }

    /// Finding 6 / 22: EXECUTING is not held back by a hanging measurement `POST` or by another
    /// operation's slow status `PUT`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn executing_is_sent_concurrently_and_not_behind_measurements() {
        let fake = FakeC8y::start().await;
        let lib = fixture();
        let id = device(&fake, &lib).await;
        fake.delay("POST", "/measurement", Duration::from_millis(1500));
        let (h, _e) = start(&fake, &lib, &id).await;
        wait_for("realtime connected", || h.snapshot().realtime_connected).await;
        fake.delay(
            "PUT",
            "/devicecontrol/operations",
            Duration::from_millis(800),
        );
        let started = std::time::Instant::now();
        let mk = |g: u32| {
            fake.create_operation(
                &id,
                json!({ "lw_switchGroupMode": { "group": g, "value": "on" } }),
            )["id"]
                .as_str()
                .unwrap()
                .to_owned()
        };
        let (a, b) = (mk(1), mk(2));
        wait_for("both EXECUTING", || {
            op_status(&fake, &a).0 == "EXECUTING" && op_status(&fake, &b).0 == "EXECUTING"
        })
        .await;
        // One by one behind 1.5 s measurement POSTs (the first full send makes 5 of them) or
        // behind each other (0.8 s + 0.8 s) would take far longer.
        assert!(
            started.elapsed() < Duration::from_millis(1600),
            "{:?}",
            started.elapsed()
        );
        h.stop().await.unwrap();
    }

    /// Finding 22: a 401 in the middle of a session; stopping still completes (the drain does
    /// not wait for answers that can never succeed).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unauthorized_in_the_middle_of_a_session_stops_cleanly() {
        let fake = FakeC8y::start().await;
        let lib = fixture();
        let id = device(&fake, &lib).await;
        let (h, emitter) = start(&fake, &lib, &id).await;
        wait_for("first send", || fake.state().measurements.len() > 5).await;
        fake.state().password = "changed".into();
        h.send(SessionCommand::SendAllNow).await.unwrap();
        wait_for("auth-expired", || {
            !fake.sink.expired.lock().unwrap().is_empty()
        })
        .await;
        let stopped = tokio::time::timeout(Duration::from_secs(8), h.stop()).await;
        assert!(stopped.is_ok(), "stop must not hang after a 401");
        let _ = emitter;
    }
}

// ───────────────────────────── D9: fire once, clear in Cumulocity ─────────────────────────────

mod cloud_clear {
    use super::*;
    use crate::model::{ItemEffect, LibraryItem, OpenAlarmStatus, ScenarioStep, Severity};
    use tokio::time::Instant;

    fn fire(target: &FaultTarget) -> SessionCommand {
        SessionCommand::SetFault {
            target: target.clone(),
            active: true,
        }
    }

    fn bus() -> FaultTarget {
        FaultTarget::DaliBus {
            alarm_type: "lw_daliBusPowerFail".into(),
        }
    }

    fn timeout() -> FaultTarget {
        FaultTarget::SlaveTimeout {
            slave_key: "kuehlung".into(),
        }
    }

    fn door() -> FaultTarget {
        FaultTarget::CustomAlarm {
            item_id: "doorOpen".into(),
        }
    }

    /// Id the rig's `ack` gives the alarm POST of `alarm_type` in `items`.
    fn posted_id(items: &[OutboundItem], alarm_type: &str) -> String {
        items
            .iter()
            .find_map(|i| match &i.payload {
                OutboundPayload::Alarm(b) if b["type"] == alarm_type => Some(format!("a{}", i.tag)),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no POST of {alarm_type}"))
    }

    fn alarm_posts(items: &[OutboundItem]) -> Vec<String> {
        items
            .iter()
            .filter_map(|i| match &i.payload {
                OutboundPayload::Alarm(b) => b["type"].as_str().map(str::to_owned),
                _ => None,
            })
            .collect()
    }

    fn clear_ids(items: &[OutboundItem]) -> Vec<String> {
        items
            .iter()
            .filter_map(|i| match &i.payload {
                OutboundPayload::AlarmClear { alarm_id } => Some(alarm_id.clone()),
                _ => None,
            })
            .collect()
    }

    fn cleared_texts(items: &[OutboundItem]) -> Vec<String> {
        events_of(items)
            .iter()
            .filter_map(|e| e["text"].as_str().map(str::to_owned))
            .filter(|t| t.starts_with("Alarm cleared: "))
            .collect()
    }

    fn bulk_clears(items: &[OutboundItem]) -> usize {
        items
            .iter()
            .filter(|i| matches!(i.payload, OutboundPayload::ClearAlarms))
            .count()
    }

    fn paused(r: &Rig, item: &str) -> bool {
        r.handle
            .snapshot()
            .items
            .iter()
            .find(|i| i.item_id == item)
            .unwrap()
            .paused
    }

    fn status_of(r: &Rig, target: &FaultTarget) -> Option<OpenAlarmStatus> {
        r.handle
            .snapshot()
            .active_faults
            .iter()
            .find(|f| f.target == *target)
            .map(|f| f.status)
    }

    impl Rig {
        /// The presenter changes alarm `id` in Cumulocity (realtime `UPDATE`).
        async fn cloud(&self, id: &str, status: &str) {
            self.rt
                .send(RealtimeMsg::Alarm(
                    json!({ "id": id, "status": status, "type": "x" }),
                ))
                .await
                .unwrap();
            self.settle().await;
        }

        async fn send_all(&mut self) -> Vec<Value> {
            self.cmd(SessionCommand::SendAllNow).await.unwrap();
            measurements(&self.flush().await)
        }
    }

    /// Fire the bus power alarm: the cascade follows (D8). Acknowledging it in Cumulocity only
    /// changes the status; clearing it there ends the bus failure: the cascade is recovered by
    /// the simulator, the bus alarm itself is not touched again, the lamps send again.
    #[tokio::test(start_paused = true)]
    async fn bus_failure_lasts_until_its_alarm_is_cleared_in_cumulocity() {
        let mut r = rig(false);
        r.settle().await;
        r.drain();
        r.cmd(fire(&bus())).await.unwrap();
        let items = r.flush().await;
        assert_eq!(alarm_posts(&items).len(), 1 + 4 * 2 + 2);
        let bus_id = posted_id(&items, "lw_daliBusPowerFail");
        let lost1 = posted_id(&items, "lw_converter_1_01_lost");
        assert!(paused(&r, "light-1"));

        r.cloud(&bus_id, "ACKNOWLEDGED").await;
        assert_eq!(status_of(&r, &bus()), Some(OpenAlarmStatus::Acknowledged));
        let ms = r.send_all().await;
        assert!(has_series(&ms, "LW Dali Control Gear 1-01", "Brightness").is_none());
        assert!(
            paused(&r, "light-1"),
            "acknowledging does not end the effect"
        );

        r.cloud(&bus_id, "CLEARED").await;
        let items = r.flush().await;
        let texts = cleared_texts(&items);
        assert_eq!(texts[0], "Alarm cleared: lw_daliBusPowerFail");
        assert_eq!(texts.len(), 1 + 4 * 2 + 2, "{texts:?}");
        let ids = clear_ids(&items);
        assert_eq!(ids.len(), 4 * 2 + 2, "cascade only");
        assert!(!ids.contains(&bus_id), "the bus alarm is CLEARED already");
        assert!(ids.contains(&lost1));
        assert!(alarm_posts(&items).is_empty(), "nothing raised again");
        assert!(
            has_series(
                &measurements(&items),
                "LW Dali Control Gear 1-01",
                "Brightness"
            )
            .is_some()
        );
        assert!(r.handle.snapshot().active_faults.is_empty());
        assert!(!paused(&r, "light-1"));

        // The echo of the simulator's own clears changes nothing.
        r.cloud(&lost1, "CLEARED").await;
        let items = r.flush().await;
        assert!(cleared_texts(&items).is_empty() && clear_ids(&items).is_empty());
    }

    /// A cascade alarm the presenter clears while the bus is still down: the lamp stays lost and
    /// the alarm is not raised again (also not by the next sync); it recovers with the bus
    /// (event, no second clear).
    #[tokio::test(start_paused = true)]
    async fn cascade_alarm_cleared_in_cumulocity_stays_cleared_until_the_bus_recovers() {
        let mut r = rig(false);
        r.settle().await;
        r.drain();
        r.cmd(fire(&bus())).await.unwrap();
        let items = r.flush().await;
        let bus_id = posted_id(&items, "lw_daliBusPowerFail");
        let lost1 = posted_id(&items, "lw_converter_1_01_lost");
        r.cloud(&lost1, "CLEARED").await;
        assert!(r.flush().await.is_empty());
        assert!(paused(&r, "light-1"));
        r.cmd(SessionCommand::ConfigChanged {
            groups: Some(fixture().demo("fixture").unwrap().dali_groups.clone()),
            inputs: None,
        })
        .await
        .unwrap();
        assert!(alarm_posts(&r.flush().await).is_empty(), "not raised again");

        r.cloud(&bus_id, "CLEARED").await;
        let items = r.flush().await;
        assert!(
            cleared_texts(&items).contains(&"Alarm cleared: lw_converter_1_01_lost".to_owned())
        );
        let ids = clear_ids(&items);
        assert!(!ids.contains(&lost1), "cleared by the presenter already");
        assert_eq!(ids.len(), 4 * 2 + 2 - 1);
        assert!(!paused(&r, "light-1"));
    }

    /// A lamp's own fault cleared in Cumulocity while the bus holds the lamp: the fault ends,
    /// the lamp stays lost until the bus recovers.
    #[tokio::test(start_paused = true)]
    async fn own_lamp_fault_cleared_while_the_bus_holds_the_lamp() {
        let mut r = rig(false);
        r.settle().await;
        r.drain();
        let lost2 = FaultTarget::DaliLamp {
            lamp: 2,
            alarm_type: "lw_converter_1_02_lost".into(),
        };
        r.cmd(fire(&lost2)).await.unwrap();
        let items = r.flush().await;
        let lost2_id = posted_id(&items, "lw_converter_1_02_lost");
        r.cmd(fire(&bus())).await.unwrap();
        let items = r.flush().await;
        let bus_id = posted_id(&items, "lw_daliBusPowerFail");
        assert!(!alarm_posts(&items).contains(&"lw_converter_1_02_lost".to_owned()));

        r.cloud(&lost2_id, "CLEARED").await;
        let items = r.flush().await;
        assert!(
            cleared_texts(&items).is_empty(),
            "still lost: the bus is down"
        );
        assert!(alarm_posts(&items).is_empty());
        assert_eq!(r.handle.snapshot().active_faults.len(), 1, "only the bus");
        assert!(paused(&r, "light-2"));

        r.cloud(&bus_id, "CLEARED").await;
        let items = r.flush().await;
        assert!(
            cleared_texts(&items).contains(&"Alarm cleared: lw_converter_1_02_lost".to_owned())
        );
        assert!(!clear_ids(&items).contains(&lost2_id));
        assert!(
            has_series(
                &measurements(&items),
                "LW Dali Control Gear 1-02",
                "Brightness"
            )
            .is_some()
        );
    }

    /// A slave timeout stops the slave's values until the presenter clears the alarm.
    #[tokio::test(start_paused = true)]
    async fn slave_timeout_stops_values_until_cleared_in_cumulocity() {
        let mut r = rig(false);
        r.settle().await;
        r.drain();
        r.cmd(fire(&timeout())).await.unwrap();
        let items = r.flush().await;
        let id = posted_id(&items, "lw_modbus_20_timeoutError");
        tokio::time::sleep(Duration::from_secs(120)).await;
        let ms = r.send_all().await;
        assert!(has_series(&ms, "LW_Modbus_Slave_20_240", "freezer_temp").is_none());
        r.cloud(&id, "ACKNOWLEDGED").await;
        assert_eq!(
            status_of(&r, &timeout()),
            Some(OpenAlarmStatus::Acknowledged)
        );
        assert!(paused(&r, "freezer_temp"));

        r.cloud(&id, "CLEARED").await;
        let items = r.flush().await;
        assert_eq!(
            cleared_texts(&items),
            vec!["Alarm cleared: lw_modbus_20_timeoutError".to_owned()]
        );
        assert!(clear_ids(&items).is_empty(), "CLEARED by the presenter");
        assert!(
            has_series(
                &measurements(&items),
                "LW_Modbus_Slave_20_240",
                "freezer_temp"
            )
            .is_some()
        );
        assert!(!paused(&r, "freezer_temp"));
        assert!(r.handle.snapshot().active_faults.is_empty());
    }

    /// Firing an open fault again re-posts its alarm; an ACKNOWLEDGED alarm stays so.
    #[tokio::test(start_paused = true)]
    async fn firing_again_reposts_and_keeps_the_status() {
        let mut r = rig(false);
        r.settle().await;
        r.drain();
        r.cmd(fire(&door())).await.unwrap();
        let items = r.flush().await;
        let id = posted_id(&items, "lw_demo_doorOpen");
        r.cloud(&id, "ACKNOWLEDGED").await;
        r.cmd(fire(&door())).await.unwrap();
        r.settle().await;
        let items = r.drain();
        assert_eq!(alarm_posts(&items), vec!["lw_demo_doorOpen".to_owned()]);
        // Cumulocity de-duplicates: same id, count up, still ACKNOWLEDGED.
        for i in &items {
            let _ = r.results.send(OutboundResult {
                tag: i.tag,
                result: Ok(json!({ "id": id, "status": "ACKNOWLEDGED", "count": 2 })),
            });
        }
        r.settle().await;
        let snap = r.handle.snapshot();
        assert_eq!(snap.active_faults.len(), 1);
        assert_eq!(snap.active_faults[0].alarm_id.as_deref(), Some(id.as_str()));
        assert_eq!(snap.active_faults[0].status, OpenAlarmStatus::Acknowledged);
    }

    /// A clear that arrives before the alarm's create answer is not lost.
    #[tokio::test(start_paused = true)]
    async fn clear_before_the_create_answer_is_not_missed() {
        let mut r = rig(false);
        r.settle().await;
        r.drain();
        r.cmd(fire(&door())).await.unwrap();
        r.settle().await;
        let posts = r.drain();
        let id = posted_id(&posts, "lw_demo_doorOpen");
        r.cloud(&id, "CLEARED").await;
        assert_eq!(
            r.handle.snapshot().active_faults.len(),
            1,
            "id not known yet"
        );
        r.ack(&posts);
        r.settle().await;
        assert!(r.handle.snapshot().active_faults.is_empty());
    }

    /// Fired again while the presenter clears the alarm: Cumulocity creates a new alarm for the
    /// re-post, the clear of the old one must not end the fault (the answer decides).
    #[tokio::test(start_paused = true)]
    async fn firing_again_while_the_old_alarm_is_cleared_follows_the_new_alarm() {
        let mut r = rig(false);
        r.settle().await;
        r.drain();
        r.cmd(fire(&door())).await.unwrap();
        let items = r.flush().await;
        let old = posted_id(&items, "lw_demo_doorOpen");
        r.cmd(fire(&door())).await.unwrap();
        r.settle().await;
        let repost = r.drain();
        assert_eq!(alarm_posts(&repost).len(), 1);
        r.cloud(&old, "CLEARED").await;
        assert_eq!(r.handle.snapshot().active_faults.len(), 1, "answer pending");
        for i in &repost {
            r.results
                .send(OutboundResult {
                    tag: i.tag,
                    result: Ok(json!({ "id": "a9", "status": "ACTIVE" })),
                })
                .unwrap();
        }
        r.settle().await;
        let items = r.drain();
        assert!(cleared_texts(&items).is_empty(), "{items:?}");
        let snap = r.handle.snapshot();
        assert_eq!(snap.active_faults.len(), 1);
        assert_eq!(snap.active_faults[0].alarm_id.as_deref(), Some("a9"));

        // A re-sync that lacks the old id while another re-post is in flight changes nothing.
        r.cmd(fire(&door())).await.unwrap();
        r.settle().await;
        let repost = r.drain();
        r.rt.send(RealtimeMsg::OpenAlarms {
            alarms: vec![],
            fetched_at: Instant::now(),
        })
        .await
        .unwrap();
        r.settle().await;
        assert_eq!(r.handle.snapshot().active_faults.len(), 1);
        for i in &repost {
            r.results
                .send(OutboundResult {
                    tag: i.tag,
                    result: Ok(json!({ "id": "a9", "status": "ACTIVE", "count": 2 })),
                })
                .unwrap();
        }
        r.settle().await;

        r.cloud("a9", "CLEARED").await;
        let items = r.flush().await;
        assert_eq!(
            cleared_texts(&items),
            vec!["Alarm cleared: lw_demo_doorOpen".to_owned()]
        );
        assert!(r.handle.snapshot().active_faults.is_empty());
    }

    /// An ACKNOWLEDGED that arrives before the create answer is applied by the answer.
    #[tokio::test(start_paused = true)]
    async fn acknowledge_before_the_create_answer_is_not_lost() {
        let mut r = rig(false);
        r.settle().await;
        r.drain();
        r.cmd(fire(&door())).await.unwrap();
        r.settle().await;
        let posts = r.drain();
        let id = posted_id(&posts, "lw_demo_doorOpen");
        r.cloud(&id, "ACKNOWLEDGED").await;
        r.ack(&posts);
        r.settle().await;
        assert_eq!(status_of(&r, &door()), Some(OpenAlarmStatus::Acknowledged));
    }

    fn with_threshold_alarm(mut p: Prepared) -> Prepared {
        for it in &mut p.params.resolved.items {
            if let LibraryItem::Modbus { id, effects, .. } = it
                && id == "ac_setpoint"
            {
                effects.push(ItemEffect::Alarm {
                    alarm_type: "lw_demo_acHot".into(),
                    severity: Severity::Major,
                    text: "Klima zu warm".into(),
                    threshold: 25.0,
                });
            }
        }
        p
    }

    /// "Clear all" ends every effect (faults, bus cascade) and clears every open alarm of the
    /// device in one go (the bulk update also catches alarms the simulator never raised); a
    /// threshold alarm whose value is still above is not raised again until it went below.
    #[tokio::test(start_paused = true)]
    async fn clear_all_ends_every_effect() {
        let mut r = rig_with(with_threshold_alarm(prepared(&fixture(), false, false)));
        r.settle().await;
        r.drain();
        for t in [bus(), timeout(), door()] {
            r.cmd(fire(&t)).await.unwrap();
        }
        r.cmd(SessionCommand::SetValue {
            item_id: "ac_setpoint".into(),
            value: 26.0,
        })
        .await
        .unwrap();
        let items = r.flush().await;
        assert!(alarm_posts(&items).contains(&"lw_demo_acHot".to_owned()));
        assert_eq!(r.handle.snapshot().active_faults.len(), 3);

        r.cmd(SessionCommand::ClearAllAlarms).await.unwrap();
        let items = r.flush().await;
        assert_eq!(bulk_clears(&items), 1);
        assert!(
            clear_ids(&items).is_empty(),
            "the bulk update clears everything"
        );
        let texts = cleared_texts(&items);
        for t in [
            "lw_daliBusPowerFail",
            "lw_modbus_20_timeoutError",
            "lw_demo_doorOpen",
        ] {
            assert!(texts.contains(&format!("Alarm cleared: {t}")), "{texts:?}");
        }
        assert!(texts.contains(&"Alarm cleared: lw_converter_1_01_lost".to_owned()));
        assert!(alarm_posts(&items).is_empty());
        let ms = measurements(&items);
        assert!(has_series(&ms, "LW Dali Control Gear 1-01", "Brightness").is_some());
        assert!(has_series(&ms, "LW_Modbus_Slave_20_240", "freezer_temp").is_some());
        assert!(r.handle.snapshot().active_faults.is_empty());
        assert!(!paused(&r, "light-1") && !paused(&r, "freezer_temp"));

        // Still above the threshold: not raised again …
        r.cmd(SessionCommand::SetValue {
            item_id: "ac_setpoint".into(),
            value: 27.0,
        })
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert!(alarm_posts(&r.flush().await).is_empty());
        // … until it went below once.
        for v in [22.0, 26.0] {
            r.cmd(SessionCommand::SetValue {
                item_id: "ac_setpoint".into(),
                value: v,
            })
            .await
            .unwrap();
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        assert_eq!(
            alarm_posts(&r.flush().await),
            vec!["lw_demo_acHot".to_owned()]
        );
    }

    /// "Clear all" fails (no alarm admin role, timeout): the session reports it, reads the open
    /// alarms and takes the ones that are still open over again.
    #[tokio::test(start_paused = true)]
    async fn failed_clear_all_restores_the_alarms_still_open() {
        let mut r = rig(false);
        r.settle().await;
        r.drain();
        r.cmd(fire(&door())).await.unwrap();
        r.cmd(fire(&timeout())).await.unwrap();
        let items = r.flush().await;
        let door_id = posted_id(&items, "lw_demo_doorOpen");
        assert_eq!(r.handle.snapshot().active_faults.len(), 2);
        let errors = |r: &Rig| {
            r.emitter
                .events()
                .iter()
                .filter(|e| matches!(e, SessionEvent::Error(_)))
                .count()
        };
        let before = errors(&r);

        r.cmd(SessionCommand::ClearAllAlarms).await.unwrap();
        r.settle().await;
        let items = r.drain();
        assert!(r.handle.snapshot().active_faults.is_empty());
        let clear = items
            .iter()
            .find(|i| matches!(i.payload, OutboundPayload::ClearAlarms))
            .unwrap();
        r.results
            .send(OutboundResult {
                tag: clear.tag,
                result: Err(crate::error::AppError::Http {
                    status: 403,
                    message: "forbidden".into(),
                }),
            })
            .unwrap();
        r.settle().await;
        assert_eq!(errors(&r), before + 1);
        let items = r.drain();
        let read = items
            .iter()
            .find(|i| matches!(i.payload, OutboundPayload::ReadOpenAlarms))
            .expect("the open alarms are read");
        // Only the door alarm is still open (the other PUT got through).
        r.results
            .send(OutboundResult {
                tag: read.tag,
                result: Ok(json!({ "alarms": [
                    { "id": door_id, "type": "lw_demo_doorOpen", "status": "ACKNOWLEDGED" }
                ] })),
            })
            .unwrap();
        r.settle().await;
        let snap = r.handle.snapshot();
        assert_eq!(snap.active_faults.len(), 1);
        assert_eq!(snap.active_faults[0].target, door());
        assert_eq!(snap.active_faults[0].status, OpenAlarmStatus::Acknowledged);
        // It ends like any open alarm when the presenter clears it.
        r.cloud(&door_id, "CLEARED").await;
        assert!(r.handle.snapshot().active_faults.is_empty());
    }

    /// A threshold alarm cleared in Cumulocity (realtime, not "Clear all") stays down until the
    /// value has been below the threshold once.
    #[tokio::test(start_paused = true)]
    async fn threshold_alarm_cleared_in_cumulocity_is_not_raised_again_until_below() {
        let mut r = rig_with(with_threshold_alarm(prepared(&fixture(), false, false)));
        r.settle().await;
        r.drain();
        let set = |v: f64| SessionCommand::SetValue {
            item_id: "ac_setpoint".into(),
            value: v,
        };
        r.cmd(set(26.0)).await.unwrap();
        let items = r.flush().await;
        let id = posted_id(&items, "lw_demo_acHot");
        r.cloud(&id, "CLEARED").await;
        assert!(r.flush().await.is_empty() || cleared_texts(&r.flush().await).is_empty());
        r.cmd(set(27.0)).await.unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert!(alarm_posts(&r.flush().await).is_empty());
        for v in [22.0, 26.0] {
            r.cmd(set(v)).await.unwrap();
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        assert_eq!(
            alarm_posts(&r.flush().await),
            vec!["lw_demo_acHot".to_owned()]
        );
    }

    /// A scenario step `active: false` clears the alarm in Cumulocity and ends the effect; the
    /// start scenario applied by hand is "Clear all".
    #[tokio::test(start_paused = true)]
    async fn scenario_clears_and_the_start_scenario_clears_all() {
        let mut p = prepared(&fixture(), false, false);
        p.params.scenarios.push(crate::model::Scenario {
            id: "fix".into(),
            name: crate::model::LocalizedText {
                de: "Repariert".into(),
                en: "Fixed".into(),
            },
            start: false,
            set: vec![ScenarioStep::Fault {
                target: timeout(),
                active: false,
            }],
        });
        let mut r = rig_with(p);
        r.settle().await;
        r.drain();
        r.cmd(SessionCommand::ApplyScenario {
            scenario_id: "trouble".into(),
        })
        .await
        .unwrap();
        let items = r.flush().await;
        let timeout_id = posted_id(&items, "lw_modbus_20_timeoutError");
        assert_eq!(r.handle.snapshot().active_faults.len(), 4);

        r.cmd(SessionCommand::ApplyScenario {
            scenario_id: "fix".into(),
        })
        .await
        .unwrap();
        let items = r.flush().await;
        assert_eq!(clear_ids(&items), vec![timeout_id]);
        assert_eq!(
            cleared_texts(&items),
            vec!["Alarm cleared: lw_modbus_20_timeoutError".to_owned()]
        );
        assert!(
            has_series(
                &measurements(&items),
                "LW_Modbus_Slave_20_240",
                "freezer_temp"
            )
            .is_some()
        );
        assert_eq!(r.handle.snapshot().active_faults.len(), 3);

        r.cmd(SessionCommand::ApplyScenario {
            scenario_id: "normal".into(),
        })
        .await
        .unwrap();
        let items = r.flush().await;
        assert_eq!(bulk_clears(&items), 1, "the start scenario clears all");
        assert!(r.handle.snapshot().active_faults.is_empty());
    }

    /// A clear made while realtime was down is found in the open-alarm list of the reconnect;
    /// an alarm created after the list was fetched is not mistaken for a cleared one; statuses
    /// are taken over.
    #[tokio::test(start_paused = true)]
    async fn reconnect_finds_a_clear_made_while_disconnected() {
        let mut r = rig(false);
        r.settle().await;
        r.drain();
        r.cmd(fire(&timeout())).await.unwrap();
        r.cmd(fire(&bus())).await.unwrap();
        let items = r.flush().await;
        let bus_id = posted_id(&items, "lw_daliBusPowerFail");
        r.rt.send(RealtimeMsg::Disconnected("down".into()))
            .await
            .unwrap();
        r.settle().await;
        assert!(!r.handle.snapshot().realtime_connected);

        // Fired while the list is being fetched: its id arrives after `fetched_at`.
        r.cmd(fire(&door())).await.unwrap();
        r.settle().await;
        let posts = r.drain();
        let fetched_at = Instant::now();
        tokio::time::sleep(Duration::from_millis(5)).await;
        r.ack(&posts);
        r.settle().await;

        r.rt.send(RealtimeMsg::Connected).await.unwrap();
        // The timeout alarm was cleared meanwhile; the bus alarm was acknowledged.
        r.rt.send(RealtimeMsg::OpenAlarms {
            alarms: vec![json!({ "id": bus_id, "status": "ACKNOWLEDGED" })],
            fetched_at,
        })
        .await
        .unwrap();
        let items = r.flush().await;
        assert!(r.handle.snapshot().realtime_connected);
        assert_eq!(
            cleared_texts(&items),
            vec!["Alarm cleared: lw_modbus_20_timeoutError".to_owned()]
        );
        assert!(clear_ids(&items).is_empty());
        assert!(!paused(&r, "freezer_temp"));
        assert_eq!(status_of(&r, &timeout()), None);
        assert_eq!(status_of(&r, &bus()), Some(OpenAlarmStatus::Acknowledged));
        assert_eq!(
            status_of(&r, &door()),
            Some(OpenAlarmStatus::Active),
            "too new"
        );
        assert!(paused(&r, "light-1"), "the bus is still down");
    }

    /// Alarms open at session start come back as effects with their status.
    #[tokio::test(start_paused = true)]
    async fn restored_alarms_keep_their_status() {
        let mut p = prepared(&fixture(), false, false);
        p.params.scenarios.retain(|s| !s.start);
        p.params.restored_alarms = vec![super::super::core::RestoredAlarm {
            alarm_type: "lw_modbus_20_timeoutError".into(),
            id: Some("T1".into()),
            raised_at: now_iso(),
            status: OpenAlarmStatus::Acknowledged,
        }];
        let mut r = rig_with(p);
        r.settle().await;
        r.drain();
        assert_eq!(
            status_of(&r, &timeout()),
            Some(OpenAlarmStatus::Acknowledged)
        );
        assert!(paused(&r, "freezer_temp"));
        r.cloud("T1", "CLEARED").await;
        r.flush().await;
        assert!(!paused(&r, "freezer_temp"));
    }

    mod fake {
        use super::super::fake::{device, start, wait_for};
        use super::*;
        use crate::c8y::fake::FakeC8y;

        fn open(fake: &FakeC8y, dev: &str) -> usize {
            fake.state()
                .alarms
                .iter()
                .filter(|a| {
                    a["source"]["id"] == dev
                        && matches!(a["status"].as_str(), Some("ACTIVE" | "ACKNOWLEDGED"))
                })
                .count()
        }

        /// End to end: the presenter acknowledges and clears in Cumulocity, "Clear all" clears
        /// also an alarm the simulator never raised, and a clear made while realtime was down is
        /// found after the reconnect.
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn presenter_acks_and_clears_in_cumulocity() {
            let fake = FakeC8y::start().await;
            let lib = fixture();
            let id = device(&fake, &lib).await;
            let (h, _e) = start(&fake, &lib, &id).await;
            wait_for("realtime connected", || h.snapshot().realtime_connected).await;

            h.send(fire(&timeout())).await.unwrap();
            wait_for("alarm id", || {
                h.snapshot()
                    .active_faults
                    .first()
                    .is_some_and(|f| f.alarm_id.is_some())
            })
            .await;
            let alarm = fake.alarm_id("lw_modbus_20_timeoutError").unwrap();
            fake.set_alarm_status(&alarm, "ACKNOWLEDGED", true);
            wait_for("acknowledged", || {
                h.snapshot()
                    .active_faults
                    .first()
                    .is_some_and(|f| f.status == OpenAlarmStatus::Acknowledged)
            })
            .await;
            assert!(
                h.snapshot()
                    .items
                    .iter()
                    .any(|i| i.item_id == "freezer_temp" && i.paused)
            );
            fake.set_alarm_status(&alarm, "CLEARED", true);
            wait_for("fault over", || h.snapshot().active_faults.is_empty()).await;
            wait_for("recovery event", || {
                fake.state()
                    .events
                    .iter()
                    .any(|e| e["text"] == "Alarm cleared: lw_modbus_20_timeoutError")
            })
            .await;
            assert!(h.snapshot().items.iter().all(|i| !i.paused));

            // Clear all: a fault, the bus cascade and a raw alarm of somebody else.
            fake.client()
                .create_alarm(
                    &json!({ "type": "c8y_Foreign", "text": "x", "severity": "MINOR",
                                       "source": { "id": id } }),
                )
                .await
                .unwrap();
            // A deleted alarm (realtime DELETE) ends its fault like a cleared one.
            h.send(fire(&door())).await.unwrap();
            wait_for("door alarm id", || {
                h.snapshot()
                    .active_faults
                    .first()
                    .is_some_and(|f| f.alarm_id.is_some())
            })
            .await;
            fake.delete_alarm(&h.snapshot().active_faults[0].alarm_id.clone().unwrap());
            wait_for("deleted alarm ends the fault", || {
                h.snapshot().active_faults.is_empty()
            })
            .await;
            h.send(fire(&bus())).await.unwrap();
            h.send(fire(&door())).await.unwrap();
            wait_for("alarms open", || open(&fake, &id) == 1 + 1 + 4 * 2 + 2 + 1).await;
            h.send(SessionCommand::ClearAllAlarms).await.unwrap();
            wait_for("all cleared", || open(&fake, &id) == 0).await;
            assert!(h.snapshot().active_faults.is_empty());
            tokio::time::sleep(Duration::from_millis(300)).await;
            assert_eq!(open(&fake, &id), 0, "nothing raised again");

            // Cleared while realtime is down: found by the reconnect.
            h.send(fire(&door())).await.unwrap();
            wait_for("door alarm id", || {
                h.snapshot()
                    .active_faults
                    .first()
                    .is_some_and(|f| f.alarm_id.is_some())
            })
            .await;
            let door_alarm = h.snapshot().active_faults[0].alarm_id.clone().unwrap();
            fake.set_alarm_status(&door_alarm, "CLEARED", false);
            tokio::time::sleep(Duration::from_millis(200)).await;
            assert_eq!(
                h.snapshot().active_faults.len(),
                1,
                "nobody told the session"
            );
            fake.drop_realtime_clients();
            wait_for("re-sync", || h.snapshot().active_faults.is_empty()).await;
            h.stop().await.unwrap();
        }
    }
}
