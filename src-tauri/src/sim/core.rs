//! Live state of a session and everything that mutates it — without I/O.
//!
//! [`SessionCore`] is owned by the session task (`session.rs`). Every input (command, realtime
//! message, outbound result, timer) is a method call that mutates the state and queues outputs;
//! [`SessionCore::finish`] then turns the iteration's changes into outbound payloads (tagged),
//! events and, when something observable changed, a new snapshot with a bumped `revision`.
//! Operation handling lives in `operations.rs`, faults in `faults.rs`, scenarios in
//! `scenario.rs` (further `impl SessionCore` blocks).

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::AtomicI64;

use serde_json::{Map, Value, json};
use tokio::time::Instant;

use super::emit::{SessionEvent, error_code};
use super::generators::{
    DEFAULT_TIME_ZONE, GeneratorState, SeededRng, Tick, counter_seed, local_hour,
    local_hour_in_zone, next_value,
};
use super::operations::{OpenPrompt, OperationTracker};
use super::outbound::{Class, LANE_DEAD_TAG, Lane, OutboundItem, OutboundPayload, OutboundResult};
use super::scheduler::{MAX_SEND_INTERVAL_SEC, MIN_SEND_INTERVAL_SEC, SEND_DEBOUNCE, Scheduler};
use crate::c8y::now_iso;
use crate::device::demo_state::{self, LOCK_REFRESH_INTERVAL};
use crate::device::setup::ResolvedSetup;
use crate::error::{AppError, AppResult};
use crate::events::{ItemValuesChanged, RealtimeStatus, SessionError};
use crate::model::{
    ActiveFault, Control, DaliGroup, DaliGroupState, DemoSnapshot, DigitalInputEntry,
    DigitalInputType, GeneratorConfig, GroupMode, ItemEffect, ItemRuntimeState, ItemValue,
    LibraryItem, LwDemoState, MeasurementNaming, OpenAlarmStatus, Scenario, SessionSnapshot,
    SessionStatus, SlaveAssignment, SlaveDefinition,
};
use crate::payload::alarm::{AlarmSpec, build_alarm};
use crate::payload::event::{
    REASON_CLOUD_OPERATION, build_alarm_cleared, build_event, build_group_change,
};
use crate::payload::measurement::{
    CounterGrid, DaliLampStatus, MeasurementValue, build_measurement_bodies, dali_lamp_values,
    item_values, item_values_with_grid,
};
use crate::payload::naming::item_naming_with_di;

/// Why a change happened (decides manual override, immediate send and effects).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Manual,
    Scenario,
    Operation,
    /// Group mode / converter switch: like manual, but no event / inventory effects.
    Group,
    Generator,
}

/// One item of the session.
#[derive(Debug, Clone)]
pub struct LiveItem {
    pub def: LibraryItem,
    pub naming: Option<MeasurementNaming>,
    pub value: f64,
    /// Start / fallback value (control default).
    pub base: f64,
    pub generator: Option<GeneratorConfig>,
    pub gen_state: GeneratorState,
    pub manual: bool,
}

impl LiveItem {
    /// Counter items never go backwards (impulse counters, counter controls / generators).
    pub fn is_counter(&self) -> bool {
        matches!(self.def, LibraryItem::ImpulseCounter { .. })
            || matches!(self.def.control(), Some(Control::Counter { .. }))
            || matches!(self.generator, Some(GeneratorConfig::Counter { .. }))
    }
}

/// A DALI lamp (also lamps without an item: every lamp of the device in a group is sent).
#[derive(Debug, Clone, Default)]
pub struct Lamp {
    /// Brightness of a lamp without an item.
    pub brightness: f64,
    pub converter_ok: bool,
    pub lamp_ok: bool,
    /// `lw_converter_1_NN_lost` / `notExisting`: the firmware stops sending the lamp.
    pub lost: bool,
    pub runtime_secs: f64,
}

/// A Cumulocity alarm raised by the session (fault, bus cascade or alarm effect), by type.
#[derive(Debug, Clone, Default)]
pub struct AlarmTrack {
    pub id: Option<String>,
    /// Open in Cumulocity as far as the session knows.
    pub active: bool,
    /// Cleared before its create returned an id: clear as soon as it does.
    pub clear_pending: bool,
    /// Create `POST`s not answered yet. While one is in flight the session does not know which
    /// Cumulocity alarm it will get (a re-post after a clear creates a new one), so a clear for
    /// the known id is only recorded and the answer decides.
    pub inflight: u32,
    /// Cleared in Cumulocity (by the presenter or "Clear all") while its cause is still there
    /// (a bus failure holding a lamp, a value above an alarm threshold): not raised again until
    /// the cause has gone away once (D9).
    pub suppressed: bool,
    /// When the session learned `id` (an open-alarm list fetched later that misses it means
    /// the alarm was cleared meanwhile).
    pub id_at: Option<Instant>,
    /// Status in Cumulocity while open.
    pub status: OpenAlarmStatus,
}

/// An alarm that was still open in Cumulocity when the session started.
#[derive(Debug, Clone)]
pub struct RestoredAlarm {
    pub alarm_type: String,
    pub id: Option<String>,
    pub raised_at: String,
    pub status: OpenAlarmStatus,
}

/// How many cleared alarm ids the session remembers (a clear that arrives before the alarm's
/// create answer, see [`SessionCore::outbound_result`]).
pub(super) const RECENT_CLEARED: usize = 256;

/// Bookkeeping for an outbound result.
#[derive(Debug, Clone, PartialEq)]
pub enum Purpose {
    Measurements {
        periodic: bool,
    },
    Alarm {
        alarm_type: String,
    },
    AlarmClear,
    /// "Clear all": every open alarm of the device.
    ClearAll,
    /// Which alarms are still open after a failed "Clear all".
    ReadOpenAlarms,
    Event,
    /// `executing`: the EXECUTING status (the final one waits for its result).
    OperationStatus {
        operation_id: String,
        executing: bool,
    },
    Inventory,
    LockRefresh,
    LockRelease,
}

/// A payload queued during one loop iteration.
#[derive(Debug)]
pub(super) struct Queued {
    pub payload: OutboundPayload,
    pub purpose: Purpose,
    /// `None`: the payload's default lane.
    pub lane: Option<Lane>,
}

/// A final operation status that waits until EXECUTING has been answered (so the two can never
/// arrive in the wrong order).
#[derive(Debug)]
pub(super) struct GatedFinal {
    pub status: crate::model::OperationStatus,
    pub reason: Option<String>,
}

/// Everything a session starts from (resolved by `session::start`).
#[derive(Debug, Clone)]
pub struct SessionParams {
    pub device_id: String,
    pub device_name: String,
    /// Lock owner (the logged-in user).
    pub user: String,
    pub demo: LwDemoState,
    pub resolved: ResolvedSetup,
    pub groups: Vec<DaliGroup>,
    pub inputs: Vec<DigitalInputEntry>,
    pub scenarios: Vec<Scenario>,
    pub auto_accept: bool,
    /// Newest stored value per counter item (counters never go backwards across restarts).
    pub counter_seeds: BTreeMap<String, f64>,
    /// `lw_demo.timeZone` of the device (`dailyProfile` runs in device time).
    pub time_zone: String,
    pub seed: u64,
    /// Server clock minus local clock (ms), measured by the client from `Date` headers.
    pub clock_skew: Arc<AtomicI64>,
    /// Mode of each DALI group as the newest `lw_daliGroupChange` event says (groups without an
    /// event are `Auto`).
    pub group_modes: BTreeMap<u32, GroupMode>,
    /// Alarms of the device that are still open in Cumulocity.
    pub restored_alarms: Vec<RestoredAlarm>,
    /// Newest SUCCESSFUL `lw_modbusWrite` value per writable item (the plugin shows it).
    pub write_seeds: BTreeMap<String, f64>,
    /// RS485 bus active (a write fails with "RS485 interface inactive" otherwise). Always `true`
    /// for a session (D10: no `lw_configRs485`); tests switch it off.
    pub rs485_active: bool,
}

