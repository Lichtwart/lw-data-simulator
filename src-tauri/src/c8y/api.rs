//! Typed Cumulocity REST helpers (more `impl C8yClient` blocks).
//!
//! Bodies and results are `serde_json::Value` on purpose: the payload / device modules shape
//! them, this module only knows paths, media types, paging and status codes.
//! Every measurement / alarm / event POST gets `time = now` if the body has none.

use reqwest::Method;
use serde_json::{Value, json};

use super::client::{C8yClient, ReqBody, RequestSpec, is_http_status};
use super::media;
use crate::error::{AppError, AppResult};
use crate::model::OperationStatus;

/// Default (and Cumulocity's maximum) page size used by the collection helpers.
pub const PAGE_SIZE: u32 = 2000;
/// Safety cap for paging loops (a partial result is logged with `warn!`).
pub const MAX_PAGES: u32 = 50;
/// Timeout of an operation status `PUT`: a hanging one must not hold back the others (the
/// plugin cancels a PENDING operation after 10 s).
pub const STATUS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
/// Timeout of a binary upload / download (floorplan images on slow links).
pub const BINARY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// `dateFrom` for `revert=true` queries that want "everything, newest first".
pub const EPOCH: &str = "1970-01-01T00:00:00Z";

/// Percent-encode a query component.
fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// `path?k=v&…` with encoded values (pairs with an empty value are kept).
pub fn with_query(path: &str, params: &[(&str, String)]) -> String {
    if params.is_empty() {
        return path.to_owned();
    }
    let q: Vec<String> = params
        .iter()
        .map(|(k, v)| format!("{}={}", enc(k), enc(v)))
        .collect();
    format!("{path}?{}", q.join("&"))
}

/// `id` of a created / fetched object (Cumulocity ids are strings).
pub fn id_of(v: &Value) -> AppResult<String> {
    match v.get("id") {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(Value::Number(n)) => Ok(n.to_string()),
        _ => Err(AppError::Internal("Cumulocity response without id".into())),
    }
}

/// Alarm status (Cumulocity).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlarmStatus {
    Active,
    Acknowledged,
    Cleared,
}

impl AlarmStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::Acknowledged => "ACKNOWLEDGED",
            Self::Cleared => "CLEARED",
        }
    }
}

/// `OperationStatus` as Cumulocity spells it.
pub fn operation_status_str(s: OperationStatus) -> &'static str {
    match s {
        OperationStatus::Pending => "PENDING",
        OperationStatus::Executing => "EXECUTING",
        OperationStatus::Successful => "SUCCESSFUL",
        OperationStatus::Failed => "FAILED",
    }
}

/// Filters for `GET /inventory/managedObjects`. Empty fields are not sent.
#[derive(Debug, Clone, Default)]
pub struct ManagedObjectQuery {
    /// Inventory query language, e.g. `$filter=(c8y_DeviceTypes eq 'demo') $orderby=name`.
    pub query: Option<String>,
    /// `type=` exact match.
    pub object_type: Option<String>,
    /// `fragmentType=`.
    pub fragment_type: Option<String>,
    /// `text=` full-text search.
    pub text: Option<String>,
    /// `withParents=true` (assetParents / deviceParents).
    pub with_parents: bool,
    /// Further raw parameters.
    pub extra: Vec<(String, String)>,
    /// Page size (default [`PAGE_SIZE`]).
    pub page_size: Option<u32>,
    /// Stop after this many objects (default: all, up to [`MAX_PAGES`] pages).
    pub limit: Option<usize>,
}

impl ManagedObjectQuery {
    /// Query parameters. The inventory query language (`query`) is not combinable with the
    /// plain filters (Cumulocity may ignore them), so the mix is rejected: put `type eq '…'` /
    /// `has(…)` into the `$filter` instead.
    fn params(&self) -> AppResult<Vec<(&str, String)>> {
        if self.query.is_some()
            && (self.object_type.is_some() || self.fragment_type.is_some() || self.text.is_some())
        {
            return Err(AppError::InvalidInput(
                "query cannot be combined with type / fragmentType / text; put them into the $filter"
                    .into(),
            ));
        }
        let mut p: Vec<(&str, String)> = Vec::new();
        if let Some(q) = &self.query {
            p.push(("query", q.clone()));
        }
        if let Some(t) = &self.object_type {
            p.push(("type", t.clone()));
        }
        if let Some(f) = &self.fragment_type {
            p.push(("fragmentType", f.clone()));
        }
        if let Some(t) = &self.text {
            p.push(("text", t.clone()));
        }
        if self.with_parents {
            p.push(("withParents", "true".into()));
        }
        for (k, v) in &self.extra {
            p.push((k.as_str(), v.clone()));
        }
        Ok(p)
    }
}

