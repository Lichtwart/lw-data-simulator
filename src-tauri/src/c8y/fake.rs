//! A small in-memory **fake Cumulocity** for tests (`#[cfg(test)]` only).
//!
//! Real HTTP on `127.0.0.1:<random port>` (axum), so the real [`C8yClient`] — headers, auth,
//! paging, multipart, realtime long-polling — is exercised end to end. Usage:
//!
//! ```ignore
//! let fake = FakeC8y::start().await;
//! let client = fake.client();                 // logged in as t100/demo, RecordingSink
//! let dev = fake.insert_managed_object(json!({"name": "d", "c8y_IsDevice": {}}));
//! client.create_alarm(&json!({ "type": "x", "text": "t", "severity": "MAJOR",
//!                              "source": { "id": dev } })).await?;
//! assert_eq!(fake.state().alarms.len(), 1);   // inspect state (std Mutex: don't hold across .await)
//! let op = fake.create_operation(&dev, json!({ "lw_fetchCore": {} })); // also pushed via realtime
//! fake.fail_next("PUT", "/devicecontrol/operations", 500);           // fault injection
//! fake.delay("PUT", "/devicecontrol/operations", Duration::from_secs(60)); // slow / hanging path
//! fake.set_clock_offset_secs(300);            // server clock 5 min ahead (`Date` header)
//! fake.drop_realtime_clients();               // next connect gets 402::Unknown client
//! ```
//!
//! Supported (subset of the real API): `GET /user/currentUser`; inventory managed objects
//! (CRUD, paging, `type` / `fragmentType` / `text` filters and a tiny `query=$filter=(…)`
//! subset: `has(x)`, `x eq 'v'` joined by `and`; `withParents=true` fills `assetParents`);
//! child additions and child assets (`POST …/childAssets`); binaries (multipart upload,
//! download, delete); measurements (single + collection; filters `source` / `type` /
//! `valueFragmentType` / `valueFragmentSeries`, `revert=true` = newest `time` first); alarms
//! (de-duplication of open alarms by source + type, filters `source` / `status` / `type`, PUT,
//! bulk PUT by `source` / `status`, realtime CREATE / UPDATE on `/alarms/<source>`,
//! `set_alarm_status` for changes "from outside");
//! events (`source` / `type`, `revert`); operations (filters `deviceId` / `status` /
//! `fragmentType`, `revert` by `creationTime`, PUT, realtime CREATE / UPDATE notifications); and
//! `/notification/realtime` (handshake, subscribe, long-poll connect, disconnect, 402 for
//! unknown clients). Measurements / alarms / events without `time` or `source.id` get 422,
//! like the real API. Every request is recorded ([`RecordedRequest`]) with its media headers
//! and the Basic-auth user name.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use serde_json::{Value, json};
use tokio::sync::Notify;

use super::activity::ActivityLog;
use super::client::{C8yClient, now_iso};
use super::sink::EventSink;
use crate::model::{ActivityEntry, Tenant};

pub const TENANT_ID: &str = "t100";
pub const USER: &str = "demo";
pub const PASSWORD: &str = "secret";

/// One request as the fake saw it.
#[derive(Debug, Clone)]
pub struct RecordedRequest {
    pub method: String,
    pub path: String,
    pub query: String,
    pub accept: Option<String>,
    pub content_type: Option<String>,
    pub use_xbasic: Option<String>,
    /// User name from the Basic `Authorization` header (password dropped).
    pub auth_user: Option<String>,
    pub body: Vec<u8>,
}

impl RecordedRequest {
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
}

#[derive(Debug, Default)]
struct RtClient {
    subscriptions: Vec<String>,
    queue: Vec<Value>,
}

/// The fake's data. Public fields so tests can seed and inspect directly.
#[derive(Debug)]
pub struct FakeState {
    /// Accepted Basic-auth user (`t100/demo`) and password.
    pub auth_user: String,
    pub password: String,
    next_id: u64,
    pub managed_objects: Vec<Value>,
    /// parent id → child ids.
    pub child_additions: HashMap<String, Vec<String>>,
    /// parent (group) id → child asset ids.
    pub child_assets: HashMap<String, Vec<String>>,
    /// id → (metadata MO, content).
    pub binaries: HashMap<String, (Value, Vec<u8>)>,
    pub measurements: Vec<Value>,
    pub alarms: Vec<Value>,
    pub events: Vec<Value>,
    pub operations: Vec<Value>,
    pub requests: Vec<RecordedRequest>,
    rt_clients: HashMap<String, RtClient>,
    /// (method, path prefix, status, skip) — the matching request after `skip` others fails with
    /// that status.
    failures: Vec<(String, String, u16, usize)>,
    /// (method, path prefix, delay) — every matching request is answered late.
    delays: Vec<(String, String, Duration)>,
    /// Seconds the server clock is ahead of the local one (`Date` header of every response).
    clock_offset_secs: i64,
    /// Max time a realtime connect is held open.
    pub connect_hold: Duration,
    /// Hold every connect for `connect_hold`, ignoring the `advice.timeout` the client asked for
    /// (a misbehaving server; exercises the client-side timeout path).
    pub ignore_advice_timeout: bool,
    /// Answer every `/meta/connect` with `402::Unknown client` (re-handshake storm).
    pub always_unknown_client: bool,
    /// `advice.timeout` (ms) in handshake replies. Small by default so it never raises the
    /// client's HTTP timeout in tests.
    pub advice_timeout_ms: u64,
    /// Answer a bulk alarm update (`PUT /alarm/alarms?…`) with `202 Accepted` and never apply it
    /// (exercises the client's check + one-by-one fallback).
    pub bulk_alarm_update_accepted: bool,
}

