//! Session handle, start-up and the task loop (ARCHITECTURE §8).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tauri::AppHandle;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::Instant;

use super::core::{Flush, RestoredAlarm, SessionCore, SessionParams, Source};
use super::emit::{SessionEmitter, SessionEvent, TauriEmitter, error_code};
use super::outbound::{self, OutboundWorker};
use crate::c8y::C8yClient;
use crate::c8y::realtime::{RealtimeMsg, RealtimeSubscription, subscribe_device};
use crate::device::{crud, demo_state, setup};
use crate::error::{AppError, AppResult};
use crate::events::SessionError;
use crate::library::Library;
use crate::model::{
    DaliGroup, DigitalInputEntry, FaultTarget, GeneratorConfig, GroupMode, LwDemoState,
    OpenAlarmStatus, OperationDecision, OperationStatus, SessionSnapshot, StartSessionRequest,
};

/// Messages from commands to the session task. Every message carries a reply
/// channel so commands can return task-side errors.
#[derive(Debug)]
pub enum SessionCommand {
    SetValue {
        item_id: String,
        value: f64,
    },
    ResumeGenerator {
        item_id: String,
    },
    SetGenerator {
        item_id: String,
        generator: Option<GeneratorConfig>,
    },
    ApplyScenario {
        scenario_id: String,
    },
    SetGroupMode {
        group_id: u32,
        mode: GroupMode,
    },
    /// Fire (`active`) a fault, or clear its alarm in Cumulocity (`!active`; scenario-step
    /// semantics, D9). The `fire_fault` command sends `active: true`.
    SetFault {
        target: FaultTarget,
        active: bool,
    },
    /// "Clear all": end every effect and clear every open alarm of the device (D9).
    ClearAllAlarms,
    /// "Reset to defaults": "Clear all", then every item back to its default value and its
    /// effective generator (counters keep their value).
    ResetToDefaults,
    SetSendInterval {
        seconds: u32,
    },
    SetAutoAccept {
        enabled: bool,
    },
    SendAllNow,
    SaveSnapshot,
    AnswerOperation {
        operation_id: String,
        decision: OperationDecision,
    },
    /// `update_demo_setup` wrote a new `lw_demo` for this device.
    SetupChanged {
        demo: Box<LwDemoState>,
    },
    /// `set_dali_groups` / `set_digital_inputs` wrote new config fragments for this device
    /// (`None` = that fragment did not change).
    ConfigChanged {
        groups: Option<Vec<DaliGroup>>,
        inputs: Option<Vec<DigitalInputEntry>>,
    },
    /// Graceful stop: fail open prompts, release the lock, cancel realtime, end the task.
    Stop,
}

type Envelope = (SessionCommand, oneshot::Sender<AppResult<()>>);

/// How long a stopping session waits for queued payloads (lock release, final statuses).
const DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

/// Cloneable sender to the session task. Commands clone it under the `AppState::session`
/// lock, drop the guard, and only then `send` — so a slow task reply never blocks other
/// commands (or `start` / `stop`) on the lock.
#[derive(Debug, Clone)]
pub struct SessionSender {
    tx: mpsc::Sender<Envelope>,
}

impl SessionSender {
    /// Send a command and wait for the task's reply. `NoSession` if the task has ended.
    pub async fn send(&self, cmd: SessionCommand) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send((cmd, reply_tx))
            .await
            .map_err(|_| AppError::NoSession)?;
        reply_rx.await.map_err(|_| AppError::NoSession)?
    }
}

/// Handle to the running session, stored in `AppState::session`.
pub struct SessionHandle {
    pub device_id: String,
    sender: SessionSender,
    snapshot: watch::Receiver<SessionSnapshot>,
    task: tauri::async_runtime::JoinHandle<()>,
}

/// Everything resolved before the task starts (I/O done by [`prepare`]).
pub struct Prepared {
    pub params: SessionParams,
}

/// Newest stored value of a series (`revert=true&pageSize=1`), if any.
async fn newest_value(
    client: &C8yClient,
    device_id: &str,
    fragment: &str,
    series: &str,
) -> Option<f64> {
    let path = crate::c8y::api::with_query(
        "/measurement/measurements",
        &[
            ("source", device_id.to_owned()),
            ("valueFragmentType", fragment.to_owned()),
            ("valueFragmentSeries", series.to_owned()),
            ("dateFrom", "1970-01-01T00:00:00Z".to_owned()),
            ("revert", "true".to_owned()),
            ("pageSize", "1".to_owned()),
        ],
    );
    let body: Value = client.get_json(&path).await.ok()?;
    body.pointer("/measurements/0")?
        .get(fragment)?
        .get(series)?
        .get("value")?
        .as_f64()
}

