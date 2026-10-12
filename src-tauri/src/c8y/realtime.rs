//! Cumulocity realtime notifications: Bayeux/CometD long-polling over HTTP on
//! `POST /notification/realtime`.
//!
//! Contract:
//! - [`subscribe_device`] spawns the connection task and returns a
//!   [`RealtimeSubscription`] right away (the first handshake result arrives as a message);
//!   dropping it cancels the task (cancel-on-drop) and sends a best-effort `/meta/disconnect`
//!   (2 s limit), so a session that ends can never leak a subscription.
//! - The subscription yields [`RealtimeMsg`]s: `Connected`, `Disconnected(error)`,
//!   `Operation(json)` for every operation create/update of the device and `Alarm(json)` for
//!   every alarm create/update (a deleted alarm arrives as `CLEARED`; DECISIONS D9).
//! - On every (re)connect, after subscribing, the task also `GET`s the device's open alarms
//!   (ACTIVE + ACKNOWLEDGED) and delivers them as one `OpenAlarms` message (with the instant
//!   taken just before the request), so the session can notice a clear it missed while the
//!   connection was down.
//! - On **every** (re)connect the task (after subscribing, so there is no gap) `GET`s the
//!   device's PENDING operations and feeds them as `Operation` messages right after
//!   `Connected`. An operation can arrive twice (PENDING list + realtime, or CREATE + UPDATE),
//!   so the worker de-duplicates per subscription ([`OpDedup`]): a message is dropped unless
//!   its status is *further along* than the last one delivered for that id
//!   (PENDING < EXECUTING < SUCCESSFUL < FAILED; FAILED wins, D5). The session must still
//!   ignore the UPDATE echo of its **own** status changes (EXECUTING / SUCCESSFUL / FAILED it
//!   just PUT): those are new statuses for the worker, so they are delivered.
//! - Flow: handshake → subscribe `/operations/<deviceId>` and `/alarms/<deviceId>` → GET PENDING
//!   operations + open alarms → connect loop. A connect reply with
//!   `402::Unknown client` or `advice.reconnect = "handshake"` re-handshakes (the first time in
//!   a row at once, no `Disconnected`; further ones wait with the exponential backoff, so a
//!   server that keeps answering 402 is not hammered). Transport / HTTP errors emit
//!   `Disconnected` and retry with backoff ([`RealtimeConfig`]). The backoff resets only after a
//!   successful `/meta/connect` reply (or a phase that lived longer than `backoff_max`).
//!   `Connected` is emitted right after the PENDING `GET`, not after the first connect reply
//!   (that one can be held for the whole long-poll).
//! - Server advice is honoured: `interval` is slept before each connect, `timeout` raises the
//!   HTTP timeout to `max(poll_timeout, advice.timeout) + poll_grace`, `reconnect: none` ends
//!   the task with `Disconnected`.
//! - A long-poll that hits the client-side timeout is simply re-issued; two in a row count as a
//!   lost connection. The per-request timeout overrides the 30 s REST default.
//! - Terminal errors: 401 (reports `auth-expired` once; retrying would only add failed logins)
//!   and 403 (missing role) emit `Disconnected` and end the task.
//! - Logging: handshake, subscribe and connects that carried data or failed go to the
//!   activity log; quiet successful long-polls are not logged (they would flood the 500-entry
//!   log).
//! - The session maps `Connected` / `Disconnected` to the `realtime-status` event.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use reqwest::Method;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::client::{C8yClient, RequestSpec, map_transport};
use super::media;
use crate::error::{AppError, AppResult};

/// A raw operation (`data` of a realtime message, or an element of the PENDING query).
pub type OperationNotification = serde_json::Value;

/// Realtime endpoint.
pub const REALTIME_PATH: &str = "/notification/realtime";

/// Time limit of the best-effort `/meta/disconnect` on cancel.
const DISCONNECT_TIMEOUT: Duration = Duration::from_secs(2);
/// Upper bound of a server-requested `advice.interval` sleep.
const MAX_ADVICE_INTERVAL: Duration = Duration::from_secs(60);
/// The de-duplication map is reset beyond this many operations.
const DEDUP_LIMIT: usize = 5000;