impl FakeState {
    fn new_id(&mut self) -> String {
        self.next_id += 1;
        self.next_id.to_string()
    }

    fn push_realtime(&mut self, channel: &str, data: Value) {
        for c in self.rt_clients.values_mut() {
            if c.subscriptions.iter().any(|s| s == channel) {
                c.queue
                    .push(json!({ "channel": channel, "data": data.clone() }));
            }
        }
    }

    /// Notify `/alarms/<source>` subscribers about an alarm (`CREATE` / `UPDATE`).
    fn push_alarm(&mut self, alarm: &Value, action: &str) {
        let channel = format!("/alarms/{}", alarm["source"]["id"].as_str().unwrap_or(""));
        self.push_realtime(
            &channel,
            json!({ "realtimeAction": action, "data": alarm.clone() }),
        );
    }

    fn find_mo(&mut self, id: &str) -> Option<&mut Value> {
        self.managed_objects.iter_mut().find(|m| m["id"] == id)
    }
}

struct Shared {
    state: Mutex<FakeState>,
    notify: Notify,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, FakeState> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }
}

/// Activity entries and `auth_expired` calls seen by a client.
#[derive(Debug, Default)]
pub struct RecordingSink {
    pub entries: Mutex<Vec<ActivityEntry>>,
    pub expired: Mutex<Vec<String>>,
}

impl EventSink for RecordingSink {
    fn activity_logged(&self, entry: &ActivityEntry) {
        self.entries.lock().unwrap().push(entry.clone());
    }
    fn auth_expired(&self, tenant_key: &str) {
        self.expired.lock().unwrap().push(tenant_key.to_owned());
    }
}

/// A running fake server. Dropping it stops the server.
pub struct FakeC8y {
    pub base_url: String,
    shared: Arc<Shared>,
    /// Sink of clients built with [`FakeC8y::client`].
    pub sink: Arc<RecordingSink>,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for FakeC8y {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl FakeC8y {
    /// Start on a random local port (call inside a tokio runtime, e.g. `#[tokio::test]`).
    pub async fn start() -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(FakeState {
                auth_user: format!("{TENANT_ID}/{USER}"),
                password: PASSWORD.to_owned(),
                next_id: 1000,
                managed_objects: Vec::new(),
                child_additions: HashMap::new(),
                child_assets: HashMap::new(),
                binaries: HashMap::new(),
                measurements: Vec::new(),
                alarms: Vec::new(),
                events: Vec::new(),
                operations: Vec::new(),
                requests: Vec::new(),
                rt_clients: HashMap::new(),
                failures: Vec::new(),
                delays: Vec::new(),
                clock_offset_secs: 0,
                connect_hold: Duration::from_millis(500),
                ignore_advice_timeout: false,
                always_unknown_client: false,
                advice_timeout_ms: 100,
                bulk_alarm_update_accepted: false,
            }),
            notify: Notify::new(),
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fake c8y");
        let addr = listener.local_addr().expect("local addr");
        let router = Router::new().fallback(handle).with_state(shared.clone());
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        Self {
            base_url: format!("http://{addr}"),
            shared,
            sink: Arc::new(RecordingSink::default()),
            server,
        }
    }

    /// A tenant pointing at this server (foreign domain → user `t100/demo`).
    pub fn tenant(&self) -> Tenant {
        Tenant {
            key: "fake".into(),
            tenant_id: TENANT_ID.into(),
            base_url: self.base_url.clone(),
        }
    }

    /// A client with valid credentials and [`Self::sink`].
    pub fn client(&self) -> C8yClient {
        self.client_with(USER, PASSWORD)
    }

    /// Like [`Self::client`] with a custom default request timeout.
    pub fn client_with_timeout(&self, timeout: Duration) -> C8yClient {
        self.client()
            .with_request_timeout(timeout)
            .expect("client timeout")
    }

    /// A client with the given credentials (plain user name) and [`Self::sink`].
    pub fn client_with(&self, user: &str, password: &str) -> C8yClient {
        C8yClient::new(
            self.tenant(),
            user.into(),
            password.into(),
            Arc::new(ActivityLog::new()),
            self.sink.clone(),
        )
        .expect("client")
    }

    /// Lock the state (std Mutex — never hold the guard across `.await`).
    pub fn state(&self) -> MutexGuard<'_, FakeState> {
        self.shared.lock()
    }