/// How many of the start-up lookups run at the same time.
const SEED_PARALLELISM: usize = 8;

/// Newest stored value per counter item, looked up concurrently (`(item id, fragment, series)`).
async fn counter_seeds(
    client: &C8yClient,
    device_id: &str,
    queries: Vec<(String, String, String)>,
) -> BTreeMap<String, f64> {
    let mut seeds = BTreeMap::new();
    for chunk in queries.chunks(SEED_PARALLELISM) {
        let mut set = tokio::task::JoinSet::new();
        for (id, fragment, series) in chunk.iter().cloned() {
            let (client, device_id) = (client.clone(), device_id.to_owned());
            set.spawn(async move {
                (
                    id,
                    newest_value(&client, &device_id, &fragment, &series).await,
                )
            });
        }
        while let Some(Ok((id, value))) = set.join_next().await {
            if let Some(v) = value {
                seeds.insert(id, v);
            }
        }
    }
    seeds
}

/// Mode of every DALI group per the newest `lw_daliGroupChange` event of that group. The plugin
/// reads the mode from there (`floorplan-plugin.md` §5), so the session starts in the same mode.
async fn group_modes(client: &C8yClient, device_id: &str) -> BTreeMap<u32, GroupMode> {
    let mut modes = BTreeMap::new();
    let events = match client
        .query_events(device_id, Some("lw_daliGroupChange"), 100)
        .await
    {
        Ok(e) => e,
        Err(e) => {
            log::warn!("group modes could not be restored: {e}");
            return modes;
        }
    };
    // Newest first: the first event of a group counts.
    for e in events {
        let change = &e["groupModeChange"];
        let Some(id) = change["id"].as_u64().and_then(|n| u32::try_from(n).ok()) else {
            continue;
        };
        let mode = match change["newMode"]
            .as_str()
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("on") => GroupMode::On,
            Some("off") => GroupMode::Off,
            Some("auto") => GroupMode::Auto,
            _ => continue,
        };
        modes.entry(id).or_insert(mode);
    }
    modes
}

/// Alarms of the device that are still ACTIVE / ACKNOWLEDGED, minus the ones the firmware
/// already recovered (realistic mode leaves a recovered alarm ACTIVE, but sends the `lw_alarm`
/// "Alarm cleared: <type>" event). Restoring those would re-apply their side effects.
pub(super) async fn open_alarms(client: &C8yClient, device_id: &str) -> Vec<RestoredAlarm> {
    let alarms: Vec<RestoredAlarm> = match client.open_alarms(device_id, None).await {
        Ok(alarms) => alarms
            .iter()
            .filter_map(|a| {
                Some(RestoredAlarm {
                    alarm_type: a.get("type")?.as_str()?.to_owned(),
                    id: crate::c8y::id_of(a).ok(),
                    raised_at: a
                        .get("time")
                        .or_else(|| a.get("creationTime"))
                        .and_then(Value::as_str)
                        .map_or_else(crate::c8y::now_iso, str::to_owned),
                    status: a
                        .get("status")
                        .and_then(Value::as_str)
                        .and_then(OpenAlarmStatus::parse)
                        .unwrap_or_default(),
                })
            })
            .collect(),
        Err(e) => {
            log::warn!("open alarms could not be restored: {e}");
            return Vec::new();
        }
    };
    if alarms.is_empty() {
        return alarms;
    }
    // Every "Alarm cleared" event since the oldest open alarm: a window by time, so a busy
    // recovery (one bus failure writes ~46 events) cannot push older ones out.
    let oldest = alarms
        .iter()
        .filter_map(|a| parse_time(&a.raised_at))
        .min()
        .map_or_else(
            || crate::c8y::api::EPOCH.to_owned(),
            |t| t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        );
    match client
        .query_events_since(device_id, "lw_alarm", &oldest)
        .await
    {
        Ok(events) => drop_recovered(alarms, &events),
        Err(e) => {
            log::warn!("alarm recoveries could not be read: {e}");
            alarms
        }
    }
}