/// What a subscription reports.
#[derive(Debug, Clone, PartialEq)]
pub enum RealtimeMsg {
    /// Connection (re)established; the PENDING operations follow as `PendingOperation` messages.
    Connected,
    /// Connection lost or handshake failed; the task keeps retrying unless the error is
    /// terminal (401, 403, `reconnect: none`; then the channel closes). Carries the error text.
    Disconnected(String),
    /// An operation of the device (raw JSON, as returned by Cumulocity), delivered live.
    Operation(OperationNotification),
    /// An operation from the PENDING list fetched on (re)connect: it may have been created while
    /// nobody was listening, so its age counts ("Simulator was offline"). Live operations are
    /// never old, whatever the clocks say.
    PendingOperation(OperationNotification),
    /// An alarm of the device was created or changed (raw JSON with `id`, `type`, `status`). A
    /// deleted alarm arrives as `{ "id", "status": "CLEARED" }`.
    Alarm(serde_json::Value),
    /// The device's open alarms (ACTIVE + ACKNOWLEDGED), fetched on every (re)connect after
    /// subscribing. `fetched_at` is taken just before the request: an alarm the consumer knew
    /// before that instant and that is missing here was cleared (or deleted) meanwhile.
    OpenAlarms {
        alarms: Vec<serde_json::Value>,
        fetched_at: tokio::time::Instant,
    },
}

/// Timing knobs (tests use short ones).
#[derive(Debug, Clone)]
pub struct RealtimeConfig {
    /// Long-poll duration requested via `advice.timeout` on connect.
    pub poll_timeout: Duration,
    /// Extra time on top of `poll_timeout` (or the server's `advice.timeout`) before the HTTP
    /// request times out. Overrides the client's 30 s REST timeout.
    pub poll_grace: Duration,
    /// First retry delay after an error; doubles up to `backoff_max`.
    pub backoff_min: Duration,
    pub backoff_max: Duration,
}

impl Default for RealtimeConfig {
    fn default() -> Self {
        Self {
            poll_timeout: Duration::from_secs(50),
            poll_grace: Duration::from_secs(15),
            backoff_min: Duration::from_secs(1),
            backoff_max: Duration::from_secs(30),
        }
    }
}

/// Drops operation messages that carry no news (see the module docs).
#[derive(Debug, Default)]
pub struct OpDedup {
    last: HashMap<String, u8>,
}

impl OpDedup {
    fn rank(status: &str) -> Option<u8> {
        match status {
            "PENDING" => Some(0),
            "EXECUTING" => Some(1),
            "SUCCESSFUL" => Some(2),
            "FAILED" => Some(3),
            _ => None,
        }
    }

    /// `true` if `op` should be delivered. Messages without an id or with an unknown status
    /// always pass.
    pub fn admit(&mut self, op: &Value) -> bool {
        let id = op.get("id").and_then(|v| match v {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        });
        let rank = op
            .get("status")
            .and_then(Value::as_str)
            .and_then(Self::rank);
        let (Some(id), Some(rank)) = (id, rank) else {
            return true;
        };
        if self.last.len() >= DEDUP_LIMIT && !self.last.contains_key(&id) {
            self.last.clear();
        }
        match self.last.get(&id) {
            Some(&seen) if rank <= seen => false,
            _ => {
                self.last.insert(id, rank);
                true
            }
        }
    }
}

/// A running subscription. Dropping it cancels the task.
#[derive(Debug)]
pub struct RealtimeSubscription {
    rx: mpsc::Receiver<RealtimeMsg>,
    cancel: CancellationToken,
    /// Kept so the task is owned; it ends by itself once `cancel` fires (after the disconnect).
    _task: tauri::async_runtime::JoinHandle<()>,
}

impl RealtimeSubscription {
    /// Wire a subscription around a spawned task (used by `subscribe_operations`).
    pub fn from_parts(
        rx: mpsc::Receiver<RealtimeMsg>,
        cancel: CancellationToken,
        task: tauri::async_runtime::JoinHandle<()>,
    ) -> Self {
        Self {
            rx,
            cancel,
            _task: task,
        }
    }

    /// Next message; `None` once the task has ended.
    pub async fn recv(&mut self) -> Option<RealtimeMsg> {
        self.rx.recv().await
    }

    /// Stop the task (also done on drop).
    pub fn cancel(&self) {
        self.cancel.cancel();
    }
}