impl Default for SessionParams {
    fn default() -> Self {
        Self {
            device_id: String::new(),
            device_name: String::new(),
            user: String::new(),
            demo: demo_state::empty_state(),
            resolved: ResolvedSetup::default(),
            groups: Vec::new(),
            inputs: Vec::new(),
            scenarios: Vec::new(),
            auto_accept: false,
            counter_seeds: BTreeMap::new(),
            time_zone: DEFAULT_TIME_ZONE.to_owned(),
            seed: 1,
            clock_skew: Arc::default(),
            group_modes: BTreeMap::new(),
            restored_alarms: Vec::new(),
            write_seeds: BTreeMap::new(),
            rs485_active: true,
        }
    }
}

/// Output of one loop iteration.
#[derive(Debug, Default)]
pub struct Flush {
    pub outbound: Vec<OutboundItem>,
    pub events: Vec<SessionEvent>,
    /// New snapshot for `get_session` (the watch channel), when anything changed.
    pub snapshot: Option<SessionSnapshot>,
    /// A periodic full send is in `outbound`: queued older periodic sends are stale.
    pub supersede_periodic: bool,
}

/// The session state.
pub struct SessionCore {
    pub(super) device_id: String,
    pub(super) device_name: String,
    pub(super) user: String,
    pub(super) started_at: String,
    pub(super) status: SessionStatus,
    pub(super) demo: LwDemoState,
    pub(super) items: Vec<LiveItem>,
    pub(super) slaves: Vec<(SlaveDefinition, SlaveAssignment)>,
    pub(super) groups: Vec<(DaliGroup, GroupMode)>,
    pub(super) inputs: Vec<DigitalInputEntry>,
    pub(super) lamps: BTreeMap<u32, Lamp>,
    pub(super) scenarios: Vec<Scenario>,
    pub(super) faults: Vec<ActiveFault>,
    pub(super) alarms: BTreeMap<String, AlarmTrack>,
    /// Alarm types held by a DALI bus power fault (lamp / sensor lost, see
    /// `faults::sync_bus_cascade`), not by a fault of their own.
    pub(super) bus_cascade: BTreeSet<String>,
    pub(super) timed_out: BTreeSet<String>,
    /// Alarm ids recently seen CLEARED in Cumulocity (newest last, at most [`RECENT_CLEARED`]).
    pub(super) recent_cleared: VecDeque<String>,
    /// The alarms the last "Clear all" ended locally: restored when its clear failed.
    /// Open statuses seen for ids the session does not know (yet): an ACKNOWLEDGED that arrives
    /// before the create answer is applied by it (newest last, at most [`RECENT_CLEARED`]).
    pub(super) recent_status: VecDeque<(String, OpenAlarmStatus)>,
    pub(super) cleared_by_all: Vec<RestoredAlarm>,
    pub(super) ops: OperationTracker,
    pub(super) prompts: Vec<OpenPrompt>,
    pub(super) auto_accept: bool,
    pub(super) realtime_connected: bool,
    pub(super) last_sent_at: Option<String>,
    pub(super) scheduler: Scheduler,
    pub(super) rng: SeededRng,
    pub(super) last_gen_tick: Instant,
    pub(super) revision: u32,
    pub(super) time_zone: String,
    pub(super) clock_skew: Arc<AtomicI64>,
    pub(super) rs485_active: bool,
    /// What the item selection was made from when the session started (see [`selection_key`]).
    selection_key: String,
    /// A digital input changed in a way that needs a restart (series name).
    config_restart: bool,
    group_modes: BTreeMap<u32, GroupMode>,
    write_seeds: BTreeMap<String, f64>,
    /// Manual changes waiting for [`SEND_DEBOUNCE`].
    debounced: BTreeSet<usize>,
    debounce_due: Option<Instant>,
    /// Operations whose EXECUTING `PUT` has not been answered.
    pub(super) exec_inflight: BTreeSet<String>,
    /// Final statuses waiting for the EXECUTING answer, by operation id.
    pub(super) gated: HashMap<String, GatedFinal>,
    /// Wall clock of the last lock refresh (a sleeping laptop stops the monotonic timers).
    last_lock_wall: chrono::DateTime<chrono::Utc>,
    /// Set when the session cannot go on (dead outbound lane, lock taken over).
    fatal: Option<AppError>,

    // ── per-iteration outputs ──
    pub(super) out: Vec<Queued>,
    pub(super) events: Vec<SessionEvent>,
    pub(super) ui_changed: BTreeSet<usize>,
    pub(super) send_items: BTreeSet<usize>,
    pub(super) send_lamps: BTreeSet<u32>,
    pub(super) send_all: Option<bool>,
    pub(super) supersede: bool,
    pub(super) dirty: bool,
    pub(super) emit_changed: bool,
    pub(super) next_tag: u32,
    pub(super) purposes: HashMap<u32, Purpose>,
    pub(super) last_error: Option<(String, Instant)>,
}

/// The part of the device setup that decides which items and slaves exist. A session keeps the
/// selection it started with; a change shows up as `restart_needed`.
fn selection_key(demo: &LwDemoState) -> String {
    serde_json::to_string(&(
        &demo.tags,
        &demo.includes,
        &demo.excludes,
        demo.dali_lamp_count,
        &demo.custom_slaves,
        &demo.custom_items,
    ))
    .unwrap_or_default()
}

/// Type and series names of a naming (what decides how a value is handled; a changed fragment,
/// i.e. a new title, D10, or unit is taken over by a running session).
fn series_names(n: &Option<MeasurementNaming>) -> Option<(&str, Vec<&str>)> {
    n.as_ref().map(|n| {
        (
            n.measurement_type.as_str(),
            n.series.iter().map(|s| s.name.as_str()).collect(),
        )
    })
}

/// Start / fallback value of an item: control default, else 100 for a lamp, else 0.
pub fn default_value(item: &LibraryItem) -> f64 {
    match item.control() {
        Some(
            Control::Slider { default, .. }
            | Control::Switch { default }
            | Control::Select { default, .. }
            | Control::Counter { default },
        ) => *default,
        None => match item {
            LibraryItem::Dali { .. } => 100.0,
            _ => 0.0,
        },
    }
}

/// Whether `value` fits the item's control (slider range, switch 0/1, select option, counter ≥ 0).
pub fn check_value(item: &LibraryItem, value: f64) -> AppResult<()> {
    let bad = |m: String| Err(AppError::InvalidInput(format!("{}: {m}", item.id())));
    if !value.is_finite() {
        return bad("value must be a finite number".into());
    }
    match item.control() {
        Some(Control::Slider { min, max, .. }) if value < *min || value > *max => {
            bad(format!("value {value} outside {min}…{max}"))
        }
        Some(Control::Switch { .. }) if value != 0.0 && value != 1.0 => {
            bad(format!("value {value} must be 0 or 1"))
        }
        Some(Control::Select { options, .. }) if !options.iter().any(|o| o.value == value) => {
            bad(format!("value {value} is not an option value"))
        }
        Some(Control::Counter { .. }) if value < 0.0 => bad("counter value must be ≥ 0".into()),
        _ => Ok(()),
    }
}