/// Newest `lw_alarm` "Alarm cleared: <type>" time per alarm type.
fn cleared_times(events: &[Value]) -> BTreeMap<String, chrono::DateTime<chrono::Utc>> {
    let mut cleared = BTreeMap::new();
    for e in events {
        let Some(alarm_type) = e
            .get("text")
            .and_then(Value::as_str)
            .and_then(|t| t.strip_prefix("Alarm cleared: "))
        else {
            continue;
        };
        let Some(at) = e.get("time").and_then(Value::as_str).and_then(parse_time) else {
            continue;
        };
        let slot = cleared.entry(alarm_type.trim().to_owned()).or_insert(at);
        if at > *slot {
            *slot = at;
        }
    }
    cleared
}

fn parse_time(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|t| t.with_timezone(&chrono::Utc))
}

/// Drop the alarms whose newest "cleared" event is later than the alarm itself.
fn drop_recovered(alarms: Vec<RestoredAlarm>, events: &[Value]) -> Vec<RestoredAlarm> {
    let cleared = cleared_times(events);
    alarms
        .into_iter()
        .filter(|a| {
            let recovered = cleared
                .get(&a.alarm_type)
                .zip(parse_time(&a.raised_at))
                .is_some_and(|(cleared_at, raised)| *cleared_at > raised);
            !recovered
        })
        .collect()
}

/// Value of the newest SUCCESSFUL `lw_modbusWrite` per writable register: the plugin shows it as
/// the control state and compares the readback with it, so the session sends exactly that.
async fn write_seeds(
    client: &C8yClient,
    device_id: &str,
    resolved: &setup::ResolvedSetup,
) -> BTreeMap<String, f64> {
    let mut seeds = BTreeMap::new();
    let ops = match client
        .newest_operations(
            device_id,
            OperationStatus::Successful,
            "lw_modbusWrite",
            100,
        )
        .await
    {
        Ok(o) => o,
        Err(e) => {
            log::warn!("modbus writes could not be restored: {e}");
            return seeds;
        }
    };
    // `<type>:<slave>:<register>` → value, newest first.
    let mut newest: BTreeMap<String, f64> = BTreeMap::new();
    for op in &ops {
        let w = &op["lw_modbusWrite"];
        let num = |k: &str| {
            w.get(k).and_then(|v| {
                v.as_u64()
                    .or_else(|| v.as_f64().map(|f| f as u64))
                    .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
            })
        };
        let (Some(slave), Some(register), Some(kind)) = (
            num("slaveAddress"),
            num("registerAddress"),
            w.get("type").and_then(Value::as_str),
        ) else {
            continue;
        };
        let Some(value) = w.get("value").and_then(|v| {
            v.as_f64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        }) else {
            continue;
        };
        newest
            .entry(format!("{kind}:{slave}:{register}"))
            .or_insert(value);
    }
    for (_, a) in &resolved.slaves {
        for r in &a.registers {
            let Some(kind) = r.register_type.write_operation_type() else {
                continue;
            };
            let writable = resolved.item(&r.item_id).is_some_and(|i| {
                matches!(i, crate::model::LibraryItem::Modbus { writable: true, .. })
            });
            if writable && let Some(v) = newest.get(&format!("{kind}:{}:{}", a.address, r.address))
            {
                seeds.insert(r.item_id.clone(), *v);
            }
        }
    }
    seeds
}

/// A visible operation that is still EXECUTING and older than the prompt timeout can only be a
/// leftover of an earlier run (a crash or a quit): the plugin has no timeout for EXECUTING, so
/// its control would spin forever. Fail them.
async fn fail_leftover_operations(client: &C8yClient, device_id: &str) {
    let ops = match client
        .query_operations(device_id, Some(OperationStatus::Executing))
        .await
    {
        Ok(o) => o,
        Err(e) => {
            log::warn!("leftover operations could not be checked: {e}");
            return;
        }
    };
    let now = crate::c8y::server_now(&client.clock_skew());
    let limit = chrono::Duration::from_std(super::operations::PROMPT_TIMEOUT).unwrap_or_default();
    let mut set = tokio::task::JoinSet::new();
    for op in ops {
        let fragment = super::operations::fragment_of(&op);
        let old = op
            .get("creationTime")
            .and_then(Value::as_str)
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
            .is_some_and(|t| now - t.with_timezone(&chrono::Utc) > limit);
        let Ok(id) = crate::c8y::id_of(&op) else {
            continue;
        };
        if old && super::operations::VISIBLE.contains(&fragment.as_str()) {
            let client = client.clone();
            set.spawn(async move {
                client
                    .set_operation_status(
                        &id,
                        OperationStatus::Failed,
                        Some(super::operations::REASON_OFFLINE),
                    )
                    .await
            });
        }
    }
    while let Some(res) = set.join_next().await {
        if let Ok(Err(e)) = res {
            log::warn!("leftover operation could not be failed: {e}");
        }
    }
}