    /// All recorded requests.
    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.state().requests.clone()
    }

    /// Store a managed object (an `id` is assigned unless present); returns the id.
    pub fn insert_managed_object(&self, mut mo: Value) -> String {
        let mut s = self.state();
        let id = match mo.get("id").and_then(Value::as_str) {
            Some(id) => id.to_owned(),
            None => s.new_id(),
        };
        mo["id"] = json!(id);
        s.managed_objects.push(mo);
        id
    }

    /// Create a PENDING operation for `device_id` (`fragment` = e.g. `{"lw_fetchCore":{}}`)
    /// and notify realtime subscribers (CREATE). Returns the operation. Panics when the real
    /// API would reject it (see [`Self::try_create_operation`]).
    pub fn create_operation(&self, device_id: &str, fragment: Value) -> Value {
        self.try_create_operation(device_id, fragment)
            .unwrap_or_else(|e| panic!("operation rejected: {e}"))
    }

    /// Like [`Self::create_operation`], but returns the error body Cumulocity answers when the
    /// device is stored and is not an agent (no `com_cumulocity_model_Agent`): the real API
    /// addresses every operation to an agent and rejects it with 422. Ids the fake does not
    /// store (bare ids like `"7"` in client tests) are accepted.
    pub fn try_create_operation(&self, device_id: &str, fragment: Value) -> Result<Value, Value> {
        let op = {
            let mut s = self.state();
            if let Some(mo) = s.managed_objects.iter().find(|m| m["id"] == device_id)
                && mo.get("com_cumulocity_model_Agent").is_none()
            {
                return Err(json!({
                    "error": "devicecontrol/Unprocessable Entity",
                    "message": format!("Agent for device '{device_id}' does not exist in inventory!"),
                }));
            }
            let mut op = fragment;
            op["id"] = json!(s.new_id());
            op["deviceId"] = json!(device_id);
            op["status"] = json!("PENDING");
            op["creationTime"] = json!(now_iso());
            s.operations.push(op.clone());
            s.push_realtime(
                &format!("/operations/{device_id}"),
                json!({ "realtimeAction": "CREATE", "data": op.clone() }),
            );
            op
        };
        self.shared.notify.notify_waiters();
        Ok(op)
    }

    /// The next request with this method and path prefix fails with `status`.
    pub fn fail_next(&self, method: &str, path_prefix: &str, status: u16) {
        self.fail_after(method, path_prefix, 0, status);
    }

    /// The matching request after `skip` successful ones fails with `status` (e.g. the second
    /// binary upload of an apply).
    pub fn fail_after(&self, method: &str, path_prefix: &str, skip: usize, status: u16) {
        self.state()
            .failures
            .push((method.to_owned(), path_prefix.to_owned(), status, skip));
    }

    /// Every request with this method and path prefix is answered `by` late (a slow tenant; a
    /// huge delay hangs it until the client times out).
    pub fn delay(&self, method: &str, path_prefix: &str, by: Duration) {
        self.state()
            .delays
            .push((method.to_owned(), path_prefix.to_owned(), by));
    }

    /// The server clock runs `secs` ahead of the local one (seen in the `Date` header).
    pub fn set_clock_offset_secs(&self, secs: i64) {
        self.state().clock_offset_secs = secs;
    }

    /// Forget all realtime clients (like a server restart): the next connect gets
    /// `402::Unknown client` with `advice.reconnect = handshake`.
    pub fn drop_realtime_clients(&self) {
        self.state().rt_clients.clear();
        self.shared.notify.notify_waiters();
    }

    /// Change an alarm's status "from outside" (the presenter in the Cumulocity UI): realtime
    /// subscribers get the `UPDATE` when `notify`, otherwise the change is silent (as if it
    /// happened while the realtime connection was down). Panics on an unknown id.
    pub fn set_alarm_status(&self, id: &str, status: &str, notify: bool) {
        {
            let mut s = self.state();
            let a = s
                .alarms
                .iter_mut()
                .find(|a| a["id"] == id)
                .unwrap_or_else(|| panic!("no alarm {id}"));
            a["status"] = json!(status);
            let a = a.clone();
            if notify {
                s.push_alarm(&a, "UPDATE");
            }
        }
        self.shared.notify.notify_waiters();
    }

    /// Delete an alarm "from outside": realtime subscribers get a `DELETE` whose `data` is the
    /// id (as Cumulocity sends it). Panics on an unknown id.
    pub fn delete_alarm(&self, id: &str) {
        {
            let mut s = self.state();
            let pos = s
                .alarms
                .iter()
                .position(|a| a["id"] == id)
                .unwrap_or_else(|| panic!("no alarm {id}"));
            let a = s.alarms.remove(pos);
            let channel = format!("/alarms/{}", a["source"]["id"].as_str().unwrap_or(""));
            s.push_realtime(&channel, json!({ "realtimeAction": "DELETE", "data": id }));
        }
        self.shared.notify.notify_waiters();
    }

    /// Id of the newest alarm of `alarm_type` (any status), if any.
    pub fn alarm_id(&self, alarm_type: &str) -> Option<String> {
        self.state()
            .alarms
            .iter()
            .rev()
            .find(|a| a["type"] == alarm_type)
            .and_then(|a| a["id"].as_str().map(str::to_owned))
    }

    /// Number of live realtime clients.
    pub fn realtime_clients(&self) -> usize {
        self.state().rt_clients.len()
    }
}

// ───────────────────────────── HTTP handling ─────────────────────────────

fn decode_component(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(b) => {
                        out.push(b);
                        i += 2;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn parse_query(q: &str) -> HashMap<String, String> {
    q.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (decode_component(k), decode_component(v))
        })
        .collect()
}

