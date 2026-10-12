//! Authenticated Cumulocity REST client: transport, headers, error mapping, activity logging.
//!
//! The typed helpers per API (inventory, measurements, alarms, events, operations, binaries)
//! live in [`super::api`] as further `impl C8yClient` blocks.

use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::Method;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

use super::activity::ActivityLog;
use super::media;
use super::sink::EventSink;
use crate::error::{AppError, AppResult};
use crate::model::{ActivityEntry, AuthState, Tenant};

/// Default timeout of a REST request.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// TCP/TLS connect timeout.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Request / response bodies are cut to this many bytes in the activity log.
pub const LOG_BODY_LIMIT: usize = 2048;

/// Basic-auth user name as sent (PRD §5): always `tenantId/user` (valid on the tenant's own
/// domain and on every other Cumulocity domain alike, so no domain guessing is needed). A name
/// that already contains `/`, or an empty tenant id, is sent unchanged.
pub fn basic_auth_user(tenant_id: &str, username: &str) -> String {
    if username.contains('/') || tenant_id.is_empty() {
        username.to_owned()
    } else {
        format!("{tenant_id}/{username}")
    }
}

/// Cut `s` to at most `limit` bytes on a char boundary, marking the cut with `…`.
pub fn truncate_body(s: &str, limit: usize) -> String {
    if s.len() <= limit {
        return s.to_owned();
    }
    let mut end = limit;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

/// `now` on the server's clock: the local clock corrected by `skew_ms` (see
/// [`C8yClient::clock_skew`]).
pub fn server_now(skew: &std::sync::atomic::AtomicI64) -> chrono::DateTime<chrono::Utc> {
    chrono::Utc::now()
        + chrono::Duration::milliseconds(skew.load(std::sync::atomic::Ordering::Relaxed))
}

/// ISO-8601 UTC time with milliseconds, e.g. `2026-10-12T12:00:00.000Z` (Cumulocity `time`).
pub fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Fill a missing `time` (now) in measurement / alarm / event bodies POSTed to their
/// collection, including every element of a `measurementCollection` (DECISIONS D5).
pub fn ensure_time(method: &Method, path: &str, body: &mut Value) {
    if *method != Method::POST {
        return;
    }
    let path = path.split('?').next().unwrap_or(path);
    if !matches!(
        path,
        "/measurement/measurements" | "/alarm/alarms" | "/event/events"
    ) {
        return;
    }
    let now = now_iso();
    let fill = |v: &mut Value| {
        if let Some(obj) = v.as_object_mut() {
            obj.entry("time")
                .or_insert_with(|| Value::String(now.clone()));
        }
    };
    match body.get_mut("measurements").and_then(Value::as_array_mut) {
        Some(items) => items.iter_mut().for_each(fill),
        None => fill(body),
    }
}

/// Message of a Cumulocity error body (`{ "error": "…", "message": "…" }`) or the raw text.
pub(crate) fn error_message(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    if let Ok(v) = serde_json::from_slice::<Value>(bytes) {
        let err = v.get("error").and_then(Value::as_str);
        let msg = v.get("message").and_then(Value::as_str);
        match (err, msg) {
            (Some(e), Some(m)) => return truncate_body(&format!("{e}: {m}"), 300),
            (Some(e), None) => return truncate_body(e, 300),
            (None, Some(m)) => return truncate_body(m, 300),
            _ => {}
        }
    }
    truncate_body(text.trim(), 300)
}

/// A decoded response.
#[derive(Debug, Clone)]
pub struct C8yResponse {
    pub status: u16,
    /// JSON body; `Null` if the body was empty.
    pub body: Value,
}

/// Request body.
pub(crate) enum ReqBody {
    None,
    Json(Value),
    /// Multipart form plus a short description for the activity log.
    Multipart(reqwest::multipart::Form, String),
}

/// Everything needed for one request. Built by the helpers; `accept` / `content_type` default
/// to [`media::for_request`].
pub(crate) struct RequestSpec {
    pub method: Method,
    /// Path + optional query, starting with `/`.
    pub path: String,
    pub body: ReqBody,
    pub accept: Option<&'static str>,
    pub content_type: Option<&'static str>,
    pub timeout: Option<Duration>,
    /// 401 reports `auth-expired` (false only for `current_user`).
    pub report_unauthorized: bool,
}

impl RequestSpec {
    pub fn new(method: Method, path: impl Into<String>) -> Self {
        Self {
            method,
            path: path.into(),
            body: ReqBody::None,
            accept: None,
            content_type: None,
            timeout: None,
            report_unauthorized: true,
        }
    }

    pub fn json(mut self, body: Value) -> Self {
        self.body = ReqBody::Json(body);
        self
    }

    pub fn accept(mut self, accept: &'static str) -> Self {
        self.accept = Some(accept);
        self
    }

    pub fn content_type(mut self, ct: &'static str) -> Self {
        self.content_type = Some(ct);
        self
    }

    pub fn timeout(mut self, t: Duration) -> Self {
        self.timeout = Some(t);
        self
    }
}

/// Result of [`C8yClient::send_raw`]: the transport result plus the (not yet recorded) log entry.
pub(crate) struct RawOutcome {
    pub result: Result<(u16, Vec<u8>), reqwest::Error>,
    pub entry: ActivityEntry,
}

/// An authenticated session against one tenant. Cheap to clone (shares the
/// reqwest connection pool and the activity log).
///
/// Lives in `AppState::auth` while logged in; the sim session task gets a clone.
#[derive(Clone)]
pub struct C8yClient {
    http: reqwest::Client,
    tenant: Tenant,
    /// User name as entered (without `tenantId/` prefix).
    username: String,
    /// User name as sent (see [`basic_auth_user`]).
    auth_user: String,
    /// Kept only in memory (and in the keychain when "remember me" is set).
    password: String,
    activity: Arc<ActivityLog>,
    /// Reports `activity-logged` and `auth-expired` (401) to the app.
    sink: Arc<dyn EventSink>,
    /// Server clock minus local clock in ms, from the `Date` header of the last response.
    clock_skew_ms: Arc<std::sync::atomic::AtomicI64>,
}

fn build_http(request_timeout: Duration) -> AppResult<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(concat!(
            "lichtwart-demo-simulator/",
            env!("CARGO_PKG_VERSION")
        ))
        .timeout(request_timeout)
        .connect_timeout(CONNECT_TIMEOUT)
        // Server affinity cookies (load balancers, `BAYEUX_BROWSER`) must come back, or a
        // multi-node tenant forgets the Bayeux client between requests.
        .cookie_store(true)
        .build()
        .map_err(|e| AppError::Internal(e.to_string()))
}