impl C8yClient {
    /// GET a collection page by page (`pageSize`, `currentPage`) and return the elements under
    /// `key` (e.g. `managedObjects`). The first page asks for `withTotalPages=true` and the loop
    /// stops at `statistics.totalPages`; without that field a short page ends it.
    pub async fn get_collection(
        &self,
        path: &str,
        params: &[(&str, String)],
        key: &str,
        page_size: u32,
        limit: Option<usize>,
    ) -> AppResult<Vec<Value>> {
        let page_size = page_size.clamp(1, PAGE_SIZE);
        let mut out = Vec::new();
        let mut total_pages: Option<u64> = None;
        for page in 1..=MAX_PAGES {
            let mut p: Vec<(&str, String)> = params.to_vec();
            p.push(("pageSize", page_size.to_string()));
            p.push(("currentPage", page.to_string()));
            if page == 1 {
                p.push(("withTotalPages", "true".into()));
            }
            let resp = self
                .execute(RequestSpec::new(Method::GET, with_query(path, &p)))
                .await?;
            if page == 1 {
                total_pages = resp
                    .body
                    .pointer("/statistics/totalPages")
                    .and_then(Value::as_u64);
            }
            let items = match resp.body.get(key) {
                Some(Value::Array(a)) => a.clone(),
                _ => Vec::new(),
            };
            let n = items.len();
            out.extend(items);
            if let Some(l) = limit
                && out.len() >= l
            {
                out.truncate(l);
                break;
            }
            let done = match total_pages {
                Some(total) => u64::from(page) >= total,
                None => n < page_size as usize,
            };
            if done {
                break;
            }
            if page == MAX_PAGES {
                log::warn!(
                    "{path}: stopped after {MAX_PAGES} pages ({} objects); result is partial",
                    out.len()
                );
            }
        }
        Ok(out)
    }

    // ───────────────────────────── inventory ─────────────────────────────

    /// `GET /inventory/managedObjects/{id}`.
    pub async fn get_managed_object(&self, id: &str) -> AppResult<Value> {
        self.get_json(&format!("/inventory/managedObjects/{id}"))
            .await
    }