impl Drop for RealtimeSubscription {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// Subscribe to `/operations/<device_id>` and `/alarms/<device_id>` with default timing.
pub fn subscribe_device(client: &C8yClient, device_id: &str) -> AppResult<RealtimeSubscription> {
    subscribe_device_with(client, device_id, RealtimeConfig::default())
}

/// Subscribe with explicit timing (tests).
pub fn subscribe_device_with(
    client: &C8yClient,
    device_id: &str,
    config: RealtimeConfig,
) -> AppResult<RealtimeSubscription> {
    if device_id.is_empty() {
        return Err(AppError::InvalidInput("empty device id".into()));
    }
    let (tx, rx) = mpsc::channel(256);
    let cancel = CancellationToken::new();
    let worker = Arc::new(Worker {
        client: client.clone(),
        device_id: device_id.to_owned(),
        channel: bayeux::operations_channel(device_id),
        alarm_channel: bayeux::alarms_channel(device_id),
        config,
        tx,
        client_id: Mutex::new(None),
        dedup: Mutex::new(OpDedup::default()),
    });
    let token = cancel.clone();
    let task = tauri::async_runtime::spawn(async move {
        let runner = worker.clone();
        tokio::select! {
            () = token.cancelled() => worker.disconnect().await,
            () = runner.run() => {}
        }
    });
    Ok(RealtimeSubscription::from_parts(rx, cancel, task))
}

/// Pure Bayeux message building / parsing.
pub mod bayeux {
    use serde_json::{Value, json};

    pub fn operations_channel(device_id: &str) -> String {
        format!("/operations/{device_id}")
    }

    pub fn alarms_channel(device_id: &str) -> String {
        format!("/alarms/{device_id}")
    }

    pub fn handshake(timeout_ms: u64) -> Value {
        json!([{
            "channel": "/meta/handshake",
            "version": "1.0",
            "minimumVersion": "1.0",
            "supportedConnectionTypes": ["long-polling"],
            "advice": { "timeout": timeout_ms, "interval": 0 }
        }])
    }

    pub fn subscribe(client_id: &str, subscription: &str) -> Value {
        json!([{
            "channel": "/meta/subscribe",
            "clientId": client_id,
            "subscription": subscription
        }])
    }

    pub fn connect(client_id: &str, timeout_ms: u64) -> Value {
        json!([{
            "channel": "/meta/connect",
            "clientId": client_id,
            "connectionType": "long-polling",
            "advice": { "timeout": timeout_ms, "interval": 0 }
        }])
    }

    pub fn disconnect(client_id: &str) -> Value {
        json!([{ "channel": "/meta/disconnect", "clientId": client_id }])
    }

    /// The reply to one meta message.
    #[derive(Debug, Clone, PartialEq, Default)]
    pub struct MetaReply {
        pub successful: bool,
        pub client_id: Option<String>,
        pub error: Option<String>,
        /// `advice.reconnect`: `retry` | `handshake` | `none`.
        pub reconnect: Option<String>,
        /// `advice.interval` (ms): wait before the next connect.
        pub interval_ms: Option<u64>,
        /// `advice.timeout` (ms): how long the server holds a connect.
        pub timeout_ms: Option<u64>,
    }

    impl MetaReply {
        /// The server forgot the client (402) or asks for a new handshake.
        pub fn needs_handshake(&self) -> bool {
            self.reconnect.as_deref() == Some("handshake")
                || self
                    .error
                    .as_deref()
                    .is_some_and(|e| e.starts_with("402") || e.contains("nknown client"))
        }

        /// The server tells us not to come back (`advice.reconnect = none`).
        pub fn reconnect_none(&self) -> bool {
            self.reconnect.as_deref() == Some("none")
        }

        /// `403::Forbidden` (missing role).
        pub fn is_forbidden(&self) -> bool {
            self.error.as_deref().is_some_and(|e| e.starts_with("403"))
        }

        pub fn error_text(&self) -> String {
            self.error
                .clone()
                .unwrap_or_else(|| "unsuccessful".to_owned())
        }
    }

    /// A parsed response array.
    #[derive(Debug, Clone, PartialEq, Default)]
    pub struct Reply {
        /// Reply to `meta_channel`, if present.
        pub meta: Option<MetaReply>,
        /// Operations delivered on the device's operation channel (CREATE / UPDATE; DELETE
        /// skipped).
        pub operations: Vec<Value>,
        /// Alarms delivered on the device's alarm channel (CREATE / UPDATE; a DELETE becomes
        /// `{ "id", "status": "CLEARED" }`).
        pub alarms: Vec<Value>,
    }