impl std::fmt::Debug for C8yClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("C8yClient")
            .field("tenant", &self.tenant.key)
            .field("username", &self.username)
            .finish_non_exhaustive()
    }
}

impl C8yClient {
    /// Build a client (does not touch the network).
    pub fn new(
        tenant: Tenant,
        username: String,
        password: String,
        activity: Arc<ActivityLog>,
        sink: Arc<dyn EventSink>,
    ) -> AppResult<Self> {
        let http = build_http(REQUEST_TIMEOUT)?;
        let base_url = tenant.base_url.trim_end_matches('/').to_owned();
        let tenant = Tenant { base_url, ..tenant };
        let auth_user = basic_auth_user(&tenant.tenant_id, &username);
        Ok(Self {
            http,
            tenant,
            username,
            auth_user,
            password,
            activity,
            sink,
            clock_skew_ms: Arc::default(),
        })
    }

    /// Shared handle on the measured clock skew (server − local, ms). Updated by every response.
    pub fn clock_skew(&self) -> Arc<std::sync::atomic::AtomicI64> {
        self.clock_skew_ms.clone()
    }

    /// Replace the default per-request timeout ([`REQUEST_TIMEOUT`]). Requests that set their own
    /// `RequestSpec::timeout` (realtime long-polls, binaries) are not affected. Tests use this to
    /// keep timeout paths fast.
    pub fn with_request_timeout(mut self, timeout: Duration) -> AppResult<Self> {
        self.http = build_http(timeout)?;
        Ok(self)
    }