/// Load the device, check the lock, resolve the setup, restore what the tenant already knows
/// (group modes, open alarms, counters, written registers), fail leftover operations, and take
/// the lock as the very last step (a failed start leaves nothing behind).
pub async fn prepare(
    client: &C8yClient,
    library: &Library,
    req: &StartSessionRequest,
) -> AppResult<Prepared> {
    let mo = crud::load_demo_mo(client, &req.device_id).await?;
    let demo = demo_state::read(&mo)?
        .ok_or_else(|| AppError::NotFound(format!("lw_demo on {}", req.device_id)))?;
    let me = client.username().to_owned();
    demo_state::check_lock(&demo, &me, req.force, chrono::Utc::now())?;

    // Everything that can fail on the setup comes before anything is written.
    let resolved = setup::resolve(library, &demo)?;
    let config = setup::device_config(library, &demo);
    let scenarios = setup::library_scenarios(library, &demo);

    // Counters never go backwards: seed them from the newest stored measurement.
    let mut counter_queries = Vec::new();
    for item in &resolved.items {
        let counter = matches!(item, crate::model::LibraryItem::ImpulseCounter { .. })
            || matches!(item.control(), Some(crate::model::Control::Counter { .. }))
            || matches!(
                demo.generators.get(item.id()).or(item.generator()),
                Some(GeneratorConfig::Counter { .. })
            );
        if !counter {
            continue;
        }
        let slave = item
            .slave_key()
            .and_then(|k| resolved.slaves.iter().find(|(d, _)| d.key == k))
            .map(|(_, a)| a);
        let di = item
            .index()
            .and_then(|i| config.digital_inputs.iter().find(|e| e.index == i))
            .filter(|_| matches!(item, crate::model::LibraryItem::ImpulseCounter { .. }));
        if let Some(n) = crate::payload::naming::item_naming_with_di(item, slave, di)
            && let Some(s) = n.series.first()
        {
            counter_queries.push((item.id().to_owned(), n.fragment.clone(), s.name.clone()));
        }
    }
    let (counter_seeds, group_modes, restored_alarms, write_seeds, ()) = tokio::join!(
        counter_seeds(client, &req.device_id, counter_queries),
        group_modes(client, &req.device_id),
        open_alarms(client, &req.device_id),
        write_seeds(client, &req.device_id, &resolved),
        fail_leftover_operations(client, &req.device_id),
    );

    // Operations reach a device only when it is its own agent: migrate older demo devices.
    let mut mo = mo;
    crud::ensure_required_fragments(client, &mut mo).await?;
    demo_state::write_lock(client, &req.device_id, &me).await?;

    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(7);
    Ok(Prepared {
        params: SessionParams {
            device_id: req.device_id.clone(),
            device_name: mo
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            user: me,
            time_zone: demo
                .time_zone
                .as_deref()
                .filter(|z| !z.is_empty())
                .unwrap_or(super::generators::DEFAULT_TIME_ZONE)
                .to_owned(),
            demo,
            resolved,
            groups: config.groups,
            inputs: config.digital_inputs,
            scenarios,
            auto_accept: req.auto_accept,
            counter_seeds,
            seed,
            clock_skew: client.clock_skew(),
            group_modes,
            restored_alarms,
            write_seeds,
            // No `lw_configRs485` any more (D10): the simulated bus is always active.
            rs485_active: true,
        },
    })
}