/// Naming of an item: Modbus items follow their slave's assignment, digital inputs the device's
/// digital input entry (type, unit and title, D10).
fn naming_of(
    def: &LibraryItem,
    slaves: &[(SlaveDefinition, SlaveAssignment)],
    inputs: &[DigitalInputEntry],
) -> Option<MeasurementNaming> {
    let slave = def
        .slave_key()
        .and_then(|k| slaves.iter().find(|(d, _)| d.key == k))
        .map(|(_, a)| a);
    let di = def
        .index()
        .filter(|_| {
            matches!(
                def,
                LibraryItem::DigitalInput { .. } | LibraryItem::ImpulseCounter { .. }
            )
        })
        .and_then(|n| inputs.iter().find(|e| e.index == n));
    item_naming_with_di(def, slave, di)
}

fn unix_secs() -> f64 {
    chrono::Utc::now().timestamp_millis() as f64 / 1000.0
}

/// A deterministic initial runtime (hours) per lamp, so lamps do not all show 0 h.
fn initial_runtime_secs(lamp: u32) -> f64 {
    f64::from(1200 + (lamp * 137) % 4000) * 3600.0
}

impl SessionCore {
    pub fn new(p: SessionParams, now: Instant) -> Self {
        let ResolvedSetup { items, slaves, .. } = p.resolved;
        let live: Vec<LiveItem> = items
            .into_iter()
            .map(|def| {
                let naming = naming_of(&def, &slaves, &p.inputs);
                let base = default_value(&def);
                let generator = p
                    .demo
                    .generators
                    .get(def.id())
                    .cloned()
                    .or_else(|| def.generator().cloned());
                LiveItem {
                    naming,
                    value: base,
                    base,
                    generator,
                    gen_state: GeneratorState::default(),
                    manual: false,
                    def,
                }
            })
            .collect();
        let defs: Vec<&LibraryItem> = live.iter().map(|i| &i.def).collect();
        let lamp_count = super::faults::lamp_count(p.demo.dali_lamp_count, &defs);
        let lamps = (1..=lamp_count)
            .map(|n| {
                let max = p
                    .groups
                    .iter()
                    .find(|g| g.converters.contains(&n))
                    .map_or(100, |g| g.max_level);
                (
                    n,
                    Lamp {
                        brightness: f64::from(max),
                        converter_ok: true,
                        lamp_ok: true,
                        lost: false,
                        runtime_secs: initial_runtime_secs(n),
                    },
                )
            })
            .collect();
        let interval = p.demo.send_interval_sec;
        let selection_key = selection_key(&p.demo);
        let mut core = Self {
            selection_key,
            config_restart: false,
            device_id: p.device_id,
            device_name: p.device_name,
            user: p.user,
            started_at: now_iso(),
            status: SessionStatus::Running,
            items: live,
            slaves,
            groups: p.groups.into_iter().map(|g| (g, GroupMode::Auto)).collect(),
            inputs: p.inputs,
            lamps,
            scenarios: p.scenarios,
            faults: Vec::new(),
            alarms: BTreeMap::new(),
            bus_cascade: BTreeSet::new(),
            timed_out: BTreeSet::new(),
            recent_cleared: VecDeque::new(),
            recent_status: VecDeque::new(),
            cleared_by_all: Vec::new(),
            ops: OperationTracker::default(),
            prompts: Vec::new(),
            auto_accept: p.auto_accept,
            realtime_connected: false,
            last_sent_at: None,
            scheduler: Scheduler::new(now, interval, LOCK_REFRESH_INTERVAL),
            rng: SeededRng::new(p.seed),
            last_gen_tick: now,
            revision: 0,
            time_zone: p.time_zone,
            clock_skew: p.clock_skew,
            rs485_active: p.rs485_active,
            group_modes: p.group_modes,
            write_seeds: p.write_seeds,
            debounced: BTreeSet::new(),
            debounce_due: None,
            exec_inflight: BTreeSet::new(),
            gated: HashMap::new(),
            last_lock_wall: chrono::Utc::now(),
            fatal: None,
            supersede: false,
            out: Vec::new(),
            events: Vec::new(),
            ui_changed: BTreeSet::new(),
            send_items: BTreeSet::new(),
            send_lamps: BTreeSet::new(),
            send_all: None,
            dirty: true,
            emit_changed: false,
            next_tag: 1,
            purposes: HashMap::new(),
            last_error: None,
            demo: p.demo,
        };
        core.seed_counters(&p.counter_seeds);
        core.restore_alarms(&p.restored_alarms, now);
        core
    }

    /// Counters start at the newest of: stored measurement, snapshot, default.
    fn seed_counters(&mut self, seeds: &BTreeMap<String, f64>) {
        let snapshot = self.demo.snapshot.as_ref().map(|s| s.values.clone());
        for item in &mut self.items {
            if !item.is_counter() {
                continue;
            }
            let id = item.def.id().to_owned();
            let best = counter_seed(
                item.base,
                snapshot.as_ref().and_then(|s| s.get(&id).copied()),
                seeds.get(&id).copied(),
            );
            item.value = best;
            item.gen_state.last = Some(best);
        }
    }

    /// Session start: start scenario, snapshot values, initial alarm effects, first full send.
    /// Problems are reported as non-fatal `session-error`s.
    pub fn start(&mut self, now: Instant) {
        // The groups are in the mode the newest `lw_daliGroupChange` says (the plugin shows that
        // one); no event, nothing changed.
        for pos in 0..self.groups.len() {
            let id = self.groups[pos].0.id;
            if let Some(&mode) = self.group_modes.get(&id)
                && mode != GroupMode::Auto
            {
                self.switch_group(pos, mode);
            }
        }
        if let Some(sc) = self.scenarios.iter().find(|s| s.start).cloned() {
            // A generator the presenter persisted for an item (`lw_demo.generators`) is not
            // replaced by the start scenario's one.
            let steps: Vec<crate::model::ScenarioStep> = sc
                .set
                .iter()
                .filter(|s| {
                    !matches!(s, crate::model::ScenarioStep::Generator { item_id, .. }
                        if self.demo.generators.contains_key(item_id))
                })
                .cloned()
                .collect();
            if let Err(e) = self.apply_scenario_steps(&steps, now) {
                self.report_error(&e, now);
            }
        }
        if let Some(snap) = self.demo.snapshot.clone() {
            for (id, v) in snap.values {
                if let Some(idx) = self.index_of(&id)
                    && !self.items[idx].is_counter()
                    && check_value(&self.items[idx].def, v).is_ok()
                {
                    let it = &mut self.items[idx];
                    it.value = v;
                    it.gen_state.last = Some(v);
                }
            }
        }
        // What the plugin shows as the control state of a writable register wins over the
        // snapshot (it compares the readback with the last SUCCESSFUL `lw_modbusWrite`).
        for (id, v) in std::mem::take(&mut self.write_seeds) {
            if let Some(idx) = self.index_of(&id)
                && v.is_finite()
                && !self.items[idx].is_counter()
            {
                let it = &mut self.items[idx];
                it.value = v;
                it.gen_state.last = Some(v);
                it.manual = true;
            }
        }
        for idx in 0..self.items.len() {
            self.alarm_effects(idx);
        }
        self.send_all = Some(false);
        self.emit_changed = true;
    }

    /// `now` on the server's clock (local clock + measured skew).
    pub(super) fn server_now(&self) -> chrono::DateTime<chrono::Utc> {
        crate::c8y::server_now(&self.clock_skew)
    }

    // ───────────────────────────── helpers ─────────────────────────────

    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    pub(super) fn index_of(&self, item_id: &str) -> Option<usize> {
        self.items.iter().position(|i| i.def.id() == item_id)
    }

    pub(super) fn item_index(&self, item_id: &str) -> AppResult<usize> {
        self.index_of(item_id)
            .ok_or_else(|| AppError::NotFound(format!("item {item_id}")))
    }

    /// Bump and return the revision (every observable change).
    pub(super) fn bump(&mut self) -> u32 {
        self.revision = self.revision.wrapping_add(1);
        self.dirty = true;
        self.revision
    }

