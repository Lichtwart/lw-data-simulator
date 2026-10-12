//! The outbound worker: the only place a session sends HTTP (PRD §8).
//!
//! The session's state loop must never wait for Cumulocity (a slow or hanging request would
//! freeze generators, commands and the 60 s operation timers). So it only *enqueues* payloads
//! here (`OutboundQueue::enqueue`, never awaits); worker tasks send them and report every
//! outcome on the results channel, which the state loop polls in its `select!` (to update
//! `lastSentAt`, emit `session-error`, store alarm ids, …).
//!
//! Two lanes:
//! - [`Lane::Normal`]: one task, strictly in order: measurements, alarms, events, inventory
//!   patches and the **final** operation statuses (queued after the effects they belong to, so
//!   the plugin never sees SUCCESSFUL before the readback).
//! - [`Lane::Priority`]: a dispatcher that sends every request **concurrently** (one hanging
//!   `PUT` must not delay the others past the plugin's 10 s PENDING timeout): `EXECUTING`
//!   statuses, the advisory lock and statuses queued at stop. Statuses of one operation stay in
//!   order because the session only queues the final status after EXECUTING returned.
//!
//! Queue policy (the normal lane is bounded to [`QUEUE_CAPACITY`] *droppable* items):
//! - measurements are droppable: when the lane is full, queued **periodic** measurements go
//!   first (a newer full send supersedes them), then queued immediate ones;
//! - alarms, events, inventory patches, lock and status updates are **never dropped** (the lane
//!   grows past the capacity for them; only a [`HARD_CAP`] of items refuses more);
//! - a new periodic send replaces the older queued periodic one ([`OutboundQueue::supersede_periodic`]);
//!   on stop all queued measurements are discarded ([`OutboundQueue::clear_measurements`]).
//!
//! A lane task that ends (panic, abort) closes its lane: `enqueue` fails with
//! [`EnqueueError::Closed`] and the session gets a result with [`LANE_DEAD_TAG`], which it
//! treats as fatal.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use serde_json::Value;
use tokio::sync::{Notify, mpsc};
use tokio::task::JoinSet;

use crate::c8y::C8yClient;
use crate::device::demo_state;
use crate::error::{AppError, AppResult};
use crate::model::OperationStatus;

/// Queue depth of the normal lane for droppable items (measurements).
pub const QUEUE_CAPACITY: usize = 256;
/// Absolute limit of queued items per lane; beyond it `enqueue` fails with
/// [`EnqueueError::Full`] (a tenant that did not answer for thousands of events).
pub const HARD_CAP: usize = 4096;
/// Result tag that reports a dead lane (never a real item tag).
pub const LANE_DEAD_TAG: u32 = u32::MAX;

/// One thing to send.
#[derive(Debug, Clone, PartialEq)]
pub enum OutboundPayload {
    /// `POST /measurement/measurements` (a `measurementCollection` body or a single one).
    Measurements(Value),
    /// `POST /alarm/alarms`.
    Alarm(Value),
    /// `PUT /alarm/alarms/{id}` `{ "status": "CLEARED" }`.
    AlarmClear { alarm_id: String },
    /// Clear every open alarm of the device ("Clear all", D9): bulk update, then one by one for
    /// what is left (`C8yClient::clear_open_alarms`).
    ClearAlarms,
    /// `GET` the open alarms of the device (after a failed "Clear all": which ones are still
    /// open?). The answer is `{ "alarms": [...] }`.
    ReadOpenAlarms,
    /// `POST /event/events`.
    Event(Value),
    /// `PUT /devicecontrol/operations/{id}` `{ status, failureReason? }`.
    OperationStatus {
        operation_id: String,
        status: OperationStatus,
        failure_reason: Option<String>,
    },
    /// `PUT /inventory/managedObjects/{device}` with a partial body (inventory effects, snapshot).
    InventoryPatch(Value),
    /// Refresh the advisory lock of `user`: read the device first and fail with `DeviceLocked`
    /// when somebody else took the lock over, else write `lw_demoLock`.
    LockRefresh { user: String },
    /// Release the advisory lock of `user` (a lock of somebody else is left alone).
    LockRelease { user: String },
}

/// Which worker sends a payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    Normal,
    Priority,
}

/// How a queued payload is treated when the queue is full.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// Never dropped.
    Critical,
    /// A measurement sent because of a change; droppable (the next periodic send repeats it).
    Immediate,
    /// A measurement of a periodic full send; dropped first, superseded by the next full send.
    Periodic,
}