/// Spawn the task around prepared parameters and explicit I/O parts (tests pass doubles).
pub fn spawn_with(
    prepared: Prepared,
    realtime: Option<RealtimeSubscription>,
    outbound: OutboundWorker,
    emitter: Arc<dyn SessionEmitter>,
) -> SessionHandle {
    let now = Instant::now();
    let mut core = SessionCore::new(prepared.params, now);
    core.start(now);
    let device_id = core.device_id().to_owned();
    let snapshot = core.snapshot();
    let (watch_tx, watch_rx) = watch::channel(snapshot);
    let (tx, rx) = mpsc::channel::<Envelope>(32);
    let inner = tokio::spawn(run(core, rx, realtime, outbound, emitter.clone(), watch_tx));
    // A panic in the state loop must not leave the UI showing a dead session: the watcher
    // reports it at once (`session-error` fatal + `session-changed(null)`). Whatever the loop
    // had open (a lock, EXECUTING operations) is cleaned up by the next start (`prepare`).
    let task = tokio::spawn(async move {
        if let Err(e) = inner.await
            && e.is_panic()
        {
            emitter.emit(SessionEvent::Error(SessionError {
                code: "internal".into(),
                message: "the simulation session crashed".into(),
                fatal: true,
            }));
            emitter.emit(SessionEvent::Changed(None));
        }
    });
    SessionHandle {
        device_id,
        sender: SessionSender { tx },
        snapshot: watch_rx,
        task: tauri::async_runtime::JoinHandle::Tokio(task),
    }
}

impl SessionHandle {
    /// Load the device + `lw_demo`, check the advisory lock, restore what Cumulocity knows
    /// (group modes, open alarms, counters, written registers), fail leftover operations, take
    /// the lock, subscribe to realtime (stale PENDING operations are failed as they arrive),
    /// apply the start scenario and spawn the task.
    ///
    /// The task (see ARCHITECTURE §8) owns the live state, talks to Cumulocity only through
    /// [`super::outbound`], refreshes the lock every `device::demo_state::LOCK_REFRESH_INTERVAL`,
    /// and on any exit that was not requested by [`SessionCommand::Stop`] (a fatal error, a
    /// panic) emits `session-error { fatal }` and `session-changed(null)` so the UI never keeps
    /// showing a dead session.
    pub async fn start(
        app: AppHandle,
        client: C8yClient,
        library: Arc<Library>,
        req: StartSessionRequest,
    ) -> AppResult<Self> {
        let emitter: Arc<dyn SessionEmitter> = Arc::new(TauriEmitter(app));
        Self::start_with(client, &library, req, emitter).await
    }

    /// [`Self::start`] with an explicit emitter (tests).
    pub async fn start_with(
        client: C8yClient,
        library: &Library,
        req: StartSessionRequest,
        emitter: Arc<dyn SessionEmitter>,
    ) -> AppResult<Self> {
        let prepared = prepare(&client, library, &req).await?;
        let realtime = match subscribe_device(&client, &req.device_id) {
            Ok(r) => r,
            Err(e) => {
                // Do not leave our lock behind.
                let _ = demo_state::clear_lock(&client, &req.device_id).await;
                return Err(e);
            }
        };
        let outbound = outbound::spawn(client, req.device_id.clone());
        Ok(spawn_with(prepared, Some(realtime), outbound, emitter))
    }

    /// Wire a handle around an already spawned task.
    pub fn spawn_parts(
        device_id: String,
        tx: mpsc::Sender<Envelope>,
        snapshot: watch::Receiver<SessionSnapshot>,
        task: tauri::async_runtime::JoinHandle<()>,
    ) -> Self {
        Self {
            device_id,
            sender: SessionSender { tx },
            snapshot,
            task,
        }
    }

    /// A cloneable sender (clone under the lock, send after releasing it).
    pub fn sender(&self) -> SessionSender {
        self.sender.clone()
    }

    /// Send a command and wait for the task's reply.
    pub async fn send(&self, cmd: SessionCommand) -> AppResult<()> {
        self.sender.send(cmd).await
    }

    /// The task has ended (crashed, fatal error, or stopped). A finished handle must be
    /// treated as "no session" (`AppState` prunes it).
    pub fn is_finished(&self) -> bool {
        self.task.inner().is_finished()
    }

    /// Latest snapshot published by the task.
    pub fn snapshot(&self) -> SessionSnapshot {
        self.snapshot.borrow().clone()
    }