    /// Like [`Self::get_managed_object`], but 404 → `None`.
    pub async fn find_managed_object(&self, id: &str) -> AppResult<Option<Value>> {
        match self.get_managed_object(id).await {
            Ok(v) => Ok(Some(v)),
            Err(e) if is_http_status(&e, 404) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// `GET /inventory/managedObjects?…` over all pages.
    pub async fn query_managed_objects(&self, q: &ManagedObjectQuery) -> AppResult<Vec<Value>> {
        self.get_collection(
            "/inventory/managedObjects",
            &q.params()?,
            "managedObjects",
            q.page_size.unwrap_or(PAGE_SIZE),
            q.limit,
        )
        .await
    }

    /// `POST /inventory/managedObjects`; returns the created object (with `id`).
    pub async fn create_managed_object(&self, body: &Value) -> AppResult<Value> {
        self.post_json("/inventory/managedObjects", body).await
    }

    /// `PUT /inventory/managedObjects/{id}` (partial: only the given top-level fragments
    /// are replaced); returns the updated object.
    pub async fn update_managed_object(&self, id: &str, body: &Value) -> AppResult<Value> {
        self.put_json(&format!("/inventory/managedObjects/{id}"), body)
            .await
    }

    /// `DELETE /inventory/managedObjects/{id}`.
    pub async fn delete_managed_object(&self, id: &str) -> AppResult<()> {
        self.delete(&format!("/inventory/managedObjects/{id}"))
            .await
    }

    /// `POST /inventory/managedObjects/{parent}/childAdditions` `{managedObject:{id}}`.
    pub async fn add_child_addition(&self, parent_id: &str, child_id: &str) -> AppResult<()> {
        let _: Value = self
            .post_json(
                &format!("/inventory/managedObjects/{parent_id}/childAdditions"),
                &json!({ "managedObject": { "id": child_id } }),
            )
            .await?;
        Ok(())
    }

    /// Child additions of `parent_id` (the referenced managed objects, unwrapped from
    /// `references[].managedObject`). There is no server-side type filter; filter by `type`.
    pub async fn list_child_additions(&self, parent_id: &str) -> AppResult<Vec<Value>> {
        let refs = self
            .get_collection(
                &format!("/inventory/managedObjects/{parent_id}/childAdditions"),
                &[],
                "references",
                PAGE_SIZE,
                None,
            )
            .await?;
        Ok(refs
            .into_iter()
            .filter_map(|mut r| r.get_mut("managedObject").map(Value::take))
            .collect())
    }

    /// `DELETE /inventory/managedObjects/{parent}/childAdditions/{child}` (unlink only).
    pub async fn remove_child_addition(&self, parent_id: &str, child_id: &str) -> AppResult<()> {
        self.delete(&format!(
            "/inventory/managedObjects/{parent_id}/childAdditions/{child_id}"
        ))
        .await
    }

    /// `POST /inventory/binaries` (multipart `object` = `{name,type}`, `filesize`, `file`);
    /// returns the binary's managed object (its `id` is the binary id).
    pub async fn upload_binary(&self, name: &str, mime: &str, bytes: Vec<u8>) -> AppResult<Value> {
        use reqwest::multipart::{Form, Part};
        let size = bytes.len();
        let object = json!({ "name": name, "type": mime }).to_string();
        let file = Part::bytes(bytes)
            .file_name(name.to_owned())
            .mime_str(mime)
            .map_err(|e| AppError::InvalidInput(format!("mime type {mime}: {e}")))?;
        let form = Form::new()
            .text("object", object.clone())
            .text("filesize", size.to_string())
            .part("file", file);
        let mut spec =
            RequestSpec::new(Method::POST, "/inventory/binaries").timeout(BINARY_TIMEOUT);
        spec.body = ReqBody::Multipart(
            form,
            format!("[multipart] object={object} file={name} ({size} bytes)"),
        );
        Ok(self.execute(spec).await?.body)
    }

    /// `GET /inventory/binaries/{id}` → raw bytes.
    pub async fn download_binary(&self, id: &str) -> AppResult<Vec<u8>> {
        let spec = RequestSpec::new(Method::GET, format!("/inventory/binaries/{id}"))
            .accept("*/*")
            .timeout(BINARY_TIMEOUT);
        Ok(self.execute_bytes(spec).await?.1)
    }

    /// `DELETE /inventory/binaries/{id}`.
    pub async fn delete_binary(&self, id: &str) -> AppResult<()> {
        self.delete(&format!("/inventory/binaries/{id}")).await
    }

    // ───────────────────────────── measurements ─────────────────────────────

    /// `POST /measurement/measurements` (one measurement, `…measurement+json`).
    pub async fn create_measurement(&self, measurement: &Value) -> AppResult<Value> {
        self.post_json("/measurement/measurements", measurement)
            .await
    }

    /// `POST /measurement/measurements` with `{ "measurements": [...] }`
    /// (`…measurementcollection+json`). An empty slice sends nothing.
    pub async fn create_measurements(&self, measurements: &[Value]) -> AppResult<Value> {
        if measurements.is_empty() {
            return Ok(json!({ "measurements": [] }));
        }
        self.post_json(
            "/measurement/measurements",
            &json!({ "measurements": measurements }),
        )
        .await
    }

    // ───────────────────────────── alarms ─────────────────────────────

    /// `POST /alarm/alarms`. Cumulocity de-duplicates ACTIVE alarms by source + type.
    pub async fn create_alarm(&self, alarm: &Value) -> AppResult<Value> {
        self.post_json("/alarm/alarms", alarm).await
    }

    /// Alarms of `source` with the given statuses (empty = any) and optional types
    /// (comma-joined `type=` filter), all pages.
    pub async fn query_alarms(
        &self,
        source: &str,
        statuses: &[AlarmStatus],
        types: &[&str],
    ) -> AppResult<Vec<Value>> {
        let mut p: Vec<(&str, String)> = vec![("source", source.to_owned())];
        if !statuses.is_empty() {
            let s: Vec<&str> = statuses.iter().map(|s| s.as_str()).collect();
            p.push(("status", s.join(",")));
        }
        if !types.is_empty() {
            p.push(("type", types.join(",")));
        }
        self.get_collection("/alarm/alarms", &p, "alarms", PAGE_SIZE, None)
            .await
    }

    /// ACTIVE + ACKNOWLEDGED alarms of `source`, optionally of one type.
    pub async fn open_alarms(
        &self,
        source: &str,
        alarm_type: Option<&str>,
    ) -> AppResult<Vec<Value>> {
        let types: Vec<&str> = alarm_type.into_iter().collect();
        self.query_alarms(
            source,
            &[AlarmStatus::Active, AlarmStatus::Acknowledged],
            &types,
        )
        .await
    }

    /// `PUT /alarm/alarms/{id}` `{ "status": … }`.
    pub async fn update_alarm_status(&self, id: &str, status: AlarmStatus) -> AppResult<Value> {
        self.put_json(
            &format!("/alarm/alarms/{id}"),
            &json!({ "status": status.as_str() }),
        )
        .await
    }

    /// Clear one alarm.
    pub async fn clear_alarm(&self, id: &str) -> AppResult<Value> {
        self.update_alarm_status(id, AlarmStatus::Cleared).await
    }

    /// Clear every ACTIVE / ACKNOWLEDGED alarm of `source`; returns how many were open.
    ///
    /// The open alarms are listed first (that is the set to clear), then cleared with one bulk
    /// update per status, `PUT /alarm/alarms?source=…&status=ACTIVE` and `…=ACKNOWLEDGED`,
    /// `{status: CLEARED}`. When Cumulocity answers `200` the update is done: the list is read
    /// once more and what is still open is cleared one `PUT` at a time. On `202 Accepted`
    /// (finished in the background) or when a bulk update failed, every listed alarm is cleared
    /// one `PUT` at a time at once: that is idempotent with the background job, and polling
    /// would hold up the ordered outbound lane (measurements, operation results) behind it. A
    /// failing `PUT` does not stop the rest, the first error is returned. The background job
    /// clears whatever is open when it runs, so an alarm raised right after "Clear all" can be
    /// caught by it; the session accepts that. 401 stops at once.
    pub async fn clear_open_alarms(&self, source: &str) -> AppResult<u32> {
        let listed: Vec<String> = self
            .open_alarms(source, None)
            .await?
            .iter()
            .filter_map(|a| id_of(a).ok())
            .collect();
        if listed.is_empty() {
            return Ok(0);
        }
        let mut accepted = false;
        let mut bulk_ok = true;
        for status in [AlarmStatus::Active, AlarmStatus::Acknowledged] {
            let path = with_query(
                "/alarm/alarms",
                &[
                    ("source", source.to_owned()),
                    ("status", status.as_str().to_owned()),
                ],
            );
            match self
                .request_json(
                    Method::PUT,
                    &path,
                    Some(json!({ "status": AlarmStatus::Cleared.as_str() })),
                )
                .await
            {
                Ok(r) => accepted |= r.status == 202,
                Err(AppError::NotAuthenticated) => return Err(AppError::NotAuthenticated),
                Err(e) => {
                    log::warn!("bulk alarm clear on {source} failed, clearing one by one: {e}");
                    bulk_ok = false;
                    break;
                }
            }
        }
        let mut remaining = listed.clone();
        if bulk_ok && !accepted {
            let open: Vec<String> = self
                .open_alarms(source, None)
                .await?
                .iter()
                .filter_map(|a| id_of(a).ok())
                .collect();
            remaining.retain(|id| open.contains(id));
        }
        let mut first_err: Option<AppError> = None;
        for id in &remaining {
            match self.clear_alarm(id).await {
                Ok(_) => {}
                Err(AppError::NotAuthenticated) => return Err(AppError::NotAuthenticated),
                Err(e) => {
                    first_err.get_or_insert(e);
                }
            }
        }
        match first_err {
            None => Ok(u32::try_from(listed.len()).unwrap_or(u32::MAX)),
            Some(e) => {
                log::warn!(
                    "clear_open_alarms({source}): {} alarms, first error: {e}",
                    listed.len()
                );
                Err(e)
            }
        }
    }

    // ───────────────────────────── events ─────────────────────────────

    /// `POST /event/events`.
    pub async fn create_event(&self, event: &Value) -> AppResult<Value> {
        self.post_json("/event/events", event).await
    }

    /// Events of `source`, optionally of one type, newest first, at most `limit`.
    pub async fn query_events(
        &self,
        source: &str,
        event_type: Option<&str>,
        limit: usize,
    ) -> AppResult<Vec<Value>> {
        let mut p: Vec<(&str, String)> = vec![
            ("source", source.to_owned()),
            ("dateFrom", EPOCH.to_owned()),
            ("revert", "true".to_owned()),
        ];
        if let Some(t) = event_type {
            p.push(("type", t.to_owned()));
        }
        let page = u32::try_from(limit.clamp(1, PAGE_SIZE as usize)).unwrap_or(PAGE_SIZE);
        self.get_collection("/event/events", &p, "events", page, Some(limit))
            .await
    }

    /// Events of `source` of one type at or after `date_from` (an ISO-8601 instant), newest
    /// first, all pages: a window by time instead of by count.
    pub async fn query_events_since(
        &self,
        source: &str,
        event_type: &str,
        date_from: &str,
    ) -> AppResult<Vec<Value>> {
        let p: Vec<(&str, String)> = vec![
            ("source", source.to_owned()),
            ("dateFrom", date_from.to_owned()),
            ("revert", "true".to_owned()),
            ("type", event_type.to_owned()),
        ];
        self.get_collection("/event/events", &p, "events", PAGE_SIZE, None)
            .await
    }

    // ───────────────────────────── operations ─────────────────────────────

    /// Operations of a device, optionally filtered by status, all pages.
    pub async fn query_operations(
        &self,
        device_id: &str,
        status: Option<OperationStatus>,
    ) -> AppResult<Vec<Value>> {
        let mut p: Vec<(&str, String)> = vec![("deviceId", device_id.to_owned())];
        if let Some(s) = status {
            p.push(("status", operation_status_str(s).to_owned()));
        }
        self.get_collection(
            "/devicecontrol/operations",
            &p,
            "operations",
            PAGE_SIZE,
            None,
        )
        .await
    }

    /// The newest operations of a device with this status and command fragment (newest first,
    /// at most `limit`; `revert=true`).
    pub async fn newest_operations(
        &self,
        device_id: &str,
        status: OperationStatus,
        fragment_type: &str,
        limit: usize,
    ) -> AppResult<Vec<Value>> {
        let p: Vec<(&str, String)> = vec![
            ("deviceId", device_id.to_owned()),
            ("status", operation_status_str(status).to_owned()),
            ("fragmentType", fragment_type.to_owned()),
            ("dateFrom", EPOCH.to_owned()),
            ("revert", "true".to_owned()),
        ];
        let page = u32::try_from(limit.clamp(1, PAGE_SIZE as usize)).unwrap_or(PAGE_SIZE);
        self.get_collection(
            "/devicecontrol/operations",
            &p,
            "operations",
            page,
            Some(limit),
        )
        .await
    }

    /// PENDING operations of a device (used on every realtime (re)connect).
    pub async fn pending_operations(&self, device_id: &str) -> AppResult<Vec<Value>> {
        self.query_operations(device_id, Some(OperationStatus::Pending))
            .await
    }

    /// `PUT /devicecontrol/operations/{id}` `{ status, failureReason? }` (`failureReason`
    /// is only sent with FAILED).
    pub async fn set_operation_status(
        &self,
        id: &str,
        status: OperationStatus,
        failure_reason: Option<&str>,
    ) -> AppResult<Value> {
        let mut body = json!({ "status": operation_status_str(status) });
        if status == OperationStatus::Failed {
            body["failureReason"] = json!(failure_reason.unwrap_or("Failed"));
        }
        let spec = RequestSpec::new(Method::PUT, format!("/devicecontrol/operations/{id}"))
            .json(body)
            .accept(media::OPERATION)
            .content_type(media::OPERATION)
            .timeout(STATUS_TIMEOUT);
        Ok(self.execute(spec).await?.body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_encoding() {
        assert_eq!(
            with_query(
                "/inventory/managedObjects",
                &[
                    ("query", "$filter=(name eq 'a b')".into()),
                    ("pageSize", "5".into())
                ]
            ),
            "/inventory/managedObjects?query=%24filter%3D%28name%20eq%20%27a%20b%27%29&pageSize=5"
        );
        assert_eq!(with_query("/x", &[]), "/x");
    }

    #[test]
    fn ids_are_strings() {
        assert_eq!(id_of(&json!({"id": "12"})).unwrap(), "12");
        assert_eq!(id_of(&json!({"id": 12})).unwrap(), "12");
        assert!(id_of(&json!({})).is_err());
    }
}