impl OutboundPayload {
    /// Default lane: `EXECUTING` and the lock go through the concurrent priority lane,
    /// everything else (incl. final statuses) through the ordered normal lane.
    pub fn default_lane(&self) -> Lane {
        match self {
            Self::OperationStatus {
                status: OperationStatus::Executing,
                ..
            }
            | Self::LockRefresh { .. }
            | Self::LockRelease { .. } => Lane::Priority,
            _ => Lane::Normal,
        }
    }

    /// Default class: measurements are droppable, the rest is not.
    pub fn default_class(&self) -> Class {
        match self {
            Self::Measurements(_) => Class::Immediate,
            _ => Class::Critical,
        }
    }
}

/// A queued payload; `tag` is chosen by the state loop and echoed in the result.
#[derive(Debug, Clone, PartialEq)]
pub struct OutboundItem {
    pub tag: u32,
    pub payload: OutboundPayload,
    pub lane: Lane,
    pub class: Class,
}

impl OutboundItem {
    /// An item with the payload's default lane and class.
    pub fn new(tag: u32, payload: OutboundPayload) -> Self {
        Self {
            tag,
            lane: payload.default_lane(),
            class: payload.default_class(),
            payload,
        }
    }
}

/// What happened to one queued payload.
#[derive(Debug)]
pub struct OutboundResult {
    pub tag: u32,
    /// The decoded response body (e.g. the created alarm with its id) or the error.
    pub result: AppResult<Value>,
}

/// Why [`OutboundQueue::enqueue`] refused an item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnqueueError {
    /// The lane's task has ended: nothing will be sent any more (fatal for the session).
    Closed,
    /// [`HARD_CAP`] reached.
    Full,
}

impl From<EnqueueError> for AppError {
    fn from(e: EnqueueError) -> Self {
        match e {
            EnqueueError::Closed => AppError::Internal("outbound worker stopped".into()),
            EnqueueError::Full => AppError::Internal("outbound queue is full".into()),
        }
    }
}

#[derive(Debug, Default)]
struct Inner {
    normal: VecDeque<OutboundItem>,
    priority: VecDeque<OutboundItem>,
    normal_closed: bool,
    priority_closed: bool,
}

#[derive(Debug, Default)]
struct Shared {
    inner: Mutex<Inner>,
    normal_ready: Notify,
    priority_ready: Notify,
}

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn pop(&self, lane: Lane) -> Option<OutboundItem> {
        let mut g = self.lock();
        match lane {
            Lane::Normal => g.normal.pop_front(),
            Lane::Priority => g.priority.pop_front(),
        }
    }

    async fn next(&self, lane: Lane) -> OutboundItem {
        loop {
            if let Some(i) = self.pop(lane) {
                return i;
            }
            match lane {
                Lane::Normal => self.normal_ready.notified().await,
                Lane::Priority => self.priority_ready.notified().await,
            }
        }
    }

    fn close(&self, lane: Lane) {
        let mut g = self.lock();
        match lane {
            Lane::Normal => g.normal_closed = true,
            Lane::Priority => g.priority_closed = true,
        }
    }
}

/// Cloneable, non-blocking enqueue side.
#[derive(Debug, Clone)]
pub struct OutboundQueue {
    shared: Arc<Shared>,
}

impl OutboundQueue {
    fn new() -> Self {
        Self {
            shared: Arc::default(),
        }
    }

    /// Queue an item on its lane without waiting. Returns the tags of queued measurements that
    /// were dropped to make room (periodic ones first, then immediate ones; never anything
    /// else); the caller forgets their purposes.
    pub fn enqueue(&self, item: OutboundItem) -> Result<Vec<u32>, EnqueueError> {
        let mut g = self.shared.lock();
        let lane = item.lane;
        let (closed, len) = match lane {
            Lane::Normal => (g.normal_closed, g.normal.len()),
            Lane::Priority => (g.priority_closed, g.priority.len()),
        };
        if closed {
            return Err(EnqueueError::Closed);
        }
        if len >= HARD_CAP {
            return Err(EnqueueError::Full);
        }
        let mut dropped = Vec::new();
        match lane {
            Lane::Priority => g.priority.push_back(item),
            Lane::Normal => {
                g.normal.push_back(item);
                let droppable = |i: &OutboundItem| i.class != Class::Critical;
                while g.normal.iter().filter(|i| droppable(i)).count() > QUEUE_CAPACITY {
                    let pos = g
                        .normal
                        .iter()
                        .position(|i| i.class == Class::Periodic)
                        .or_else(|| g.normal.iter().position(|i| i.class == Class::Immediate));
                    match pos.and_then(|p| g.normal.remove(p)) {
                        Some(i) => dropped.push(i.tag),
                        None => break,
                    }
                }
            }
        }
        drop(g);
        match lane {
            Lane::Normal => self.shared.normal_ready.notify_one(),
            Lane::Priority => self.shared.priority_ready.notify_one(),
        }
        Ok(dropped)
    }