    /// Stop gracefully and wait for the task to finish. A task that is already gone is fine:
    /// a closed channel is not an error here.
    pub async fn stop(self) -> AppResult<()> {
        let res = match self.sender.send(SessionCommand::Stop).await {
            Err(AppError::NoSession) => Ok(()),
            other => other,
        };
        let _ = self.task.await;
        res
    }
}

/// Next realtime message, or pending forever when there is no subscription.
async fn recv_realtime(rt: &mut Option<RealtimeSubscription>) -> Option<RealtimeMsg> {
    match rt {
        Some(r) => r.recv().await,
        None => std::future::pending().await,
    }
}

fn publish(
    flush: Flush,
    core: &mut SessionCore,
    outbound: &OutboundWorker,
    emitter: &Arc<dyn SessionEmitter>,
    watch_tx: &watch::Sender<SessionSnapshot>,
) {
    let now = Instant::now();
    if flush.supersede_periodic {
        // A newer full send replaces the older queued one.
        let stale = outbound.queue.supersede_periodic();
        core.outbound_dropped(&stale, false, now);
    }
    for item in flush.outbound {
        let tag = item.tag;
        match outbound.queue.enqueue(item) {
            Ok(dropped) => core.outbound_dropped(&dropped, true, now),
            Err(e) => core.enqueue_failed(tag, e, now),
        }
    }
    if let Some(s) = flush.snapshot {
        let _ = watch_tx.send(s);
    }
    for e in flush.events {
        emitter.emit(e);
    }
}

/// Handle one command.
fn command(core: &mut SessionCore, cmd: SessionCommand, now: Instant) -> AppResult<()> {
    match cmd {
        SessionCommand::SetValue { item_id, value } => {
            core.set_checked(&item_id, value, Source::Manual)
        }
        SessionCommand::ResumeGenerator { item_id } => core.resume_generator(&item_id),
        SessionCommand::SetGenerator { item_id, generator } => {
            core.set_generator(&item_id, generator)
        }
        SessionCommand::ApplyScenario { scenario_id } => core.apply_scenario(&scenario_id, now),
        SessionCommand::SetGroupMode { group_id, mode } => core.set_group_mode(
            group_id,
            mode,
            crate::payload::event::REASON_CLOUD_OPERATION,
            false,
        ),
        SessionCommand::SetFault { target, active } => core.set_fault(&target, active),
        SessionCommand::ClearAllAlarms => {
            core.clear_all_alarms();
            Ok(())
        }
        SessionCommand::ResetToDefaults => {
            core.reset_to_defaults();
            Ok(())
        }
        SessionCommand::SetSendInterval { seconds } => core.set_send_interval(seconds, now),
        SessionCommand::SetAutoAccept { enabled } => {
            core.set_auto_accept(enabled);
            Ok(())
        }
        SessionCommand::SendAllNow => {
            core.send_all_now();
            Ok(())
        }
        SessionCommand::SaveSnapshot => core.save_snapshot(),
        SessionCommand::AnswerOperation {
            operation_id,
            decision,
        } => core.answer_operation(&operation_id, &decision),
        SessionCommand::SetupChanged { demo } => {
            core.setup_changed(*demo, now);
            Ok(())
        }
        SessionCommand::ConfigChanged { groups, inputs } => {
            core.config_changed(groups, inputs);
            Ok(())
        }
        SessionCommand::Stop => Ok(()),
    }
}