    pub(super) fn push(&mut self, payload: OutboundPayload, purpose: Purpose) {
        self.out.push(Queued {
            payload,
            purpose,
            lane: None,
        });
    }

    pub(super) fn push_on(&mut self, payload: OutboundPayload, purpose: Purpose, lane: Lane) {
        self.out.push(Queued {
            payload,
            purpose,
            lane: Some(lane),
        });
    }

    pub(super) fn event(
        &mut self,
        event_type: &str,
        text: &str,
        extra: Option<Map<String, Value>>,
    ) {
        let body = build_event(&self.device_id, &now_iso(), event_type, text, extra);
        self.push(OutboundPayload::Event(body), Purpose::Event);
    }

    /// Non-fatal `session-error` (repeats of the same code are throttled to one per 30 s).
    pub(super) fn report_error(&mut self, e: &AppError, now: Instant) {
        let code = error_code(e);
        if let Some((last, at)) = &self.last_error
            && *last == code
            && now.duration_since(*at) < std::time::Duration::from_secs(30)
        {
            log::warn!("session error (throttled): {e}");
            return;
        }
        self.last_error = Some((code.clone(), now));
        self.bump();
        self.events.push(SessionEvent::Error(SessionError {
            code,
            message: e.to_string(),
            fatal: false,
        }));
    }

    /// Slave key of a Modbus item.
    pub(super) fn is_paused(&self, item: &LiveItem) -> bool {
        item.def
            .slave_key()
            .is_some_and(|k| self.timed_out.contains(k))
    }

    /// Lamp number of a DALI item.
    fn lamp_of(item: &LiveItem) -> Option<u32> {
        match item.def {
            LibraryItem::Dali { index, .. } => Some(index),
            _ => None,
        }
    }

    pub(super) fn lamp_item(&self, lamp: u32) -> Option<usize> {
        self.items
            .iter()
            .position(|i| Self::lamp_of(i) == Some(lamp))
    }

    /// Whether the firmware would send lamp `n`: assigned to a group (when groups exist) and
    /// not lost.
    fn lamp_sendable(&self, n: u32) -> bool {
        self.lamp_grouped(n) && !self.lamps.get(&n).is_some_and(|l| l.lost)
    }

    /// Lamp `n` is assigned to a group (every lamp when the device has no groups).
    pub(super) fn lamp_grouped(&self, n: u32) -> bool {
        self.groups.is_empty() || self.groups.iter().any(|(g, _)| g.converters.contains(&n))
    }

    fn lamp_status(&self, n: u32) -> DaliLampStatus {
        let l = self.lamps.get(&n).cloned().unwrap_or(Lamp {
            converter_ok: true,
            lamp_ok: true,
            ..Lamp::default()
        });
        DaliLampStatus {
            converter_ok: l.converter_ok,
            lamp_ok: l.lamp_ok,
            runtime_hours: (l.runtime_secs / 3600.0) as u32,
        }
    }

    /// Measurement values an item sends now (empty when paused / not sendable).
    fn values_of(&self, idx: usize) -> Vec<MeasurementValue> {
        let item = &self.items[idx];
        let Some(naming) = &item.naming else {
            return Vec::new();
        };
        if self.is_paused(item) {
            return Vec::new();
        }
        if let Some(n) = Self::lamp_of(item) {
            if !self.lamp_sendable(n) {
                return Vec::new();
            }
            return item_values(naming, item.value, Some(self.lamp_status(n)));
        }
        if let (Some(index), true) = (
            item.def.index(),
            matches!(
                item.def,
                LibraryItem::DigitalInput { .. } | LibraryItem::ImpulseCounter { .. }
            ),
        ) && self
            .inputs
            .iter()
            .any(|e| e.index == index && !e.settings.is_active)
        {
            return Vec::new();
        }
        if self.sensor_lost(&item.def) {
            return Vec::new();
        }
        let grid = match (&item.def, item.def.index()) {
            (LibraryItem::ImpulseCounter { .. }, Some(n)) => self
                .inputs
                .iter()
                .find(|e| e.index == n)
                .map(|e| CounterGrid {
                    init_count: e.settings.init_count.unwrap_or(0),
                    increment: e.settings.increment.unwrap_or(1).max(1),
                }),
            _ => None,
        };
        item_values_with_grid(naming, item.value, None, grid)
    }

    /// A brightness / motion sensor with an active `_lost` / `_notExisting` fault, or while the
    /// DALI bus has no power, is not sent (firmware §1.3, §2.1).
    pub(super) fn sensor_lost(&self, def: &LibraryItem) -> bool {
        let (kind, index) = match def {
            LibraryItem::DaliBrightness { index, .. } => ("brightness", *index),
            LibraryItem::DaliMotion { index, .. } => ("motion", *index),
            _ => return false,
        };
        self.bus_down() || self.faults.iter().any(|f| {
            matches!(&f.target, crate::model::FaultTarget::Sensor { sensor_kind, index: i, alarm }
                if sensor_kind == kind
                    && u32::from(*i) == index
                    && (alarm.ends_with("_lost") || alarm.ends_with("_notExisting")))
        })
    }

    fn hidden_lamp_values(&self, n: u32) -> Vec<MeasurementValue> {
        if self.lamp_item(n).is_some() || !self.lamp_sendable(n) {
            return Vec::new();
        }
        let b = self.lamps.get(&n).map_or(0.0, |l| l.brightness);
        dali_lamp_values(n, b, self.lamp_status(n))
    }

    /// Queue the measurements of lamp `n` (item or hidden lamp).
    pub(super) fn send_lamp(&mut self, n: u32) {
        match self.lamp_item(n) {
            Some(idx) => {
                self.send_items.insert(idx);
            }
            None => {
                self.send_lamps.insert(n);
            }
        }
    }

    // ───────────────────────────── values ─────────────────────────────

    /// Set an item's value. `Manual` / `Scenario` / `Operation` / `Group` set the manual override
    /// and send at once; `Generator` only updates the UI (sent periodically).
    pub(super) fn set_value(&mut self, idx: usize, value: f64, source: Source) {
        let prev = self.items[idx].value;
        {
            let it = &mut self.items[idx];
            it.value = value;
            it.gen_state.last = Some(value);
            if source != Source::Generator {
                it.manual = true;
            }
        }
        self.ui_changed.insert(idx);
        match source {
            Source::Generator => {}
            // A slider drag: one request per window, with the latest value.
            Source::Manual => {
                self.debounced.insert(idx);
                self.debounce_due
                    .get_or_insert(Instant::now() + SEND_DEBOUNCE);
            }
            _ => {
                self.send_items.insert(idx);
            }
        }
        if prev != value {
            if matches!(
                source,
                Source::Manual | Source::Scenario | Source::Operation
            ) {
                self.value_effects(idx);
            }
            self.digital_input_action(idx, prev, value);
        }
        self.alarm_effects(idx);
    }

    /// Validate and set a value from a command / scenario / operation (counters never decrease).
    pub(super) fn set_checked(
        &mut self,
        item_id: &str,
        value: f64,
        source: Source,
    ) -> AppResult<()> {
        let idx = self.item_index(item_id)?;
        let item = &self.items[idx];
        if item.naming.is_none() {
            return Err(AppError::InvalidInput(format!("{item_id} has no value")));
        }
        check_value(&item.def, value)?;
        if item.is_counter() && value < item.value {
            return Err(AppError::InvalidInput(format!(
                "{item_id}: counter cannot go back from {} to {value}",
                item.value
            )));
        }
        self.set_value(idx, value, source);
        Ok(())
    }