    pub fn tenant(&self) -> &Tenant {
        &self.tenant
    }

    pub fn username(&self) -> &str {
        &self.username
    }

    /// Basic-auth user name as sent: `tenantId/user` (see [`basic_auth_user`]).
    pub fn auth_user(&self) -> String {
        self.auth_user.clone()
    }

    /// Append `entry` to the activity log (fills `base_url` / `user`, never headers or the
    /// password) and notify the sink (`activity-logged`). Call once per request.
    pub fn record(&self, mut entry: ActivityEntry) -> ActivityEntry {
        entry.base_url = self.tenant.base_url.clone();
        entry.user = self.auth_user();
        let entry = self.activity.push(entry);
        self.sink.activity_logged(&entry);
        entry
    }

    /// Call after every authenticated response: a 401 means the credentials are no longer
    /// valid; the sink clears the login and stops the session in the backend.
    /// (Not for `current_user`: a 401 there is `AuthFailed`.)
    pub fn check_unauthorized(&self, status: u16) {
        if status == 401 {
            self.sink.auth_expired(&self.tenant.key);
        }
    }

    // ───────────────────────────── transport ─────────────────────────────

    /// Send one request. Does **not** record the log entry (the caller decides; see
    /// [`Self::execute_bytes`]); never logs headers or the password.
    pub(crate) async fn send_raw(&self, spec: RequestSpec) -> RawOutcome {
        let RequestSpec {
            method,
            path,
            mut body,
            accept,
            content_type,
            timeout,
            ..
        } = spec;
        if let ReqBody::Json(v) = &mut body {
            ensure_time(&method, &path, v);
        }
        let json_body = match &body {
            ReqBody::Json(v) => Some(v),
            _ => None,
        };
        let media = media::for_request(method.as_str(), &path, json_body);
        let accept = accept.unwrap_or(media.accept);

        let url = format!("{}{}", self.tenant.base_url, path);
        let mut req = self
            .http
            .request(method.clone(), &url)
            .basic_auth(&self.auth_user, Some(&self.password))
            .header("UseXBasic", "true")
            .header(reqwest::header::ACCEPT, accept);
        if let Some(t) = timeout {
            req = req.timeout(t);
        }
        let logged_body = match body {
            ReqBody::None => None,
            ReqBody::Json(v) => {
                let bytes = serde_json::to_vec(&v).unwrap_or_default();
                let ct = content_type.or(media.content_type).unwrap_or(media::JSON);
                let logged = truncate_body(&String::from_utf8_lossy(&bytes), LOG_BODY_LIMIT);
                req = req.header(reqwest::header::CONTENT_TYPE, ct).body(bytes);
                Some(logged)
            }
            ReqBody::Multipart(form, summary) => {
                req = req.multipart(form);
                Some(summary)
            }
        };

        let time = now_iso();
        let started = Instant::now();
        let skew = self.clock_skew_ms.clone();
        let result = async {
            let resp = req.send().await?;
            let status = resp.status().as_u16();
            if let Some(date) = resp
                .headers()
                .get(reqwest::header::DATE)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| chrono::DateTime::parse_from_rfc2822(v).ok())
            {
                let ms = (date.with_timezone(&chrono::Utc) - chrono::Utc::now()).num_milliseconds();
                skew.store(ms, std::sync::atomic::Ordering::Relaxed);
            }
            let bytes = resp.bytes().await?;
            Ok::<_, reqwest::Error>((status, bytes.to_vec()))
        }
        .await;
        let duration_ms = u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX);

        let (status, response_body, error) = match &result {
            Ok((status, bytes)) => {
                let text = match std::str::from_utf8(bytes) {
                    Ok("") => None,
                    Ok(t) => Some(truncate_body(t, LOG_BODY_LIMIT)),
                    Err(_) => Some(format!("[{} bytes]", bytes.len())),
                };
                (Some(*status), text, None)
            }
            Err(e) => (None, None, Some(e.to_string())),
        };
        RawOutcome {
            result,
            entry: ActivityEntry {
                id: 0,
                time,
                method: method.as_str().to_owned(),
                base_url: String::new(),
                user: String::new(),
                path,
                status,
                duration_ms,
                request_body: logged_body,
                response_body,
                error,
            },
        }
    }

    /// Send, record the activity entry, map errors: transport → `Network`, 401 → `auth-expired`
    /// + `NotAuthenticated` (or `AuthFailed` without reporting), other non-2xx → `Http`.
    pub(crate) async fn execute_bytes(&self, spec: RequestSpec) -> AppResult<(u16, Vec<u8>)> {
        let report = spec.report_unauthorized;
        let outcome = self.send_raw(spec).await;
        self.record(outcome.entry);
        let (status, bytes) = outcome.result.map_err(map_transport)?;
        check_status(self, status, &bytes, report)?;
        Ok((status, bytes))
    }

    /// Send and decode the JSON response (empty body = `Null`).
    pub(crate) async fn execute(&self, spec: RequestSpec) -> AppResult<C8yResponse> {
        let (status, bytes) = self.execute_bytes(spec).await?;
        let body = if bytes.iter().all(u8::is_ascii_whitespace) {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .map_err(|e| AppError::Internal(format!("invalid JSON from Cumulocity: {e}")))?
        };
        Ok(C8yResponse { status, body })
    }

    /// Send any JSON request and return status + body. `path` starts with `/` and may carry
    /// a query. Headers follow [`media::for_request`]. Used by raw fire.
    pub async fn request_json(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> AppResult<C8yResponse> {
        let mut spec = RequestSpec::new(method, path);
        if let Some(b) = body {
            spec = spec.json(b);
        }
        self.execute(spec).await
    }

    // ───────────────────────────── generic helpers ─────────────────────────────

    /// Validate the credentials with `GET /user/currentUser` (PRD §5). 401 → `AuthFailed`
    /// (no `auth-expired`).
    pub async fn current_user(&self) -> AppResult<AuthState> {
        let mut spec = RequestSpec::new(Method::GET, "/user/currentUser");
        spec.report_unauthorized = false;
        let user = self.execute(spec).await?.body;
        let str_field = |k: &str| {
            user.get(k)
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .map(str::to_owned)
        };
        let user_id = str_field("id")
            .or_else(|| str_field("userName"))
            .unwrap_or_else(|| self.username.clone());
        let full_name = match (str_field("firstName"), str_field("lastName")) {
            (Some(f), Some(l)) => Some(format!("{f} {l}")),
            (Some(n), None) | (None, Some(n)) => Some(n),
            (None, None) => None,
        };
        let display_name = str_field("displayName")
            .or(full_name)
            .or_else(|| str_field("email"));
        Ok(AuthState {
            tenant: self.tenant.clone(),
            username: self.username.clone(),
            user_id,
            display_name,
        })
    }

    /// `GET <path>` and decode JSON.
    pub async fn get_json<T: DeserializeOwned>(&self, path: &str) -> AppResult<T> {
        let resp = self.execute(RequestSpec::new(Method::GET, path)).await?;
        decode(resp.body)
    }

    /// `POST <path>` with a JSON body and decode the JSON response.
    /// Measurement / alarm / event bodies without `time` get `time = now`.
    pub async fn post_json<B: Serialize + Sync, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> AppResult<T> {
        let body = serde_json::to_value(body)?;
        let resp = self
            .execute(RequestSpec::new(Method::POST, path).json(body))
            .await?;
        decode(resp.body)
    }

    /// `PUT <path>` with a JSON body and decode the JSON response.
    pub async fn put_json<B: Serialize + Sync, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> AppResult<T> {
        let body = serde_json::to_value(body)?;
        let resp = self
            .execute(RequestSpec::new(Method::PUT, path).json(body))
            .await?;
        decode(resp.body)
    }

    /// `DELETE <path>`.
    pub async fn delete(&self, path: &str) -> AppResult<()> {
        self.execute_bytes(RequestSpec::new(Method::DELETE, path))
            .await
            .map(|_| ())
    }
}