fn json_resp(status: u16, body: Value) -> Response {
    (
        StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

fn error(status: u16, err: &str, msg: &str) -> Response {
    json_resp(status, json!({ "error": err, "message": msg }))
}

fn header_str(h: &HeaderMap, name: impl header::AsHeaderName) -> Option<String> {
    h.get(name).and_then(|v| v.to_str().ok()).map(str::to_owned)
}

fn basic_credentials(h: &HeaderMap) -> Option<(String, String)> {
    let v = header_str(h, header::AUTHORIZATION)?;
    let b64 = v.strip_prefix("Basic ")?;
    let raw = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    let s = String::from_utf8(raw).ok()?;
    let (u, p) = s.split_once(':')?;
    Some((u.to_owned(), p.to_owned()))
}

/// Paged collection response.
fn collection(key: &str, items: Vec<Value>, q: &HashMap<String, String>) -> Response {
    let page_size: usize = q
        .get("pageSize")
        .and_then(|v| v.parse().ok())
        .unwrap_or(5)
        .max(1);
    let page: usize = q
        .get("currentPage")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1)
        .max(1);
    let total_pages = items.len().div_ceil(page_size).max(1);
    let page_items: Vec<Value> = items
        .into_iter()
        .skip((page - 1) * page_size)
        .take(page_size)
        .collect();
    let mut stats = json!({ "pageSize": page_size, "currentPage": page });
    if q.get("withTotalPages").is_some_and(|v| v == "true") {
        stats["totalPages"] = json!(total_pages);
    }
    json_resp(200, json!({ key: page_items, "statistics": stats }))
}

fn csv(q: &HashMap<String, String>, k: &str) -> Option<Vec<String>> {
    q.get(k)
        .map(|v| v.split(',').map(|s| s.trim().to_owned()).collect())
}

/// Tiny subset of the inventory query language: `$filter=(a and b) $orderby=…` with terms
/// `has(frag)` and `path eq 'value'` (arrays match if they contain the value; `*` wildcard
/// at the end of the value).
fn matches_query(mo: &Value, query: &str) -> bool {
    let filter = query
        .split("$orderby")
        .next()
        .unwrap_or("")
        .trim()
        .trim_start_matches("$filter=")
        .trim();
    let filter = filter.trim_start_matches('(').trim_end_matches(')');
    if filter.is_empty() {
        return true;
    }
    filter.split(" and ").all(|term| {
        let term = term
            .trim()
            .trim_start_matches('(')
            .trim_end_matches(')')
            .trim();
        assert!(
            !term.contains(" or "),
            "fake c8y: `or` is not supported in {query:?}"
        );
        if let Some(frag) = term.strip_prefix("has(") {
            return mo.get(frag.trim_end_matches(')').trim()).is_some();
        }
        if let Some((field, value)) = term.split_once(" eq ") {
            let value = value.trim().trim_matches('\'');
            let ptr = format!("/{}", field.trim().replace('.', "/"));
            let eq = |v: &Value| match v.as_str() {
                Some(s) => match value.strip_suffix('*') {
                    Some(prefix) => s.starts_with(prefix),
                    None => s == value,
                },
                None => serde_json::from_str::<Value>(value).ok().as_ref() == Some(v),
            };
            return match mo.pointer(&ptr) {
                Some(Value::Array(a)) => a.iter().any(eq),
                Some(v) => eq(v),
                None => false,
            };
        }
        // The real server understands more; the fake must not silently match what it cannot
        // evaluate (a wrong query in code under test would pass).
        panic!("fake c8y: unsupported inventory query term {term:?} in {query:?}")
    })
}

/// `assetParents.references` of managed object `id` (`withParents=true`).
fn asset_parents(s: &FakeState, id: &str) -> Value {
    let refs: Vec<Value> = s
        .child_assets
        .iter()
        .filter(|(_, kids)| kids.iter().any(|k| k == id))
        .filter_map(|(parent, _)| s.managed_objects.iter().find(|m| m["id"] == *parent))
        .map(|p| json!({ "managedObject": { "id": p["id"], "name": p["name"] } }))
        .collect();
    json!({ "references": refs })
}

/// `revert=true`: newest first by `key` (ISO strings sort by time); else insertion order.
fn ordered(mut items: Vec<Value>, q: &HashMap<String, String>, key: &str) -> Vec<Value> {
    if q.get("revert").is_some_and(|v| v == "true") {
        items.sort_by(|a, b| {
            b[key]
                .as_str()
                .unwrap_or("")
                .cmp(a[key].as_str().unwrap_or(""))
        });
    }
    items
}

/// Shallow merge (PUT on managed objects / alarms / operations).
fn merge(target: &mut Value, patch: &Value) {
    if let (Some(t), Some(p)) = (target.as_object_mut(), patch.as_object()) {
        for (k, v) in p {
            if k != "id" {
                t.insert(k.clone(), v.clone());
            }
        }
    }
}

/// Minimal multipart/form-data parser → name → bytes.
fn parse_multipart(content_type: &str, body: &[u8]) -> HashMap<String, Vec<u8>> {
    let mut parts = HashMap::new();
    let Some(boundary) = content_type
        .split(';')
        .find_map(|p| p.trim().strip_prefix("boundary="))
    else {
        return parts;
    };
    let delim = format!("--{}", boundary.trim_matches('"'));
    let delim = delim.as_bytes();
    let find = |hay: &[u8], needle: &[u8], from: usize| {
        hay[from..]
            .windows(needle.len())
            .position(|w| w == needle)
            .map(|p| p + from)
    };
    let mut pos = match find(body, delim, 0) {
        Some(p) => p + delim.len(),
        None => return parts,
    };
    while let Some(next) = find(body, delim, pos) {
        let part = &body[pos..next];
        let part = part.strip_prefix(b"\r\n").unwrap_or(part);
        let part = part.strip_suffix(b"\r\n").unwrap_or(part);
        if let Some(split) = find(part, b"\r\n\r\n", 0) {
            let headers = String::from_utf8_lossy(&part[..split]);
            let content = part[split + 4..].to_vec();
            if let Some(name) = headers
                .split("name=\"")
                .nth(1)
                .and_then(|r| r.split('"').next())
            {
                parts.insert(name.to_owned(), content);
            }
        }
        pos = next + delim.len();
    }
    parts
}

async fn handle(
    State(shared): State<Arc<Shared>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let path = uri.path().to_owned();
    let query_str = uri.query().unwrap_or("").to_owned();
    let creds = basic_credentials(&headers);
    {
        let mut s = shared.lock();
        s.requests.push(RecordedRequest {
            method: method.to_string(),
            path: path.clone(),
            query: query_str.clone(),
            accept: header_str(&headers, header::ACCEPT),
            content_type: header_str(&headers, header::CONTENT_TYPE),
            use_xbasic: header_str(&headers, "usexbasic"),
            auth_user: creds.as_ref().map(|c| c.0.clone()),
            body: body.to_vec(),
        });
        let authorized = creds
            .as_ref()
            .is_some_and(|(u, p)| *u == s.auth_user && *p == s.password);
        if !authorized {
            return error(401, "security/Unauthorized", "Invalid credentials!");
        }
        if let Some(i) = s
            .failures
            .iter()
            .position(|(m, p, _, _)| m == method.as_str() && path.starts_with(p.as_str()))
        {
            if s.failures[i].3 > 0 {
                s.failures[i].3 -= 1;
            } else {
                let (_, _, status, _) = s.failures.remove(i);
                return error(status, "fake/injected", "injected failure");
            }
        }
    }
    let delay = {
        let s = shared.lock();
        s.delays
            .iter()
            .filter(|(m, p, _)| m == method.as_str() && path.starts_with(p.as_str()))
            .map(|(_, _, d)| *d)
            .max()
    };
    if let Some(d) = delay {
        tokio::time::sleep(d).await;
    }

    let q = parse_query(&query_str);
    let json_body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let segs: Vec<&str> = path.trim_matches('/').split('/').collect();

    if segs.as_slice() == ["notification", "realtime"] && method == Method::POST {
        return realtime(&shared, json_body).await;
    }

    let (mut resp, offset) = {
        let mut s = shared.lock();
        let offset = s.clock_offset_secs;
        (
            route(&mut s, &method, &segs, &q, &headers, &body, json_body),
            offset,
        )
    };
    shared.notify.notify_waiters();
    let date = (chrono::Utc::now() + chrono::Duration::seconds(offset))
        .format("%a, %d %b %Y %H:%M:%S GMT")
        .to_string();
    if let Ok(v) = header::HeaderValue::from_str(&date) {
        resp.headers_mut().insert(header::DATE, v);
    }
    resp
}

fn require_time_and_source(body: &Value) -> Option<Response> {
    if body.get("time").and_then(Value::as_str).is_none() {
        return Some(error(422, "fake/Unprocessable", "time missing"));
    }
    if body.pointer("/source/id").is_none() {
        return Some(error(422, "fake/Unprocessable", "source.id missing"));
    }
    None
}

#[allow(clippy::too_many_lines)]
fn route(
    s: &mut FakeState,
    method: &Method,
    segs: &[&str],
    q: &HashMap<String, String>,
    headers: &HeaderMap,
    raw: &[u8],
    body: Value,
) -> Response {
    let m = method.as_str();
    match (m, segs) {
        ("GET", ["user", "currentUser"]) => json_resp(
            200,
            json!({ "id": USER, "userName": USER, "firstName": "Demo", "lastName": "User",
                    "email": "demo@example.com" }),
        ),

        // ── inventory ──
        ("GET", ["inventory", "managedObjects"]) => {
            let items: Vec<Value> = s
                .managed_objects
                .iter()
                .filter(|mo| q.get("type").is_none_or(|t| mo["type"] == **t))
                .filter(|mo| {
                    q.get("fragmentType")
                        .is_none_or(|f| mo.get(f.as_str()).is_some())
                })
                .filter(|mo| {
                    q.get("text").is_none_or(|t| {
                        mo["name"]
                            .as_str()
                            .is_some_and(|n| n.to_lowercase().contains(&t.to_lowercase()))
                    })
                })
                .filter(|mo| q.get("query").is_none_or(|qq| matches_query(mo, qq)))
                .cloned()
                .map(|mut mo| {
                    if q.get("withParents").is_some_and(|v| v == "true") {
                        mo["assetParents"] = asset_parents(s, mo["id"].as_str().unwrap_or(""));
                    }
                    mo
                })
                .collect();
            collection("managedObjects", items, q)
        }
        ("POST", ["inventory", "managedObjects"]) => {
            let mut mo = body;
            mo["id"] = json!(s.new_id());
            mo["creationTime"] = json!(now_iso());
            mo["lastUpdated"] = json!(now_iso());
            s.managed_objects.push(mo.clone());
            json_resp(201, mo)
        }
        ("GET", ["inventory", "managedObjects", id]) => match s.find_mo(id).cloned() {
            Some(mut mo) => {
                if q.get("withParents").is_some_and(|v| v == "true") {
                    mo["assetParents"] = asset_parents(s, id);
                }
                json_resp(200, mo)
            }
            None => error(404, "inventory/Not Found", "Finding failed"),
        },
        ("PUT", ["inventory", "managedObjects", id]) => match s.find_mo(id) {
            Some(mo) => {
                merge(mo, &body);
                mo["lastUpdated"] = json!(now_iso());
                json_resp(200, mo.clone())
            }
            None => error(404, "inventory/Not Found", "Finding failed"),
        },
        ("DELETE", ["inventory", "managedObjects", id]) => {
            let before = s.managed_objects.len();
            s.managed_objects.retain(|mo| mo["id"] != *id);
            if s.managed_objects.len() == before {
                return error(404, "inventory/Not Found", "Finding failed");
            }
            s.child_additions.remove(*id);
            s.child_assets.remove(*id);
            for children in s
                .child_additions
                .values_mut()
                .chain(s.child_assets.values_mut())
            {
                children.retain(|c| c != id);
            }
            StatusCode::NO_CONTENT.into_response()
        }
        ("POST", ["inventory", "managedObjects", parent, "childAdditions"]) => {
            let Some(child) = body.pointer("/managedObject/id").and_then(Value::as_str) else {
                return error(422, "fake/Unprocessable", "managedObject.id missing");
            };
            let child = child.to_owned();
            if s.find_mo(parent).is_none() || s.find_mo(&child).is_none() {
                return error(404, "inventory/Not Found", "Finding failed");
            }
            s.child_additions
                .entry((*parent).to_owned())
                .or_default()
                .push(child.clone());
            json_resp(201, json!({ "managedObject": { "id": child } }))
        }
        ("POST", ["inventory", "managedObjects", parent, "childAssets"]) => {
            let Some(child) = body.pointer("/managedObject/id").and_then(Value::as_str) else {
                return error(422, "fake/Unprocessable", "managedObject.id missing");
            };
            let child = child.to_owned();
            if s.find_mo(parent).is_none() || s.find_mo(&child).is_none() {
                return error(404, "inventory/Not Found", "Finding failed");
            }
            s.child_assets
                .entry((*parent).to_owned())
                .or_default()
                .push(child.clone());
            json_resp(201, json!({ "managedObject": { "id": child } }))
        }
        ("GET", ["inventory", "managedObjects", parent, "childAdditions"]) => {
            let ids = s.child_additions.get(*parent).cloned().unwrap_or_default();
            let refs: Vec<Value> = ids
                .iter()
                .filter_map(|id| s.managed_objects.iter().find(|m| m["id"] == *id))
                .map(|mo| json!({ "managedObject": mo }))
                .collect();
            collection("references", refs, q)
        }
        (
            "DELETE",
            [
                "inventory",
                "managedObjects",
                parent,
                "childAdditions",
                child,
            ],
        ) => {
            if let Some(children) = s.child_additions.get_mut(*parent) {
                children.retain(|c| c != child);
            }
            StatusCode::NO_CONTENT.into_response()
        }

        // ── binaries ──
        ("POST", ["inventory", "binaries"]) => {
            let ct = header_str(headers, header::CONTENT_TYPE).unwrap_or_default();
            let parts = parse_multipart(&ct, raw);
            let (Some(object), Some(file)) = (parts.get("object"), parts.get("file")) else {
                return error(422, "fake/Unprocessable", "object/file part missing");
            };
            let object: Value = serde_json::from_slice(object).unwrap_or(Value::Null);
            let id = s.new_id();
            let meta = json!({
                "id": id, "name": object["name"], "type": object["type"],
                "contentType": object["type"], "length": file.len(), "c8y_IsBinary": {}
            });
            s.binaries.insert(id, (meta.clone(), file.clone()));
            json_resp(201, meta)
        }
        ("GET", ["inventory", "binaries", id]) => match s.binaries.get(*id) {
            Some((meta, bytes)) => (
                StatusCode::OK,
                [(
                    header::CONTENT_TYPE,
                    meta["type"]
                        .as_str()
                        .unwrap_or("application/octet-stream")
                        .to_owned(),
                )],
                bytes.clone(),
            )
                .into_response(),
            None => error(404, "inventory/Not Found", "Finding failed"),
        },
        ("DELETE", ["inventory", "binaries", id]) => match s.binaries.remove(*id) {
            Some(_) => StatusCode::NO_CONTENT.into_response(),
            None => error(404, "inventory/Not Found", "Finding failed"),
        },

        // ── measurements ──
        ("POST", ["measurement", "measurements"]) => {
            let (items, collection_body) = match body.get("measurements") {
                Some(Value::Array(a)) => (a.clone(), true),
                _ => (vec![body.clone()], false),
            };
            for m in &items {
                if let Some(r) = require_time_and_source(m) {
                    return r;
                }
            }
            let mut created = Vec::new();
            for mut m in items {
                m["id"] = json!(s.new_id());
                s.measurements.push(m.clone());
                created.push(m);
            }
            if collection_body {
                json_resp(201, json!({ "measurements": created }))
            } else {
                json_resp(201, created.remove(0))
            }
        }
        ("GET", ["measurement", "measurements"]) => {
            let items: Vec<Value> = s
                .measurements
                .iter()
                .filter(|m| q.get("source").is_none_or(|src| m["source"]["id"] == **src))
                .filter(|m| q.get("type").is_none_or(|t| m["type"] == **t))
                .filter(|m| {
                    q.get("valueFragmentType")
                        .is_none_or(|f| m.get(f.as_str()).is_some())
                })
                .filter(
                    |m| match (q.get("valueFragmentType"), q.get("valueFragmentSeries")) {
                        (Some(f), Some(series)) => m[f.as_str()].get(series.as_str()).is_some(),
                        _ => true,
                    },
                )
                .cloned()
                .collect();
            collection("measurements", ordered(items, q, "time"), q)
        }

        // ── alarms ──
        ("POST", ["alarm", "alarms"]) => {
            if let Some(r) = require_time_and_source(&body) {
                return r;
            }
            let src = body["source"]["id"].clone();
            let typ = body["type"].clone();
            if let Some(existing) = s.alarms.iter_mut().find(|a| {
                a["source"]["id"] == src
                    && a["type"] == typ
                    && matches!(a["status"].as_str(), Some("ACTIVE" | "ACKNOWLEDGED"))
            }) {
                let count = existing["count"].as_u64().unwrap_or(1) + 1;
                existing["count"] = json!(count);
                let a = existing.clone();
                s.push_alarm(&a, "UPDATE");
                return json_resp(201, a);
            }
            let mut a = body;
            a["id"] = json!(s.new_id());
            if a.get("status").is_none() {
                a["status"] = json!("ACTIVE");
            }
            a["count"] = json!(1);
            s.alarms.push(a.clone());
            s.push_alarm(&a, "CREATE");
            json_resp(201, a)
        }
        ("GET", ["alarm", "alarms"]) => {
            let statuses = csv(q, "status");
            let types = csv(q, "type");
            let items: Vec<Value> = s
                .alarms
                .iter()
                .filter(|a| q.get("source").is_none_or(|src| a["source"]["id"] == **src))
                .filter(|a| {
                    statuses
                        .as_ref()
                        .is_none_or(|st| st.iter().any(|x| a["status"] == **x))
                })
                .filter(|a| {
                    types
                        .as_ref()
                        .is_none_or(|ts| ts.iter().any(|x| a["type"] == **x))
                })
                .cloned()
                .collect();
            collection("alarms", items, q)
        }
        // Bulk update (`source` + `status` filters; only `status` changes, like the real API).
        ("PUT", ["alarm", "alarms"]) => {
            if q.get("source").is_none() && q.get("status").is_none() {
                return error(422, "fake/Unprocessable", "bulk update needs a filter");
            }
            if s.bulk_alarm_update_accepted {
                // "Being updated in the background" — and the fake never finishes it.
                return json_resp(202, Value::Null);
            }
            let statuses = csv(q, "status");
            let mut changed = Vec::new();
            for a in &mut s.alarms {
                let hit = q.get("source").is_none_or(|src| a["source"]["id"] == **src)
                    && statuses
                        .as_ref()
                        .is_none_or(|st| st.iter().any(|x| a["status"] == **x));
                if hit && let Some(status) = body.get("status") {
                    a["status"] = status.clone();
                    changed.push(a.clone());
                }
            }
            for a in &changed {
                s.push_alarm(a, "UPDATE");
            }
            json_resp(200, Value::Null)
        }
        ("PUT", ["alarm", "alarms", id]) => match s.alarms.iter_mut().find(|a| a["id"] == *id) {
            Some(a) => {
                merge(a, &body);
                let a = a.clone();
                s.push_alarm(&a, "UPDATE");
                json_resp(200, a)
            }
            None => error(404, "alarm/Not Found", "Finding failed"),
        },

        // ── events ──
        ("POST", ["event", "events"]) => {
            if let Some(r) = require_time_and_source(&body) {
                return r;
            }
            let mut e = body;
            e["id"] = json!(s.new_id());
            s.events.push(e.clone());
            json_resp(201, e)
        }
        ("GET", ["event", "events"]) => {
            let items: Vec<Value> = s
                .events
                .iter()
                .filter(|e| q.get("source").is_none_or(|src| e["source"]["id"] == **src))
                .filter(|e| q.get("type").is_none_or(|t| e["type"] == **t))
                .filter(|e| {
                    // `dateFrom`: events at or after it (RFC 3339 instants).
                    let at = |v: &str| chrono::DateTime::parse_from_rfc3339(v).ok();
                    match (
                        q.get("dateFrom").and_then(|d| at(d)),
                        e["time"].as_str().and_then(at),
                    ) {
                        (Some(from), Some(t)) => t >= from,
                        _ => true,
                    }
                })
                .cloned()
                .collect();
            collection("events", ordered(items, q, "time"), q)
        }

        // ── operations ──
        ("GET", ["devicecontrol", "operations"]) => {
            let items: Vec<Value> = s
                .operations
                .iter()
                .filter(|o| q.get("deviceId").is_none_or(|d| o["deviceId"] == **d))
                .filter(|o| q.get("status").is_none_or(|st| o["status"] == **st))
                .filter(|o| {
                    q.get("fragmentType")
                        .is_none_or(|f| o.get(f.as_str()).is_some())
                })
                .cloned()
                .collect();
            collection("operations", ordered(items, q, "creationTime"), q)
        }
        ("GET", ["devicecontrol", "operations", id]) => {
            match s.operations.iter().find(|o| o["id"] == *id) {
                Some(o) => json_resp(200, o.clone()),
                None => error(404, "devicecontrol/Not Found", "Finding failed"),
            }
        }
        ("PUT", ["devicecontrol", "operations", id]) => {
            let Some(op) = s.operations.iter_mut().find(|o| o["id"] == *id) else {
                return error(404, "devicecontrol/Not Found", "Finding failed");
            };
            merge(op, &body);
            let op = op.clone();
            let channel = format!("/operations/{}", op["deviceId"].as_str().unwrap_or(""));
            s.push_realtime(
                &channel,
                json!({ "realtimeAction": "UPDATE", "data": op.clone() }),
            );
            json_resp(200, op)
        }

        _ => error(
            404,
            "fake/Not Found",
            &format!("{m} /{} not supported", segs.join("/")),
        ),
    }
}

async fn realtime(shared: &Arc<Shared>, body: Value) -> Response {
    let msgs: Vec<Value> = match body {
        Value::Array(a) => a,
        v @ Value::Object(_) => vec![v],
        _ => return error(400, "fake/BadRequest", "bayeux body"),
    };
    let mut replies = Vec::new();
    for msg in msgs {
        let channel = msg["channel"].as_str().unwrap_or("").to_owned();
        let client_id = msg["clientId"].as_str().unwrap_or("").to_owned();
        match channel.as_str() {
            "/meta/handshake" => {
                let mut s = shared.lock();
                let id = format!("c{}", s.new_id());
                let advice_timeout = s.advice_timeout_ms;
                s.rt_clients.insert(id.clone(), RtClient::default());
                replies.push(json!({
                    "channel": channel, "successful": true, "clientId": id, "version": "1.0",
                    "supportedConnectionTypes": ["long-polling"],
                    "advice": { "reconnect": "retry", "interval": 0, "timeout": advice_timeout }
                }));
            }
            "/meta/subscribe" => {
                let mut s = shared.lock();
                let sub = msg["subscription"].as_str().unwrap_or("").to_owned();
                match s.rt_clients.get_mut(&client_id) {
                    Some(c) => {
                        c.subscriptions.push(sub.clone());
                        replies.push(json!({ "channel": channel, "successful": true,
                                             "clientId": client_id, "subscription": sub }));
                    }
                    None => replies.push(unknown_client(&channel)),
                }
            }
            "/meta/disconnect" => {
                shared.lock().rt_clients.remove(&client_id);
                replies.push(json!({ "channel": channel, "successful": true }));
            }
            "/meta/connect" => {
                let requested = msg
                    .pointer("/advice/timeout")
                    .and_then(Value::as_u64)
                    .map(Duration::from_millis);
                let (hold, always_402) = {
                    let s = shared.lock();
                    let hold = match requested {
                        Some(r) if !s.ignore_advice_timeout => r.min(s.connect_hold),
                        _ => s.connect_hold,
                    };
                    (hold, s.always_unknown_client)
                };
                if always_402 {
                    replies.push(unknown_client(&channel));
                    continue;
                }
                let deadline = tokio::time::Instant::now() + hold;
                loop {
                    let notified = shared.notify.notified();
                    tokio::pin!(notified);
                    notified.as_mut().enable();
                    {
                        let mut s = shared.lock();
                        match s.rt_clients.get_mut(&client_id) {
                            None => {
                                replies.push(unknown_client(&channel));
                                break;
                            }
                            Some(c) if !c.queue.is_empty() => {
                                replies.append(&mut c.queue);
                                replies.push(json!({ "channel": channel, "successful": true }));
                                break;
                            }
                            Some(_) => {}
                        }
                    }
                    if tokio::time::timeout_at(deadline, notified).await.is_err() {
                        replies.push(json!({ "channel": channel, "successful": true }));
                        break;
                    }
                }
            }
            _ => replies.push(json!({ "channel": channel, "successful": false,
                                      "error": "400::unknown channel" })),
        }
    }
    json_resp(200, Value::Array(replies))
}

fn unknown_client(channel: &str) -> Value {
    json!({
        "channel": channel, "successful": false, "error": "402::Unknown client",
        "advice": { "reconnect": "handshake", "interval": 0 }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_subset() {
        let mo = json!({ "name": "Tankstelle", "c8y_IsDevice": {}, "c8y_DeviceTypes": ["demo"],
                         "c8y_Hardware": { "model": "Lichtwart Core v2" } });
        assert!(matches_query(
            &mo,
            "$filter=(has(c8y_IsDevice) and c8y_DeviceTypes eq 'demo')"
        ));
        assert!(matches_query(
            &mo,
            "$filter=(name eq 'Tank*') $orderby=name"
        ));
        assert!(matches_query(
            &mo,
            "c8y_Hardware.model eq 'Lichtwart Core v2'"
        ));
        assert!(!matches_query(&mo, "$filter=(c8y_DeviceTypes eq 'real')"));
        assert!(!matches_query(&mo, "has(lw_demo)"));
    }

    #[test]
    #[should_panic(expected = "unsupported inventory query term")]
    fn query_rejects_unknown_terms() {
        matches_query(&json!({ "name": "x" }), "$filter=(bygroupid(5))");
    }

    #[test]
    #[should_panic(expected = "`or` is not supported")]
    fn query_rejects_or() {
        matches_query(
            &json!({ "name": "x" }),
            "$filter=(name eq 'x' or name eq 'y')",
        );
    }

    #[test]
    fn decodes_query() {
        let q = parse_query("a=1&query=%24filter%3D%28x%20eq%20%27y%27%29&b=c+d");
        assert_eq!(q["query"], "$filter=(x eq 'y')");
        assert_eq!(q["b"], "c d");
    }
}