/// The state loop: owns all live state, never awaits HTTP.
async fn run(
    mut core: SessionCore,
    mut rx: mpsc::Receiver<Envelope>,
    mut realtime: Option<RealtimeSubscription>,
    mut outbound: OutboundWorker,
    emitter: Arc<dyn SessionEmitter>,
    watch_tx: watch::Sender<SessionSnapshot>,
) {
    // Initial outputs (start scenario, first full send).
    let flush = core.finish();
    publish(flush, &mut core, &outbound, &emitter, &watch_tx);

    let mut stop_reply: Option<oneshot::Sender<AppResult<()>>> = None;
    let mut fatal: Option<AppError> = None;
    loop {
        let deadline = core.next_deadline();
        tokio::select! {
            msg = rx.recv() => match msg {
                Some((SessionCommand::Stop, reply)) => {
                    stop_reply = Some(reply);
                    break;
                }
                Some((cmd, reply)) => {
                    let res = command(&mut core, cmd, Instant::now());
                    let _ = reply.send(res);
                }
                None => break,
            },
            msg = recv_realtime(&mut realtime) => match msg {
                Some(RealtimeMsg::Connected) => core.realtime_status(true, None),
                Some(RealtimeMsg::Disconnected(e)) => core.realtime_status(false, Some(e)),
                Some(RealtimeMsg::Operation(op)) => core.on_operation(&op, Instant::now(), false),
                Some(RealtimeMsg::PendingOperation(op)) => {
                    core.on_operation(&op, Instant::now(), true);
                }
                Some(RealtimeMsg::Alarm(alarm)) => core.on_alarm(&alarm),
                Some(RealtimeMsg::OpenAlarms { alarms, fetched_at }) => {
                    core.reconcile_open_alarms(&alarms, fetched_at);
                }
                None => {
                    realtime = None;
                    core.realtime_status(false, Some("realtime subscription ended".into()));
                }
            },
            Some(res) = outbound.results.recv() => core.outbound_result(res, Instant::now()),
            () = tokio::time::sleep_until(deadline) => core.timers(Instant::now()),
        }
        let flush = core.finish();
        publish(flush, &mut core, &outbound, &emitter, &watch_tx);
        if let Some(e) = core.take_fatal() {
            fatal = Some(e);
            break;
        }
    }

    // ── shutdown ──
    drop(realtime);
    if stop_reply.is_none()
        && let Some(e) = &fatal
    {
        // The loop ended on its own: say why right away (the drain below can take seconds).
        emitter.emit(SessionEvent::Error(SessionError {
            code: error_code(e),
            message: e.to_string(),
            fatal: true,
        }));
    }
    core.set_stopping();
    let _ = watch_tx.send(core.snapshot());
    core.fail_open_prompts();
    core.release_lock();
    // The lock release and the final statuses must not wait behind data nobody needs.
    let stale = outbound.queue.clear_measurements();
    core.outbound_dropped(&stale, false, Instant::now());
    let flush = core.finish();
    publish(flush, &mut core, &outbound, &emitter, &watch_tx);
    let until = Instant::now() + DRAIN_TIMEOUT;
    while core.outstanding() > 0 {
        match tokio::time::timeout_at(until, outbound.results.recv()).await {
            Ok(Some(res)) => {
                core.outbound_result(res, Instant::now());
                // A final status may have been waiting for this answer.
                let flush = core.finish();
                publish(flush, &mut core, &outbound, &emitter, &watch_tx);
            }
            _ => break,
        }
    }
    drop(outbound);
    if stop_reply.is_none() {
        // The loop ended on its own: the session is gone.
        emitter.emit(SessionEvent::Changed(None));
    }
    if let Some(reply) = stop_reply {
        let _ = reply.send(Ok(()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn alarm(t: &str, at: &str) -> RestoredAlarm {
        RestoredAlarm {
            alarm_type: t.into(),
            id: Some("1".into()),
            raised_at: at.into(),
            status: OpenAlarmStatus::Active,
        }
    }

    #[test]
    fn recovered_alarms_are_not_restored() {
        let events = [
            json!({ "type": "lw_alarm", "text": "Alarm cleared: lw_a", "time": "2026-10-12T10:02:00.000Z" }),
            // Older than the second raise of lw_b: that alarm is a new one.
            json!({ "type": "lw_alarm", "text": "Alarm cleared: lw_b", "time": "2026-10-12T09:00:00.000Z" }),
            json!({ "type": "lw_alarm", "text": "something else", "time": "2026-10-12T11:00:00.000Z" }),
        ];
        let kept = drop_recovered(
            vec![
                alarm("lw_a", "2026-10-12T10:00:00.000Z"),
                alarm("lw_b", "2026-10-12T10:00:00.000Z"),
                alarm("lw_c", "2026-10-12T10:00:00.000Z"),
                // Raised again after the recovery.
                alarm("lw_a", "2026-10-12T10:05:00.000Z"),
            ],
            &events,
        );
        let kept: Vec<_> = kept
            .iter()
            .map(|a| (&*a.alarm_type, &*a.raised_at))
            .collect();
        assert_eq!(
            kept,
            vec![
                ("lw_b", "2026-10-12T10:00:00.000Z"),
                ("lw_c", "2026-10-12T10:00:00.000Z"),
                ("lw_a", "2026-10-12T10:05:00.000Z"),
            ]
        );
    }
}