    /// Event / inventory effects of a multi-effect item (on real value changes, not generator
    /// steps).
    fn value_effects(&mut self, idx: usize) {
        let item = self.items[idx].clone();
        let effects: Vec<ItemEffect> = match &item.def {
            LibraryItem::Dali { effects, .. }
            | LibraryItem::DaliBrightness { effects, .. }
            | LibraryItem::DaliMotion { effects, .. }
            | LibraryItem::DigitalInput { effects, .. }
            | LibraryItem::ImpulseCounter { effects, .. }
            | LibraryItem::Modbus { effects, .. } => effects.clone(),
            _ => Vec::new(),
        };
        let value_text = crate::payload::json_number(item.value).to_string();
        let label = match item.def.control() {
            Some(Control::Select { options, .. }) => options
                .iter()
                .find(|o| o.value == item.value)
                .map(|o| o.label.de.clone()),
            _ => None,
        };
        for e in effects {
            match e {
                ItemEffect::Event { event_type, text } => {
                    let text = text
                        .replace("{value}", &value_text)
                        .replace("{label}", label.as_deref().unwrap_or(&value_text));
                    self.event(&event_type, &text, None);
                }
                ItemEffect::Inventory { property } => {
                    let v = match &label {
                        Some(l) => json!(l),
                        None => crate::payload::json_number(item.value),
                    };
                    let mut body = Map::new();
                    body.insert(property, v);
                    self.push(
                        OutboundPayload::InventoryPatch(Value::Object(body)),
                        Purpose::Inventory,
                    );
                }
                ItemEffect::Alarm { .. } => {}
            }
        }
    }

    /// Alarm effects: raised while value ≥ threshold, recovered below (PRD §7.5 rules). An
    /// effect alarm cleared in Cumulocity while the value is still above is raised again only
    /// after the value went below once (D9).
    pub(super) fn alarm_effects(&mut self, idx: usize) {
        let item = &self.items[idx];
        let value = item.value;
        let effects: Vec<ItemEffect> = match &item.def {
            LibraryItem::Dali { effects, .. }
            | LibraryItem::DaliBrightness { effects, .. }
            | LibraryItem::DaliMotion { effects, .. }
            | LibraryItem::DigitalInput { effects, .. }
            | LibraryItem::ImpulseCounter { effects, .. }
            | LibraryItem::Modbus { effects, .. } => effects.clone(),
            _ => Vec::new(),
        };
        for e in effects {
            if let ItemEffect::Alarm {
                alarm_type,
                severity,
                text,
                threshold,
            } = e
            {
                let track = self.alarms.get(&alarm_type);
                let active = track.is_some_and(|a| a.active);
                let suppressed = track.is_some_and(|a| a.suppressed);
                if value >= threshold && !active && !suppressed {
                    self.raise_alarm(&AlarmSpec {
                        alarm_type,
                        text,
                        severity,
                    });
                } else if value < threshold {
                    if active {
                        let put = !self.demo.realistic_mode;
                        self.recover_alarm(&alarm_type, put);
                    } else if let Some(t) = self.alarms.get_mut(&alarm_type) {
                        t.suppressed = false;
                    }
                }
            }
        }
    }

    /// `daliAction` digital input: rising edge → `highLevelAction`, falling → `lowLevelAction`
    /// on the configured groups (reason `digital-input-NN`).
    fn digital_input_action(&mut self, idx: usize, prev: f64, value: f64) {
        let LibraryItem::DigitalInput { index, .. } = self.items[idx].def else {
            return;
        };
        let Some(cfg) = self
            .inputs
            .iter()
            .find(|e| e.index == index && e.settings.input_type == DigitalInputType::DaliAction)
            .cloned()
        else {
            return;
        };
        let action = match (prev != 0.0, value != 0.0) {
            (false, true) => cfg.settings.high_level_action,
            (true, false) => cfg.settings.low_level_action,
            _ => None,
        };
        let mode = match action.as_deref() {
            Some("on") => GroupMode::On,
            Some("off") => GroupMode::Off,
            Some("auto") => GroupMode::Auto,
            _ => return,
        };
        let reason = format!("digital-input-{index:02}");
        for g in cfg.settings.groups {
            let _ = self.set_group_mode(g, mode, &reason, false);
        }
    }

    // ───────────────────────────── alarms ─────────────────────────────

    /// Raise a Cumulocity alarm (tracked by type). Raising an open one again re-posts it
    /// (Cumulocity de-duplicates it and counts up); a new one forgets the old id.
    pub(super) fn raise_alarm(&mut self, spec: &AlarmSpec) {
        let track = self.alarms.entry(spec.alarm_type.clone()).or_default();
        if !track.active {
            track.id = None;
            track.id_at = None;
            track.status = OpenAlarmStatus::Active;
        }
        track.active = true;
        track.suppressed = false;
        track.clear_pending = false;
        track.inflight += 1;
        let body = build_alarm(&self.device_id, &now_iso(), spec);
        self.push(
            OutboundPayload::Alarm(body),
            Purpose::Alarm {
                alarm_type: spec.alarm_type.clone(),
            },
        );
    }

    /// Recover an alarm like the firmware: `lw_alarm` "Alarm cleared: <type>", then, with
    /// `put_clear`, clear the Cumulocity alarm (PRD §7.5; realistic mode passes `false`).
    pub(super) fn recover_alarm(&mut self, alarm_type: &str, put_clear: bool) {
        let body = build_alarm_cleared(&self.device_id, &now_iso(), alarm_type);
        self.push(OutboundPayload::Event(body), Purpose::Event);
        if put_clear {
            self.put_alarm_clear(alarm_type);
        }
        if let Some(track) = self.alarms.get_mut(alarm_type) {
            track.active = false;
            track.suppressed = false;
        }
    }

    /// `PUT` CLEARED for the tracked alarm of `alarm_type` (once its create has answered, if it
    /// is still in flight). Does not change `active`.
    pub(super) fn put_alarm_clear(&mut self, alarm_type: &str) {
        let Some(track) = self.alarms.get_mut(alarm_type) else {
            return;
        };
        match track.id.clone() {
            Some(id) => self.push(
                OutboundPayload::AlarmClear { alarm_id: id },
                Purpose::AlarmClear,
            ),
            // Only an alarm whose create is still in flight gets its id later.
            None if track.inflight > 0 => track.clear_pending = true,
            None => {}
        }
    }

    // ───────────────────────────── DALI groups ─────────────────────────────

    /// Switch a DALI group (PRD §7.2): on → max level, off → 0, auto → generator / default;
    /// `lw_daliGroupChange` when the mode really changed, or always with `always_event` (a cloud
    /// `lw_switchGroupMode`: the plugin reads the mode from the newest event, so even "no
    /// change" is reported, as insurance against a mode this session did not know).
    pub(super) fn set_group_mode(
        &mut self,
        group_id: u32,
        mode: GroupMode,
        reason: &str,
        always_event: bool,
    ) -> AppResult<()> {
        let pos = self
            .groups
            .iter()
            .position(|(g, _)| g.id == group_id)
            .ok_or_else(|| AppError::NotFound(format!("group {group_id}")))?;
        let prev = self.switch_group(pos, mode);
        if prev != mode || always_event {
            let body =
                build_group_change(&self.device_id, &now_iso(), group_id, prev, mode, reason);
            self.push(OutboundPayload::Event(body), Purpose::Event);
        }
        self.emit_changed = true;
        Ok(())
    }