    /// Parse a Bayeux response (array, or a single object) for `meta_channel`
    /// (`/meta/connect` …) and data messages of `device_id` (`/operations/<id>`,
    /// `/alarms/<id>`).
    pub fn parse(body: &Value, meta_channel: &str, device_id: &str) -> Reply {
        let ops_channel = operations_channel(device_id);
        let alarm_channel = alarms_channel(device_id);
        let msgs: Vec<&Value> = match body {
            Value::Array(a) => a.iter().collect(),
            Value::Object(_) => vec![body],
            _ => Vec::new(),
        };
        let mut reply = Reply::default();
        for m in msgs {
            let channel = m.get("channel").and_then(Value::as_str).unwrap_or("");
            if channel == meta_channel {
                reply.meta = Some(MetaReply {
                    successful: m
                        .get("successful")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    client_id: m.get("clientId").and_then(Value::as_str).map(str::to_owned),
                    error: m.get("error").and_then(Value::as_str).map(str::to_owned),
                    reconnect: m
                        .pointer("/advice/reconnect")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    interval_ms: m.pointer("/advice/interval").and_then(Value::as_u64),
                    timeout_ms: m.pointer("/advice/timeout").and_then(Value::as_u64),
                });
            } else if channel == ops_channel
                && let Some(op) = operation_of(m.get("data"))
            {
                reply.operations.push(op);
            } else if channel == alarm_channel
                && let Some(alarm) = alarm_of(m.get("data"))
            {
                reply.alarms.push(alarm);
            }
        }
        reply
    }

    /// Like [`operation_of`], but a DELETE (data = the id) counts as a clear: the alarm is gone.
    fn alarm_of(data: Option<&Value>) -> Option<Value> {
        let data = data?;
        if data.get("realtimeAction").and_then(Value::as_str) == Some("DELETE") {
            let id = match data.get("data")? {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Object(o) => match o.get("id")? {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => n.to_string(),
                    _ => return None,
                },
                _ => return None,
            };
            return Some(json!({ "id": id, "status": "CLEARED" }));
        }
        operation_of(Some(data))
    }

    /// Cumulocity wraps the object: `{ "realtimeAction": "CREATE"|"UPDATE"|"DELETE", "data": {...} }`.
    fn operation_of(data: Option<&Value>) -> Option<Value> {
        let data = data?;
        match data.get("realtimeAction").and_then(Value::as_str) {
            Some("DELETE") => None,
            Some(_) => data.get("data").filter(|d| d.is_object()).cloned(),
            None => data.is_object().then(|| data.clone()),
        }
    }
}

/// Why a connect phase ended.
enum PhaseEnd {
    /// Re-handshake (402 / advice): at once the first time in a row, then with backoff.
    Rehandshake,
    /// Lost: report, back off, re-handshake.
    Lost(String),
    /// Retrying cannot help (401, 403, `reconnect: none`): report and stop.
    Fatal(String),
}

/// What the current connect phase has proven.
#[derive(Default)]
struct Progress {
    /// A `/meta/connect` reply with `successful: true` arrived.
    proven: bool,
}

/// Latest server advice of this phase.
#[derive(Default)]
struct Advice {
    interval: Duration,
    timeout: Option<Duration>,
}

impl Advice {
    fn absorb(&mut self, m: &bayeux::MetaReply) {
        if let Some(i) = m.interval_ms {
            self.interval = Duration::from_millis(i).min(MAX_ADVICE_INTERVAL);
        }
        if let Some(t) = m.timeout_ms {
            self.timeout = Some(Duration::from_millis(t));
        }
    }
}

struct Worker {
    client: C8yClient,
    device_id: String,
    channel: String,
    alarm_channel: String,
    config: RealtimeConfig,
    tx: mpsc::Sender<RealtimeMsg>,
    /// Bayeux client id of the running phase (for the disconnect on cancel).
    client_id: Mutex<Option<String>>,
    dedup: Mutex<OpDedup>,
}

impl Worker {
    async fn emit(&self, msg: RealtimeMsg) -> bool {
        self.tx.send(msg).await.is_ok()
    }

    /// Deliver an operation unless it is a duplicate. `false` = receiver gone.
    async fn emit_operation(&self, op: Value, initial: bool) -> bool {
        let admit = self
            .dedup
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .admit(&op);
        if !admit {
            return true;
        }
        self.emit(if initial {
            RealtimeMsg::PendingOperation(op)
        } else {
            RealtimeMsg::Operation(op)
        })
        .await
    }

    /// Deliver the data messages of a reply: operations (de-duplicated), then alarms.
    /// `false` = receiver gone.
    async fn deliver(&self, reply: bayeux::Reply) -> bool {
        for op in reply.operations {
            if !self.emit_operation(op, false).await {
                return false;
            }
        }
        for alarm in reply.alarms {
            if !self.emit(RealtimeMsg::Alarm(alarm)).await {
                return false;
            }
        }
        true
    }

    fn set_client_id(&self, id: Option<String>) {
        *self.client_id.lock().unwrap_or_else(|p| p.into_inner()) = id;
    }