fn decode<T: DeserializeOwned>(v: Value) -> AppResult<T> {
    serde_json::from_value(v)
        .map_err(|e| AppError::Internal(format!("unexpected Cumulocity response: {e}")))
}

/// Transport failures (DNS, TLS, timeout, reset) → `Network`.
pub(crate) fn map_transport(e: reqwest::Error) -> AppError {
    if e.is_timeout() {
        AppError::Network(format!("timeout: {e}"))
    } else {
        AppError::Network(e.to_string())
    }
}

fn check_status(client: &C8yClient, status: u16, bytes: &[u8], report: bool) -> AppResult<()> {
    if (200..300).contains(&status) {
        return Ok(());
    }
    if status == 401 {
        if report {
            client.check_unauthorized(status);
            return Err(AppError::NotAuthenticated);
        }
        return Err(AppError::AuthFailed(
            Some(error_message(bytes)).filter(|m| !m.is_empty()),
        ));
    }
    Err(AppError::Http {
        status,
        message: error_message(bytes),
    })
}

/// True if `e` is an HTTP error with this status (e.g. 404).
pub fn is_http_status(e: &AppError, status: u16) -> bool {
    matches!(e, AppError::Http { status: s, .. } if *s == status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_user_always_carries_the_tenant() {
        assert_eq!(basic_auth_user("t819292026", "sl"), "t819292026/sl");
        assert_eq!(basic_auth_user("t1", "t1/sl"), "t1/sl");
        assert_eq!(basic_auth_user("", "sl"), "sl");
    }

    #[test]
    fn truncates_on_char_boundary() {
        assert_eq!(truncate_body("abc", 5), "abc");
        assert_eq!(truncate_body("äää", 3), "ä…");
        assert_eq!(truncate_body("abcdef", 3), "abc…");
    }

    #[test]
    fn fills_missing_time_only() {
        let mut m = serde_json::json!({ "type": "x" });
        ensure_time(&Method::POST, "/measurement/measurements", &mut m);
        assert!(m["time"].as_str().unwrap().ends_with('Z'));

        let mut keep = serde_json::json!({ "time": "2020-01-01T00:00:00Z" });
        ensure_time(&Method::POST, "/alarm/alarms", &mut keep);
        assert_eq!(keep["time"], "2020-01-01T00:00:00Z");

        let mut coll = serde_json::json!({ "measurements": [{ "a": 1 }, { "time": "t" }] });
        ensure_time(&Method::POST, "/measurement/measurements", &mut coll);
        assert!(coll["measurements"][0]["time"].is_string());
        assert_eq!(coll["measurements"][1]["time"], "t");

        let mut put = serde_json::json!({ "status": "CLEARED" });
        ensure_time(&Method::PUT, "/alarm/alarms/1", &mut put);
        assert!(put.get("time").is_none());
        let mut inv = serde_json::json!({});
        ensure_time(&Method::POST, "/inventory/managedObjects", &mut inv);
        assert!(inv.get("time").is_none());
    }

    #[test]
    fn error_message_from_c8y_body() {
        assert_eq!(
            error_message(br#"{"error":"inventory/Not Found","message":"Finding failed"}"#),
            "inventory/Not Found: Finding failed"
        );
        assert_eq!(error_message(b"plain"), "plain");
    }

    #[test]
    fn iso_time_format() {
        let t = now_iso();
        assert_eq!(t.len(), "2026-10-12T12:00:00.000Z".len());
        assert!(t.ends_with('Z'));
    }
}