    /// Put group `pos` into `mode` and move its lamps; returns the previous mode. No event.
    fn switch_group(&mut self, pos: usize, mode: GroupMode) -> GroupMode {
        let (group, prev) = self.groups[pos].clone();
        self.groups[pos].1 = mode;
        for &lamp in &group.converters {
            let target = match mode {
                GroupMode::On => Some(f64::from(group.max_level)),
                GroupMode::Off => Some(0.0),
                GroupMode::Auto => None,
            };
            match self.lamp_item(lamp) {
                Some(idx) => match target {
                    Some(v) => self.set_value(idx, v, Source::Group),
                    None => {
                        let it = &mut self.items[idx];
                        it.manual = false;
                        let v = if it.generator.is_some() {
                            it.gen_state.last.unwrap_or(it.base)
                        } else {
                            it.base
                        };
                        self.set_value(idx, v, Source::Group);
                        self.items[idx].manual = false;
                    }
                },
                // A lamp the device does not have (a group may list more lamps than
                // `daliLampCount`) sends nothing.
                None => {
                    if let Some(l) = self.lamps.get_mut(&lamp) {
                        l.brightness = target.unwrap_or(f64::from(group.max_level));
                        self.send_lamps.insert(lamp);
                    }
                }
            }
        }
        prev
    }

    /// `lw_switchConverterState`: one lamp on (its group's max level) or off.
    pub(super) fn switch_converter(&mut self, lamp: u32, on: bool) -> AppResult<()> {
        if !self.lamps.contains_key(&lamp) {
            return Err(AppError::NotFound(format!("lamp {lamp}")));
        }
        let max = self
            .groups
            .iter()
            .find(|(g, _)| g.converters.contains(&lamp))
            .map_or(100, |(g, _)| g.max_level);
        let v = if on { f64::from(max) } else { 0.0 };
        match self.lamp_item(lamp) {
            Some(idx) => self.set_value(idx, v, Source::Group),
            None => {
                if let Some(l) = self.lamps.get_mut(&lamp) {
                    l.brightness = v;
                }
                self.send_lamps.insert(lamp);
            }
        }
        Ok(())
    }

    // ───────────────────────────── commands ─────────────────────────────

    pub fn set_generator(
        &mut self,
        item_id: &str,
        generator: Option<GeneratorConfig>,
    ) -> AppResult<()> {
        let idx = self.item_index(item_id)?;
        if let Some(g) = &generator
            && let Some(p) = crate::library::validate::generator_problem(g)
        {
            return Err(AppError::InvalidInput(format!("{item_id}: {p}")));
        }
        let it = &mut self.items[idx];
        if it.naming.is_none() {
            return Err(AppError::InvalidInput(format!("{item_id} has no value")));
        }
        it.gen_state.last = Some(it.value);
        if generator.is_some() {
            it.manual = false;
        }
        it.generator = generator;
        self.ui_changed.insert(idx);
        // `item-values-changed` does not carry the generator: refresh the snapshot as well.
        self.emit_changed = true;
        self.dirty = true;
        Ok(())
    }

    pub fn resume_generator(&mut self, item_id: &str) -> AppResult<()> {
        let idx = self.item_index(item_id)?;
        let it = &mut self.items[idx];
        it.manual = false;
        it.gen_state.last = Some(it.value);
        self.ui_changed.insert(idx);
        Ok(())
    }

    pub fn set_send_interval(&mut self, seconds: u32, now: Instant) -> AppResult<()> {
        if !(MIN_SEND_INTERVAL_SEC..=MAX_SEND_INTERVAL_SEC).contains(&seconds) {
            return Err(AppError::InvalidInput(format!(
                "send interval {seconds} s ({MIN_SEND_INTERVAL_SEC}–{MAX_SEND_INTERVAL_SEC})"
            )));
        }
        self.demo.send_interval_sec = seconds;
        self.scheduler.set_interval(now, seconds);
        self.emit_changed = true;
        Ok(())
    }

    pub fn set_auto_accept(&mut self, enabled: bool) {
        self.auto_accept = enabled;
        self.emit_changed = true;
    }

    pub fn send_all_now(&mut self) {
        self.send_all = Some(self.send_all.unwrap_or(false));
    }

    /// Write the live values to `lw_demo.snapshot` (queued).
    pub fn save_snapshot(&mut self) -> AppResult<()> {
        let values: BTreeMap<String, f64> = self
            .items
            .iter()
            .filter(|i| i.naming.is_some())
            .map(|i| (i.def.id().to_owned(), i.value))
            .collect();
        let snapshot = DemoSnapshot {
            saved_at: now_iso(),
            values,
        };
        // Its own fragment: nothing else of `lw_demo` is written from this session's copy.
        let body = demo_state::snapshot_body(Some(&snapshot))?;
        self.demo.snapshot = Some(snapshot);
        self.push(OutboundPayload::InventoryPatch(body), Purpose::Inventory);
        Ok(())
    }

    /// `update_demo_setup` changed the device's setup: take interval, realistic mode and
    /// generator overrides (the item selection changes with the next session start).
    pub fn setup_changed(&mut self, demo: LwDemoState, now: Instant) {
        if demo.send_interval_sec != self.demo.send_interval_sec {
            self.scheduler.set_interval(now, demo.send_interval_sec);
        }
        for (idx, it) in self.items.iter_mut().enumerate() {
            let id = it.def.id();
            // Only an item whose persisted entry changed takes the new generator: unsaved
            // session overrides of the other items stay.
            if demo.generators.get(id) == self.demo.generators.get(id) {
                continue;
            }
            let g = demo
                .generators
                .get(id)
                .cloned()
                .or_else(|| it.def.generator().cloned());
            if g != it.generator {
                it.generator = g;
                it.gen_state.last = Some(it.value);
                self.ui_changed.insert(idx);
            }
        }
        self.demo = demo;
        self.emit_changed = true;
    }

    /// `set_dali_groups` / `set_digital_inputs` changed the device config: take the new groups
    /// (keeping the mode of groups that still exist) and digital inputs (series, unit, actions).
    pub fn config_changed(
        &mut self,
        groups: Option<Vec<DaliGroup>>,
        inputs: Option<Vec<DigitalInputEntry>>,
    ) {
        if let Some(gs) = groups {
            let old: BTreeMap<u32, GroupMode> =
                self.groups.iter().map(|(g, m)| (g.id, *m)).collect();
            self.groups = gs
                .into_iter()
                .map(|g| {
                    let m = old.get(&g.id).copied().unwrap_or(GroupMode::Auto);
                    (g, m)
                })
                .collect();
            // Lamps that joined / left a group join / leave a running bus failure.
            self.sync_bus_cascade();
        }
        if let Some(ins) = inputs {
            self.inputs = ins;
            for idx in 0..self.items.len() {
                let naming = naming_of(&self.items[idx].def, &self.slaves, &self.inputs);
                if naming == self.items[idx].naming {
                    continue;
                }
                if series_names(&naming) == series_names(&self.items[idx].naming) {
                    // Only the unit or the title (= fragment, D10) changed.
                    self.items[idx].naming = naming;
                    self.ui_changed.insert(idx);
                } else {
                    // A type change (value <-> impulse counter) also changes the control and
                    // counter semantics: the item keeps running as it started until a restart.
                    self.config_restart = true;
                }
            }
        }
        self.emit_changed = true;
    }

    // ───────────────────────────── realtime / results ─────────────────────────────

    pub fn realtime_status(&mut self, connected: bool, error: Option<String>) {
        if self.realtime_connected == connected && error.is_none() {
            return;
        }
        self.realtime_connected = connected;
        let revision = self.bump();
        self.events.push(SessionEvent::Realtime(RealtimeStatus {
            revision,
            device_id: self.device_id.clone(),
            connected,
            error,
        }));
        self.emit_changed = true;
    }