    /// Best-effort `/meta/disconnect` (cancel): the server frees the client and its
    /// subscription at once instead of at its own timeout.
    async fn disconnect(&self) {
        let id = self
            .client_id
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        if let Some(id) = id {
            let _ = self
                .post(bayeux::disconnect(&id), DISCONNECT_TIMEOUT, false)
                .await;
        }
    }

    /// POST a Bayeux message array. `always_log` = log even a quiet successful reply / timeout.
    async fn post(&self, body: Value, timeout: Duration, always_log: bool) -> Result<Value, Post> {
        let spec = RequestSpec::new(Method::POST, REALTIME_PATH)
            .json(body)
            .accept(media::JSON)
            .content_type(media::JSON)
            .timeout(timeout);
        let outcome = self.client.send_raw(spec).await;
        let entry = outcome.entry;
        match outcome.result {
            Err(e) if e.is_timeout() => {
                if always_log {
                    self.client.record(entry);
                }
                Err(Post::Timeout)
            }
            Err(e) => {
                self.client.record(entry);
                Err(Post::Failed(map_transport(e).to_string()))
            }
            Ok((status, bytes)) => {
                let parsed: Option<Value> = serde_json::from_slice(&bytes).ok();
                let interesting = always_log
                    || !(200..300).contains(&status)
                    || parsed.as_ref().is_none_or(|v| !is_quiet_connect(v));
                if interesting {
                    self.client.record(entry);
                }
                if status == 401 {
                    self.client.check_unauthorized(status);
                    return Err(Post::Unauthorized);
                }
                if status == 403 {
                    return Err(Post::Forbidden);
                }
                if !(200..300).contains(&status) {
                    return Err(Post::Failed(format!("HTTP {status}")));
                }
                parsed.ok_or_else(|| Post::Failed("invalid realtime response".into()))
            }
        }
    }

    async fn run(&self) {
        let mut backoff = self.config.backoff_min;
        let mut rehandshakes = 0u32;
        loop {
            let mut progress = Progress::default();
            let started = Instant::now();
            let end = self.connect_phase(&mut progress).await;
            self.set_client_id(None);
            let Some(end) = end else {
                return; // receiver gone
            };
            if progress.proven || started.elapsed() >= self.config.backoff_max {
                backoff = self.config.backoff_min;
                rehandshakes = 0;
            }
            match end {
                PhaseEnd::Rehandshake => {
                    if rehandshakes > 0 {
                        tokio::time::sleep(backoff).await;
                        backoff = (backoff * 2).min(self.config.backoff_max);
                    }
                    rehandshakes += 1;
                }
                PhaseEnd::Lost(err) => {
                    log::warn!("realtime {}: {err}", self.channel);
                    if !self.emit(RealtimeMsg::Disconnected(err)).await {
                        return;
                    }
                    rehandshakes = 0;
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(self.config.backoff_max);
                }
                PhaseEnd::Fatal(err) => {
                    log::warn!("realtime {} stopped: {err}", self.channel);
                    let _ = self.emit(RealtimeMsg::Disconnected(err)).await;
                    return;
                }
            }
        }
    }