    fn remove_where(&self, f: impl Fn(&OutboundItem) -> bool) -> Vec<u32> {
        let mut g = self.shared.lock();
        let mut dropped = Vec::new();
        g.normal.retain(|i| {
            if f(i) {
                dropped.push(i.tag);
                false
            } else {
                true
            }
        });
        dropped
    }

    /// Close a lane as a dead worker task would (tests).
    #[cfg(test)]
    pub fn close_lane_for_test(&self, lane: Lane) {
        self.shared.close(lane);
    }

    /// Remove every queued periodic measurement (a new full send replaces them).
    pub fn supersede_periodic(&self) -> Vec<u32> {
        self.remove_where(|i| i.class == Class::Periodic)
    }

    /// Remove every queued measurement (the session stops: lock release and final statuses must
    /// not wait behind data nobody needs).
    pub fn clear_measurements(&self) -> Vec<u32> {
        self.remove_where(|i| i.class != Class::Critical)
    }

    /// Queued items per lane `(normal, priority)`.
    pub fn len(&self) -> (usize, usize) {
        let g = self.shared.lock();
        (g.normal.len(), g.priority.len())
    }

    /// No item is queued.
    pub fn is_empty(&self) -> bool {
        self.len() == (0, 0)
    }
}

/// The worker tasks plus their channels. Dropping it aborts the tasks.
#[derive(Debug)]
pub struct OutboundWorker {
    pub queue: OutboundQueue,
    /// One result per queued item (in order per lane for the normal lane).
    pub results: mpsc::UnboundedReceiver<OutboundResult>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl OutboundWorker {
    /// Abort the task of `lane` (tests simulate a lane that dies).
    #[cfg(test)]
    pub fn kill_lane(&self, lane: Lane) {
        let i = match lane {
            Lane::Normal => 0,
            Lane::Priority => 1,
        };
        self.tasks[i].abort();
    }
}

impl Drop for OutboundWorker {
    fn drop(&mut self) {
        for t in &self.tasks {
            t.abort();
        }
    }
}

/// Closes the lane and reports it when the task ends for any reason (also on panic / abort).
struct LaneGuard {
    shared: Arc<Shared>,
    lane: Lane,
    results_tx: mpsc::UnboundedSender<OutboundResult>,
}

impl Drop for LaneGuard {
    fn drop(&mut self) {
        self.shared.close(self.lane);
        let _ = self.results_tx.send(OutboundResult {
            tag: LANE_DEAD_TAG,
            result: Err(AppError::Internal("outbound lane stopped".into())),
        });
    }
}

fn normal_task(
    queue: OutboundQueue,
    client: C8yClient,
    device_id: String,
    results_tx: mpsc::UnboundedSender<OutboundResult>,
) -> tokio::task::JoinHandle<()> {
    // Built before the task starts: it also fires when the task is aborted before its first poll.
    let guard = LaneGuard {
        shared: queue.shared.clone(),
        lane: Lane::Normal,
        results_tx: results_tx.clone(),
    };
    tokio::spawn(async move {
        let _guard = guard;
        loop {
            let item = queue.shared.next(Lane::Normal).await;
            let result = send(&client, &device_id, &item.payload).await;
            if results_tx
                .send(OutboundResult {
                    tag: item.tag,
                    result,
                })
                .is_err()
            {
                break;
            }
        }
    })
}

fn priority_task(
    queue: OutboundQueue,
    client: C8yClient,
    device_id: String,
    results_tx: mpsc::UnboundedSender<OutboundResult>,
) -> tokio::task::JoinHandle<()> {
    let guard = LaneGuard {
        shared: queue.shared.clone(),
        lane: Lane::Priority,
        results_tx: results_tx.clone(),
    };
    tokio::spawn(async move {
        let _guard = guard;
        // Dropping the set (task abort) aborts the requests in flight.
        let mut set: JoinSet<()> = JoinSet::new();
        loop {
            tokio::select! {
                item = queue.shared.next(Lane::Priority) => {
                    let (client, device_id, tx) = (client.clone(), device_id.clone(), results_tx.clone());
                    set.spawn(async move {
                        let result = send(&client, &device_id, &item.payload).await;
                        let _ = tx.send(OutboundResult { tag: item.tag, result });
                    });
                }
                Some(_) = set.join_next(), if !set.is_empty() => {}
            }
        }
    })
}

/// Start the workers for `device_id` (must be called inside a tokio runtime).
pub fn spawn(client: C8yClient, device_id: String) -> OutboundWorker {
    let queue = OutboundQueue::new();
    let (results_tx, results) = mpsc::unbounded_channel();
    let tasks = vec![
        normal_task(
            queue.clone(),
            client.clone(),
            device_id.clone(),
            results_tx.clone(),
        ),
        priority_task(queue.clone(), client, device_id, results_tx),
    ];
    OutboundWorker {
        queue,
        results,
        tasks,
    }
}

/// Reads queued items like the worker would (priority lane first); the test answers through the
/// results sender.
#[cfg(test)]
#[derive(Debug)]
pub struct TestOut {
    queue: OutboundQueue,
}

#[cfg(test)]
impl TestOut {
    /// Next queued item (priority lane first), if any.
    pub fn try_recv(&mut self) -> Result<OutboundItem, mpsc::error::TryRecvError> {
        self.queue
            .shared
            .pop(Lane::Priority)
            .or_else(|| self.queue.shared.pop(Lane::Normal))
            .ok_or(mpsc::error::TryRecvError::Empty)
    }
}

/// Test double: no HTTP. The state loop enqueues into the real queue (same drop policy, same
/// lanes); the test drains it through [`TestOut`] and answers through the returned results
/// sender.
#[cfg(test)]
pub fn test_pair() -> (
    OutboundWorker,
    TestOut,
    mpsc::UnboundedSender<OutboundResult>,
) {
    let queue = OutboundQueue::new();
    let (results_tx, results) = mpsc::unbounded_channel();
    (
        OutboundWorker {
            queue: queue.clone(),
            results,
            tasks: Vec::new(),
        },
        TestOut { queue },
        results_tx,
    )
}

/// Execute one payload against Cumulocity. Measurement bodies with `measurements` get the
/// collection media type from the client (`c8y::media::for_request`).
async fn send(client: &C8yClient, device_id: &str, payload: &OutboundPayload) -> AppResult<Value> {
    match payload {
        OutboundPayload::Measurements(body) => {
            client.post_json("/measurement/measurements", body).await
        }
        OutboundPayload::Alarm(body) => client.create_alarm(body).await,
        OutboundPayload::AlarmClear { alarm_id } => client.clear_alarm(alarm_id).await,
        OutboundPayload::ClearAlarms => client
            .clear_open_alarms(device_id)
            .await
            .map(|n| serde_json::json!({ "cleared": n })),
        OutboundPayload::ReadOpenAlarms => client
            .open_alarms(device_id, None)
            .await
            .map(|alarms| serde_json::json!({ "alarms": alarms })),
        OutboundPayload::Event(body) => client.create_event(body).await,
        OutboundPayload::OperationStatus {
            operation_id,
            status,
            failure_reason,
        } => {
            // One retry on a transport error (the status `PUT` has a short timeout).
            match client
                .set_operation_status(operation_id, *status, failure_reason.as_deref())
                .await
            {
                Err(AppError::Network(_)) => {
                    client
                        .set_operation_status(operation_id, *status, failure_reason.as_deref())
                        .await
                }
                other => other,
            }
        }
        OutboundPayload::InventoryPatch(body) => {
            client.update_managed_object(device_id, body).await
        }
        OutboundPayload::LockRefresh { user } => {
            let mo = client.get_managed_object(device_id).await?;
            if let Some(l) = demo_state::foreign_lock(&mo, user, chrono::Utc::now()) {
                return Err(AppError::DeviceLocked {
                    user: l.user,
                    since: l.since,
                });
            }
            let lock = demo_state::new_lock(user);
            client
                .update_managed_object(device_id, &demo_state::lock_body(Some(&lock))?)
                .await
        }
        OutboundPayload::LockRelease { user } => {
            let mo = client.get_managed_object(device_id).await?;
            if demo_state::foreign_lock(&mo, user, chrono::Utc::now()).is_some() {
                // Somebody took the device over: their lock stays.
                return Ok(Value::Null);
            }
            client
                .update_managed_object(device_id, &demo_state::lock_body(None)?)
                .await
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;

    use super::*;
    use crate::c8y::fake::FakeC8y;

    fn item(tag: u32, payload: OutboundPayload, class: Class) -> OutboundItem {
        let mut i = OutboundItem::new(tag, payload);
        i.class = class;
        i
    }

    fn m(tag: u32, class: Class) -> OutboundItem {
        item(tag, OutboundPayload::Measurements(json!({})), class)
    }

    fn tags(q: &OutboundQueue) -> Vec<u32> {
        q.shared.lock().normal.iter().map(|i| i.tag).collect()
    }

    #[test]
    fn full_queue_drops_periodic_then_immediate_but_never_critical() {
        let q = OutboundQueue::new();
        for t in 0..QUEUE_CAPACITY as u32 {
            assert!(q.enqueue(m(t, Class::Periodic)).unwrap().is_empty());
        }
        // Alarms, events and statuses are not droppable and do not count against the capacity.
        let alarm = item(1000, OutboundPayload::Alarm(json!({})), Class::Critical);
        let event = item(1001, OutboundPayload::Event(json!({})), Class::Critical);
        let status = item(
            1002,
            OutboundPayload::OperationStatus {
                operation_id: "1".into(),
                status: OperationStatus::Successful,
                failure_reason: None,
            },
            Class::Critical,
        );
        for i in [alarm, event, status] {
            assert!(q.enqueue(i).unwrap().is_empty());
        }
        // An immediate measurement pushes out the OLDEST PERIODIC one.
        assert_eq!(q.enqueue(m(2000, Class::Immediate)).unwrap(), vec![0]);
        // Once no periodic one is left, the oldest immediate one goes.
        let q2 = OutboundQueue::new();
        for t in 0..QUEUE_CAPACITY as u32 {
            q2.enqueue(m(t, Class::Immediate)).unwrap();
        }
        assert_eq!(q2.enqueue(m(999, Class::Immediate)).unwrap(), vec![0]);
        // A flood of measurements never displaces a critical item.
        for t in 3000..3600 {
            q.enqueue(m(t, Class::Immediate)).unwrap();
        }
        let kept = tags(&q);
        for critical in [1000, 1001, 1002] {
            assert!(kept.contains(&critical), "{critical} was dropped");
        }
        assert_eq!(
            kept.iter().filter(|t| **t < 1000).count(),
            0,
            "periodic ones went first"
        );
    }

    #[test]
    fn periodic_supersede_and_clear_on_stop() {
        let q = OutboundQueue::new();
        q.enqueue(m(1, Class::Periodic)).unwrap();
        q.enqueue(m(2, Class::Immediate)).unwrap();
        q.enqueue(item(3, OutboundPayload::Event(json!({})), Class::Critical))
            .unwrap();
        q.enqueue(m(4, Class::Periodic)).unwrap();
        assert_eq!(q.supersede_periodic(), vec![1, 4]);
        assert_eq!(tags(&q), vec![2, 3]);
        assert_eq!(q.clear_measurements(), vec![2]);
        assert_eq!(tags(&q), vec![3], "the event stays");
    }

    #[test]
    fn closed_lane_and_hard_cap_refuse_items() {
        let q = OutboundQueue::new();
        for t in 0..HARD_CAP as u32 {
            q.enqueue(item(t, OutboundPayload::Event(json!({})), Class::Critical))
                .unwrap();
        }
        assert_eq!(
            q.enqueue(item(1, OutboundPayload::Event(json!({})), Class::Critical)),
            Err(EnqueueError::Full)
        );
        let q = OutboundQueue::new();
        q.close_lane_for_test(Lane::Normal);
        assert_eq!(q.enqueue(m(1, Class::Immediate)), Err(EnqueueError::Closed));
        // The other lane still works.
        let exec = OutboundPayload::OperationStatus {
            operation_id: "1".into(),
            status: OperationStatus::Executing,
            failure_reason: None,
        };
        assert!(q.enqueue(OutboundItem::new(2, exec)).is_ok());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn priority_lane_is_not_held_up_by_a_hanging_normal_request() {
        let fake = FakeC8y::start().await;
        let op = fake.create_operation("7", json!({ "lw_switchGroupMode": {} }));
        let op_id = crate::c8y::id_of(&op).unwrap();
        fake.delay("POST", "/measurement", Duration::from_millis(1200));
        let mut w = spawn(fake.client(), "7".into());
        let ms = json!({ "measurements": [{ "type": "t", "time": "2026-10-12T12:00:00.000Z",
                                              "source": { "id": "7" }, "x": { "y": { "value": 1 } } }] });
        w.queue
            .enqueue(OutboundItem::new(1, OutboundPayload::Measurements(ms)))
            .unwrap();
        w.queue
            .enqueue(OutboundItem::new(
                2,
                OutboundPayload::OperationStatus {
                    operation_id: op_id,
                    status: OperationStatus::Executing,
                    failure_reason: None,
                },
            ))
            .unwrap();
        let first = tokio::time::timeout(Duration::from_secs(5), w.results.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.tag, 2, "EXECUTING overtakes the slow measurement");
        assert!(first.result.is_ok());
        let second = tokio::time::timeout(Duration::from_secs(5), w.results.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(second.tag, 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn normal_lane_keeps_its_order() {
        let fake = FakeC8y::start().await;
        let mut w = spawn(fake.client(), "7".into());
        for t in 1..=5u32 {
            w.queue
                .enqueue(item(
                    t,
                    OutboundPayload::Event(
                        json!({ "type": "e", "text": t.to_string(), "source": { "id": "7" } }),
                    ),
                    Class::Critical,
                ))
                .unwrap();
        }
        for t in 1..=5u32 {
            let r = tokio::time::timeout(Duration::from_secs(5), w.results.recv())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(r.tag, t);
        }
        let texts: Vec<String> = fake
            .state()
            .events
            .iter()
            .map(|e| e["text"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(texts, vec!["1", "2", "3", "4", "5"]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_dead_lane_is_reported_and_closed() {
        let fake = FakeC8y::start().await;
        let mut w = spawn(fake.client(), "7".into());
        w.kill_lane(Lane::Normal);
        let r = tokio::time::timeout(Duration::from_secs(5), w.results.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(r.tag, LANE_DEAD_TAG);
        assert!(r.result.is_err());
        assert_eq!(
            w.queue.enqueue(m(1, Class::Immediate)),
            Err(EnqueueError::Closed)
        );
    }

    /// Finding 10: the lock refresh writes only the lock fragment, and notices a takeover.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn lock_refresh_leaves_the_setup_alone_and_notices_a_takeover() {
        let fake = FakeC8y::start().await;
        let dev = fake.insert_managed_object(json!({ "name": "d", "lw_demo": { "marker": 1 } }));
        let mut w = spawn(fake.client(), dev.clone());
        let mine = |tag| {
            OutboundItem::new(
                tag,
                OutboundPayload::LockRefresh {
                    user: "demo".into(),
                },
            )
        };
        w.queue.enqueue(mine(1)).unwrap();
        let r = w.results.recv().await.unwrap();
        assert!(r.result.is_ok());
        {
            let s = fake.state();
            let mo = s
                .managed_objects
                .iter()
                .find(|m| m["id"] == dev.as_str())
                .unwrap();
            assert_eq!(mo["lw_demo"], json!({ "marker": 1 }), "lw_demo untouched");
            assert_eq!(mo["lw_demoLock"]["user"], "demo");
        }
        // Somebody else takes the device over.
        fake.client()
            .update_managed_object(
                &dev,
                &crate::device::demo_state::lock_body(Some(&crate::device::demo_state::new_lock(
                    "other",
                )))
                .unwrap(),
            )
            .await
            .unwrap();
        w.queue.enqueue(mine(2)).unwrap();
        let r = w.results.recv().await.unwrap();
        assert!(
            matches!(r.result, Err(AppError::DeviceLocked { ref user, .. }) if user == "other")
        );
        // Releasing leaves their lock alone.
        w.queue
            .enqueue(OutboundItem::new(
                3,
                OutboundPayload::LockRelease {
                    user: "demo".into(),
                },
            ))
            .unwrap();
        w.results.recv().await.unwrap();
        let mo = fake.client().get_managed_object(&dev).await.unwrap();
        assert_eq!(mo["lw_demoLock"]["user"], "other");
    }
}