    /// Book an outbound result.
    pub fn outbound_result(&mut self, res: OutboundResult, now: Instant) {
        if res.tag == LANE_DEAD_TAG {
            // A lane task ended: nothing will be sent through it any more.
            self.fatal.get_or_insert_with(|| {
                AppError::Internal("the outbound worker stopped; the session cannot send".into())
            });
            return;
        }
        let Some(purpose) = self.purposes.remove(&res.tag) else {
            return;
        };
        match (purpose, res.result) {
            (Purpose::Measurements { periodic }, Ok(_)) => {
                self.last_sent_at = Some(now_iso());
                self.dirty = true;
                if periodic {
                    self.emit_changed = true;
                }
            }
            (Purpose::Alarm { alarm_type }, Ok(body)) => {
                let id = crate::c8y::id_of(&body).ok();
                // A re-posted ACKNOWLEDGED alarm stays ACKNOWLEDGED.
                let status = body
                    .get("status")
                    .and_then(Value::as_str)
                    .and_then(OpenAlarmStatus::parse);
                let mut clear = None;
                if let Some(track) = self.alarms.get_mut(&alarm_type) {
                    if track.id != id || track.id_at.is_none() {
                        track.id_at = Some(now);
                    }
                    track.id = id.clone();
                    track.inflight = track.inflight.saturating_sub(1);
                    if let Some(st) = status {
                        track.status = st;
                    }
                    if track.clear_pending && !track.active {
                        track.clear_pending = false;
                        clear = id.clone();
                    }
                }
                if let Some(id) = clear {
                    self.push(
                        OutboundPayload::AlarmClear { alarm_id: id },
                        Purpose::AlarmClear,
                    );
                }
                let mut changed = None;
                for f in &mut self.faults {
                    let st = status.unwrap_or(f.status);
                    if f.alarm_type == alarm_type && (f.alarm_id != id || f.status != st) {
                        f.alarm_id = id.clone();
                        f.status = st;
                        changed = Some(f.clone());
                    }
                }
                if let Some(f) = changed {
                    self.fault_event(f.target.clone(), Some(f));
                }
                // Changed in Cumulocity before this answer told us the id.
                if let Some(id) = &id
                    && let Some(&(_, st)) = self.recent_status.iter().rev().find(|(i, _)| i == id)
                {
                    self.alarm_status_in_cloud(id, st);
                }
                if let Some(id) = id
                    && self.recent_cleared.contains(&id)
                {
                    self.alarm_cleared_in_cloud(&id);
                }
            }
            (Purpose::Alarm { alarm_type }, Err(e)) => {
                // The alarm does not exist in Cumulocity (a failed re-post of a known one leaves
                // it alone): forget it, so a later clear does not wait for an id that never
                // comes and a threshold effect can raise it again. A fault lives exactly as
                // long as its alarm (D9), so it ends too, without the "Alarm cleared" event:
                // there was no alarm. A derived (cascade) alarm is released as well: the next
                // sync raises it again.
                let mut gone = false;
                if let Some(track) = self.alarms.get_mut(&alarm_type) {
                    track.inflight = track.inflight.saturating_sub(1);
                    if track.id.is_none() && track.inflight == 0 {
                        track.active = false;
                        track.clear_pending = false;
                        gone = true;
                    }
                }
                if gone {
                    self.bus_cascade.remove(&alarm_type);
                    while let Some(pos) =
                        self.faults.iter().position(|f| f.alarm_type == alarm_type)
                    {
                        self.end_fault(pos);
                    }
                }
                self.report_error(&e, now);
            }
            (
                Purpose::OperationStatus {
                    operation_id,
                    executing,
                },
                res,
            ) => {
                if let Err(e) = &res {
                    log::warn!("operation {operation_id} status update failed: {e}");
                    self.report_error(e, now);
                }
                if executing {
                    // EXECUTING is answered (or failed): the final status may follow now.
                    self.exec_inflight.remove(&operation_id);
                    if let Some(g) = self.gated.remove(&operation_id) {
                        self.op_status_now(&operation_id, g.status, g.reason);
                    }
                }
            }
            (Purpose::LockRefresh, Err(e @ AppError::DeviceLocked { .. })) => {
                // Somebody took the device over: two sessions must not simulate one device.
                self.fatal.get_or_insert(e);
            }
            (Purpose::ClearAll, Err(e)) => {
                // Alarms may still be open in Cumulocity although every effect ended here:
                // read them and take the ones the clear missed over again.
                let auth = matches!(e, AppError::NotAuthenticated);
                self.report_error(&e, now);
                if !auth && !self.cleared_by_all.is_empty() {
                    self.push(OutboundPayload::ReadOpenAlarms, Purpose::ReadOpenAlarms);
                }
            }
            (Purpose::ReadOpenAlarms, Ok(body)) => {
                let open: Vec<(String, Option<OpenAlarmStatus>)> = body["alarms"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|a| {
                                let st = a
                                    .get("status")
                                    .and_then(Value::as_str)
                                    .and_then(OpenAlarmStatus::parse);
                                Some((crate::c8y::id_of(a).ok()?, st))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let still: Vec<RestoredAlarm> = std::mem::take(&mut self.cleared_by_all)
                    .into_iter()
                    .filter_map(|mut a| {
                        let (_, st) = open.iter().find(|(id, _)| Some(id) == a.id.as_ref())?;
                        if let Some(st) = st {
                            a.status = *st;
                        }
                        Some(a)
                    })
                    .collect();
                if !still.is_empty() {
                    self.restore_alarms(&still, now);
                    self.emit_changed = true;
                }
            }
            (_, Ok(_)) => {}
            (_, Err(e)) => self.report_error(&e, now),
        }
    }

    // ───────────────────────────── timers ─────────────────────────────

    /// Earliest timer: scheduler or a prompt deadline.
    pub fn next_deadline(&self) -> Instant {
        self.prompts
            .iter()
            .map(|p| p.deadline)
            .chain(self.debounce_due)
            .fold(self.scheduler.next_deadline(), Instant::min)
    }

    pub fn timers(&mut self, now: Instant) {
        let due = self.scheduler.poll(now);
        if due.generators {
            self.step_generators(now);
        }
        if due.periodic {
            self.send_all = Some(true);
        }
        if self.debounce_due.is_some_and(|d| now >= d) {
            self.debounce_due = None;
            let items = std::mem::take(&mut self.debounced);
            self.send_items.extend(items);
        }
        // The lock refresh also follows the wall clock: a laptop that slept stops the monotonic
        // timers, and the lock would look abandoned.
        let since = chrono::Utc::now() - self.last_lock_wall;
        let interval = chrono::Duration::from_std(LOCK_REFRESH_INTERVAL).unwrap_or_default();
        if due.lock_refresh || since >= interval {
            self.refresh_lock();
        }
        self.expire_prompts(now);
    }

    fn step_generators(&mut self, now: Instant) {
        let dt = now.duration_since(self.last_gen_tick).as_secs_f64();
        self.last_gen_tick = now;
        let wall = unix_secs();
        let tick = Tick {
            now_secs: wall,
            dt_secs: dt,
            local_hour: local_hour_in_zone(wall, &self.time_zone)
                .or_else(|| local_hour_in_zone(wall, DEFAULT_TIME_ZONE))
                .unwrap_or_else(|| local_hour(wall, 0.0)),
        };
        for idx in 0..self.items.len() {
            let it = &self.items[idx];
            let Some(cfg) = it.generator.clone() else {
                continue;
            };
            if it.manual || it.naming.is_none() || self.is_paused(it) {
                continue;
            }
            let mut state = it.gen_state.clone();
            let mut v = next_value(&cfg, &mut state, &tick, it.base, &mut self.rng);
            if self.items[idx].is_counter() {
                v = v.max(self.items[idx].value);
            }
            self.items[idx].gen_state = state;
            if v != self.items[idx].value {
                self.set_value(idx, v, Source::Generator);
            }
        }
        // Converter runtime of burning lamps.
        let burning: Vec<u32> = self
            .lamps
            .keys()
            .copied()
            .filter(|n| match self.lamp_item(*n) {
                Some(idx) => self.items[idx].value > 0.0,
                None => self.lamps[n].brightness > 0.0,
            })
            .collect();
        for n in burning {
            if let Some(l) = self.lamps.get_mut(&n) {
                l.runtime_secs += dt;
            }
        }
    }

    /// Refresh the advisory lock: its own fragment, written after checking that the device
    /// still carries our lock (done by the outbound worker, which reports a takeover back).
    fn refresh_lock(&mut self) {
        self.last_lock_wall = chrono::Utc::now();
        let user = self.user.clone();
        self.push(OutboundPayload::LockRefresh { user }, Purpose::LockRefresh);
    }

    /// Release the lock (on stop); a lock of somebody else is left alone by the worker.
    pub fn release_lock(&mut self) {
        let user = self.user.clone();
        self.push(OutboundPayload::LockRelease { user }, Purpose::LockRelease);
    }

    // ───────────────────────────── snapshot / flush ─────────────────────────────

    pub fn snapshot(&self) -> SessionSnapshot {
        SessionSnapshot {
            revision: self.revision,
            device_id: self.device_id.clone(),
            device_name: self.device_name.clone(),
            status: self.status,
            started_at: self.started_at.clone(),
            auto_accept: self.auto_accept,
            realistic_mode: self.demo.realistic_mode,
            send_interval_sec: self.demo.send_interval_sec,
            last_sent_at: self.last_sent_at.clone(),
            realtime_connected: self.realtime_connected,
            restart_needed: self.config_restart || selection_key(&self.demo) != self.selection_key,
            items: self
                .items
                .iter()
                .map(|i| ItemRuntimeState {
                    item_id: i.def.id().to_owned(),
                    value: i.value,
                    manual_override: i.manual,
                    generator: i.generator.clone(),
                    naming: i.naming.clone(),
                    paused: self.is_paused(i)
                        || self.sensor_lost(&i.def)
                        || Self::lamp_of(i)
                            .is_some_and(|n| self.lamps.get(&n).is_some_and(|l| l.lost)),
                })
                .collect(),
            groups: self
                .groups
                .iter()
                .map(|(g, m)| DaliGroupState {
                    id: g.id,
                    name: g.name.clone(),
                    mode: *m,
                })
                .collect(),
            active_faults: self.faults.clone(),
            pending_operations: self.prompts.iter().map(|p| p.prompt.clone()).collect(),
        }
    }

    /// Turn this iteration's changes into tagged outbound items, events and a snapshot.
    pub fn finish(&mut self) -> Flush {
        // Measurements.
        let mut values = Vec::new();
        let stopping = self.status == SessionStatus::Stopping;
        let periodic = match self.send_all.take() {
            Some(periodic) => {
                self.debounced.clear();
                self.debounce_due = None;
                for idx in 0..self.items.len() {
                    values.extend(self.values_of(idx));
                }
                let lamps: Vec<u32> = self.lamps.keys().copied().collect();
                for n in lamps {
                    values.extend(self.hidden_lamp_values(n));
                }
                self.send_items.clear();
                self.send_lamps.clear();
                periodic
            }
            None => {
                let items = std::mem::take(&mut self.send_items);
                for idx in items {
                    values.extend(self.values_of(idx));
                }
                let lamps = std::mem::take(&mut self.send_lamps);
                for n in lamps {
                    values.extend(self.hidden_lamp_values(n));
                }
                false
            }
        };
        if !values.is_empty() && !stopping {
            match build_measurement_bodies(&self.device_id, &now_iso(), &values) {
                Ok(bodies) => {
                    // Measurements go first so a readback precedes the operation's SUCCESSFUL
                    // on the wire as far as the lanes allow.
                    let mut ms: Vec<Queued> = bodies
                        .into_iter()
                        .map(|b| Queued {
                            payload: OutboundPayload::Measurements(b),
                            purpose: Purpose::Measurements { periodic },
                            lane: None,
                        })
                        .collect();
                    self.supersede |= periodic;
                    ms.append(&mut self.out);
                    self.out = ms;
                }
                Err(e) => log::warn!("measurement bodies: {e}"),
            }
        }

        let mut flush = Flush {
            supersede_periodic: std::mem::take(&mut self.supersede),
            ..Flush::default()
        };
        for q in std::mem::take(&mut self.out) {
            let tag = self.next_tag;
            self.next_tag = match self.next_tag.wrapping_add(1) {
                LANE_DEAD_TAG => 1,
                t => t,
            };
            let class = match &q.purpose {
                Purpose::Measurements { periodic: true } => Class::Periodic,
                Purpose::Measurements { periodic: false } => Class::Immediate,
                _ => Class::Critical,
            };
            let mut item = OutboundItem::new(tag, q.payload);
            item.class = class;
            if let Some(l) = q.lane {
                item.lane = l;
            }
            self.purposes.insert(tag, q.purpose);
            flush.outbound.push(item);
        }

        if !self.ui_changed.is_empty() {
            let values: Vec<ItemValue> = std::mem::take(&mut self.ui_changed)
                .into_iter()
                .map(|idx| ItemValue {
                    item_id: self.items[idx].def.id().to_owned(),
                    value: self.items[idx].value,
                    manual_override: self.items[idx].manual,
                })
                .collect();
            let revision = self.bump();
            self.events.push(SessionEvent::Values(ItemValuesChanged {
                revision,
                device_id: self.device_id.clone(),
                values,
            }));
        }
        if self.emit_changed {
            self.emit_changed = false;
            self.bump();
            let snap = self.snapshot();
            self.events.push(SessionEvent::Changed(Some(snap.clone())));
            flush.snapshot = Some(snap);
            self.dirty = false;
        } else if self.dirty {
            self.dirty = false;
            flush.snapshot = Some(self.snapshot());
        }
        flush.events = std::mem::take(&mut self.events);
        flush
    }

    /// Mark a queue failure (the payload was not sent). A closed lane is fatal.
    pub fn enqueue_failed(&mut self, tag: u32, e: super::outbound::EnqueueError, now: Instant) {
        if let Some(Purpose::OperationStatus {
            operation_id,
            executing: true,
        }) = self.purposes.remove(&tag)
        {
            // EXECUTING will never be answered: do not hold the final status back forever.
            self.exec_inflight.remove(&operation_id);
            if let Some(g) = self.gated.remove(&operation_id) {
                self.op_status_now(&operation_id, g.status, g.reason);
            }
        }
        let e = AppError::from(e);
        if matches!(e, AppError::Internal(ref m) if m.contains("stopped")) {
            self.fatal.get_or_insert(e);
        } else {
            self.report_error(&e, now);
        }
    }

    /// Queued items that were removed from the queue before they were sent: `overflow` = the
    /// queue was full (reported once), else a newer send replaced them / the session stops.
    pub fn outbound_dropped(&mut self, tags: &[u32], overflow: bool, now: Instant) {
        for t in tags {
            self.purposes.remove(t);
        }
        if overflow && !tags.is_empty() {
            self.report_error(
                &AppError::Internal(format!(
                    "the outbound queue is full: {} queued measurements were dropped",
                    tags.len()
                )),
                now,
            );
        }
    }

    /// Outstanding outbound results (and final statuses waiting for their EXECUTING answer).
    pub fn outstanding(&self) -> usize {
        self.purposes.len() + self.gated.len()
    }

    /// The reason the session has to end, if any (dead outbound lane, lock taken over).
    pub fn take_fatal(&mut self) -> Option<AppError> {
        self.fatal.take()
    }

    /// Stop: no more measurements, nothing pending is sent.
    pub fn set_stopping(&mut self) {
        self.status = SessionStatus::Stopping;
        self.send_items.clear();
        self.send_lamps.clear();
        self.send_all = None;
        self.debounced.clear();
        self.debounce_due = None;
    }

    /// The session ended with an error.
    pub fn set_failed(&mut self) {
        self.status = SessionStatus::Failed;
    }

    pub(super) fn group_reason() -> &'static str {
        REASON_CLOUD_OPERATION
    }
}