    /// Handshake, subscribe, feed PENDING ops, then long-poll until something breaks.
    /// `None` = the subscription was dropped.
    async fn connect_phase(&self, progress: &mut Progress) -> Option<PhaseEnd> {
        let short = self.config.backoff_max.max(Duration::from_secs(10));
        let mut advice = Advice::default();
        let timeout_ms = u64::try_from(self.config.poll_timeout.as_millis()).unwrap_or(60_000);

        // Handshake.
        let reply = match self.post(bayeux::handshake(timeout_ms), short, true).await {
            Ok(v) => bayeux::parse(&v, "/meta/handshake", &self.device_id),
            Err(e) => return Some(e.into_end("handshake")),
        };
        let client_id = match reply.meta {
            Some(m) if m.successful && m.client_id.is_some() => {
                advice.absorb(&m);
                if m.reconnect_none() {
                    return Some(PhaseEnd::Fatal(
                        "handshake: server said reconnect none".into(),
                    ));
                }
                m.client_id.unwrap_or_default()
            }
            Some(m) if m.is_forbidden() => {
                return Some(PhaseEnd::Fatal(format!(
                    "permission denied: handshake: {}",
                    m.error_text()
                )));
            }
            Some(m) if m.reconnect_none() => {
                return Some(PhaseEnd::Fatal(format!(
                    "handshake: {} (reconnect none)",
                    m.error_text()
                )));
            }
            Some(m) => return Some(PhaseEnd::Lost(format!("handshake: {}", m.error_text()))),
            None => return Some(PhaseEnd::Lost("handshake: no reply".into())),
        };
        self.set_client_id(Some(client_id.clone()));

        // Subscribe (operations, then alarms).
        // Alarm changes are optional: a user without alarm read access still gets operations.
        let mut early = bayeux::Reply::default();
        let mut follow_alarms = true;
        for channel in [&self.channel, &self.alarm_channel] {
            let is_alarm = *channel == self.alarm_channel;
            let reply = match self
                .post(bayeux::subscribe(&client_id, channel), short, true)
                .await
            {
                Ok(v) => bayeux::parse(&v, "/meta/subscribe", &self.device_id),
                Err(Post::Forbidden) if is_alarm => {
                    log::warn!(
                        "realtime {channel}: permission denied, alarm changes are not followed"
                    );
                    follow_alarms = false;
                    continue;
                }
                Err(e) => return Some(e.into_end("subscribe")),
            };
            match &reply.meta {
                Some(m) if m.successful => advice.absorb(m),
                Some(m) if m.needs_handshake() => return Some(PhaseEnd::Rehandshake),
                Some(m) if m.is_forbidden() && is_alarm => {
                    log::warn!(
                        "realtime {channel}: permission denied ({}), alarm changes are not followed",
                        m.error_text()
                    );
                    follow_alarms = false;
                    continue;
                }
                Some(m) if m.is_forbidden() => {
                    return Some(PhaseEnd::Fatal(format!(
                        "permission denied: subscribe {channel}: {}",
                        m.error_text()
                    )));
                }
                Some(m) if m.reconnect_none() => {
                    return Some(PhaseEnd::Fatal(format!(
                        "subscribe: {} (reconnect none)",
                        m.error_text()
                    )));
                }
                Some(m) => return Some(PhaseEnd::Lost(format!("subscribe: {}", m.error_text()))),
                None => return Some(PhaseEnd::Lost("subscribe: no reply".into())),
            }
            early.operations.extend(reply.operations);
            early.alarms.extend(reply.alarms);
        }

        // PENDING operations and open alarms (after subscribing, so nothing created or changed
        // in between is missed).
        let pending = match self.client.pending_operations(&self.device_id).await {
            Ok(p) => p,
            Err(e) => return Some(fetch_end("pending operations", e)),
        };
        let fetched_at = tokio::time::Instant::now();
        // The open alarms only help the session to notice clears it missed: a failure (no read
        // access, a 5xx) must not take operations down. One retry, then go on without them.
        let mut open_alarms = None;
        if follow_alarms {
            for attempt in 0..2 {
                match self.client.open_alarms(&self.device_id, None).await {
                    Ok(a) => {
                        open_alarms = Some(a);
                        break;
                    }
                    Err(AppError::NotAuthenticated) => {
                        return Some(fetch_end("open alarms", AppError::NotAuthenticated));
                    }
                    Err(e) => {
                        log::warn!("realtime: open alarms (attempt {}): {e}", attempt + 1);
                        if matches!(e, AppError::Http { status: 403, .. }) {
                            break;
                        }
                    }
                }
            }
        }
        if !self.emit(RealtimeMsg::Connected).await {
            return None;
        }
        for op in pending {
            if !self.emit_operation(op, true).await {
                return None;
            }
        }
        if let Some(alarms) = open_alarms
            && !self
                .emit(RealtimeMsg::OpenAlarms { alarms, fetched_at })
                .await
        {
            return None;
        }
        if !self.deliver(early).await {
            return None;
        }

        // Connect loop.
        let mut timeouts = 0;
        loop {
            if !advice.interval.is_zero() {
                tokio::time::sleep(advice.interval).await;
            }
            let http_timeout = self
                .config
                .poll_timeout
                .max(advice.timeout.unwrap_or_default())
                + self.config.poll_grace;
            let body = match self
                .post(bayeux::connect(&client_id, timeout_ms), http_timeout, false)
                .await
            {
                Ok(v) => v,
                Err(Post::Timeout) => {
                    timeouts += 1;
                    if timeouts >= 2 {
                        self.disconnect().await;
                        return Some(PhaseEnd::Lost("connect: timed out".into()));
                    }
                    continue;
                }
                Err(e) => return Some(e.into_end("connect")),
            };
            timeouts = 0;
            let mut reply = bayeux::parse(&body, "/meta/connect", &self.device_id);
            let meta = reply.meta.take();
            if !self.deliver(reply).await {
                return None;
            }
            match meta {
                Some(m) if m.successful => {
                    progress.proven = true;
                    advice.absorb(&m);
                    if m.reconnect_none() {
                        return Some(PhaseEnd::Fatal("server said reconnect none".into()));
                    }
                    if m.reconnect.as_deref() == Some("handshake") {
                        return Some(PhaseEnd::Rehandshake);
                    }
                }
                Some(m) if m.needs_handshake() => return Some(PhaseEnd::Rehandshake),
                Some(m) if m.is_forbidden() => {
                    return Some(PhaseEnd::Fatal(format!(
                        "permission denied: connect: {}",
                        m.error_text()
                    )));
                }
                Some(m) if m.reconnect_none() => {
                    return Some(PhaseEnd::Fatal(format!(
                        "connect: {} (reconnect none)",
                        m.error_text()
                    )));
                }
                Some(m) => return Some(PhaseEnd::Lost(format!("connect: {}", m.error_text()))),
                // Some servers send data without the meta reply; just poll again.
                None => {}
            }
        }
    }
}

/// The phase end for a failed `GET` on (re)connect: 401 / 403 are terminal, the rest retries.
fn fetch_end(what: &str, e: AppError) -> PhaseEnd {
    match e {
        AppError::NotAuthenticated => PhaseEnd::Fatal("authentication failed (HTTP 401)".into()),
        AppError::Http {
            status: 403,
            message,
        } => PhaseEnd::Fatal(format!("permission denied: {what}: {message}")),
        e => PhaseEnd::Lost(format!("{what}: {e}")),
    }
}

/// A successful connect reply without data (not worth an activity entry).
fn is_quiet_connect(v: &Value) -> bool {
    match v {
        Value::Array(a) => a.iter().all(|m| {
            m.get("channel").and_then(Value::as_str) == Some("/meta/connect")
                && m.get("successful").and_then(Value::as_bool) == Some(true)
        }),
        _ => false,
    }
}

enum Post {
    Timeout,
    Failed(String),
    /// HTTP 401 (`auth-expired` already reported).
    Unauthorized,
    /// HTTP 403.
    Forbidden,
}

impl Post {
    /// The phase end for a failed `stage` request.
    fn into_end(self, stage: &str) -> PhaseEnd {
        match self {
            Self::Timeout => PhaseEnd::Lost(format!("{stage}: timed out")),
            Self::Failed(s) => PhaseEnd::Lost(format!("{stage}: {s}")),
            Self::Unauthorized => PhaseEnd::Fatal("authentication failed (HTTP 401)".into()),
            Self::Forbidden => PhaseEnd::Fatal(format!("permission denied: {stage}: HTTP 403")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::bayeux::*;
    use super::*;
    use serde_json::json;

    const CH: &str = "/operations/42";
    const DEV: &str = "42";

    #[test]
    fn builds_messages() {
        let h = handshake(60_000);
        assert_eq!(h[0]["channel"], "/meta/handshake");
        assert_eq!(h[0]["supportedConnectionTypes"][0], "long-polling");
        let s = subscribe("abc", CH);
        assert_eq!(s[0]["subscription"], CH);
        assert_eq!(s[0]["clientId"], "abc");
        let c = connect("abc", 5000);
        assert_eq!(c[0]["connectionType"], "long-polling");
        assert_eq!(c[0]["advice"]["timeout"], 5000);
        assert_eq!(operations_channel("42"), CH);
        assert_eq!(disconnect("abc")[0]["channel"], "/meta/disconnect");
        assert_eq!(h[0]["advice"]["timeout"], 60_000);
    }

    #[test]
    fn parses_advice() {
        let r = parse(
            &json!([{ "channel": "/meta/connect", "successful": true,
                      "advice": { "reconnect": "none", "interval": 2500, "timeout": 90000 } }]),
            "/meta/connect",
            DEV,
        );
        let m = r.meta.unwrap();
        assert_eq!(m.interval_ms, Some(2500));
        assert_eq!(m.timeout_ms, Some(90_000));
        assert!(m.reconnect_none());
        let f = MetaReply {
            error: Some("403::Forbidden".into()),
            ..Default::default()
        };
        assert!(f.is_forbidden());
    }

    fn op(id: &str, status: &str) -> Value {
        json!({ "id": id, "status": status })
    }

    #[test]
    fn dedup_drops_repeats_and_regressions() {
        let mut d = OpDedup::default();
        assert!(d.admit(&op("1", "PENDING")));
        // Same op again (PENDING list after reconnect, or CREATE + list).
        assert!(!d.admit(&op("1", "PENDING")));
        assert!(d.admit(&op("1", "EXECUTING")));
        // Stale PENDING / echo of the same status.
        assert!(!d.admit(&op("1", "PENDING")));
        assert!(!d.admit(&op("1", "EXECUTING")));
        assert!(d.admit(&op("1", "FAILED")));
        // FAILED wins over a later SUCCESSFUL, and nothing comes back from a terminal state.
        assert!(!d.admit(&op("1", "SUCCESSFUL")));
        assert!(!d.admit(&op("1", "PENDING")));
        // FAILED after SUCCESSFUL is still let through.
        assert!(d.admit(&op("2", "SUCCESSFUL")));
        assert!(d.admit(&op("2", "FAILED")));
        // Other ids are independent; no id / unknown status always pass.
        assert!(d.admit(&op("3", "PENDING")));
        assert!(d.admit(&json!({ "status": "PENDING" })));
        assert!(d.admit(&json!({ "status": "PENDING" })));
        assert!(d.admit(&op("4", "WEIRD")));
        assert!(d.admit(&op("4", "WEIRD")));
    }

    #[test]
    fn parses_handshake() {
        let r = parse(
            &json!([{ "channel": "/meta/handshake", "successful": true, "clientId": "x1",
                      "advice": { "reconnect": "retry" } }]),
            "/meta/handshake",
            DEV,
        );
        let m = r.meta.unwrap();
        assert!(m.successful);
        assert_eq!(m.client_id.as_deref(), Some("x1"));
        assert!(!m.needs_handshake());
    }

    #[test]
    fn parses_connect_with_operations() {
        let body = json!([
            { "channel": CH, "data": { "realtimeAction": "CREATE", "data": { "id": "1", "status": "PENDING" } } },
            { "channel": CH, "data": { "realtimeAction": "UPDATE", "data": { "id": "1", "status": "FAILED" } } },
            { "channel": CH, "data": { "realtimeAction": "DELETE", "data": "1" } },
            { "channel": "/operations/other", "data": { "realtimeAction": "CREATE", "data": { "id": "9" } } },
            { "channel": "/meta/connect", "successful": true }
        ]);
        let r = parse(&body, "/meta/connect", DEV);
        assert_eq!(r.operations.len(), 2);
        assert_eq!(r.operations[1]["status"], "FAILED");
        assert!(r.meta.unwrap().successful);
        assert!(!is_quiet_connect(&body));
        assert!(is_quiet_connect(
            &json!([{ "channel": "/meta/connect", "successful": true }])
        ));
    }

    #[test]
    fn parses_alarm_notifications() {
        let body = json!([
            { "channel": "/alarms/42", "data": { "realtimeAction": "UPDATE",
              "data": { "id": "5", "type": "lw_x", "status": "ACKNOWLEDGED" } } },
            { "channel": "/alarms/42", "data": { "realtimeAction": "DELETE", "data": "6" } },
            { "channel": "/alarms/43", "data": { "realtimeAction": "UPDATE", "data": { "id": "9" } } },
            { "channel": "/meta/connect", "successful": true }
        ]);
        let r = parse(&body, "/meta/connect", DEV);
        assert!(r.operations.is_empty());
        assert_eq!(r.alarms.len(), 2);
        assert_eq!(r.alarms[0]["status"], "ACKNOWLEDGED");
        assert_eq!(r.alarms[1], json!({ "id": "6", "status": "CLEARED" }));
        assert_eq!(alarms_channel("42"), "/alarms/42");
    }

    #[test]
    fn unknown_client_needs_handshake() {
        let r = parse(
            &json!([{ "channel": "/meta/connect", "successful": false,
                      "error": "402::Unknown client", "advice": { "reconnect": "handshake" } }]),
            "/meta/connect",
            DEV,
        );
        let m = r.meta.unwrap();
        assert!(!m.successful);
        assert!(m.needs_handshake());

        let only_error = MetaReply {
            error: Some("402::Unknown client".into()),
            ..Default::default()
        };
        assert!(only_error.needs_handshake());
        let other = MetaReply {
            error: Some("500::boom".into()),
            ..Default::default()
        };
        assert!(!other.needs_handshake());
        assert_eq!(other.error_text(), "500::boom");
    }

    #[test]
    fn tolerates_single_object_and_garbage() {
        let r = parse(
            &json!({ "channel": "/meta/subscribe", "successful": true }),
            "/meta/subscribe",
            DEV,
        );
        assert!(r.meta.unwrap().successful);
        assert_eq!(parse(&json!("x"), "/meta/connect", DEV), Reply::default());
    }
}
