//! In-memory activity log: last [`CAPACITY`] requests (PRD §16).

use std::collections::VecDeque;
use std::sync::Mutex;

use crate::model::ActivityEntry;

/// Max entries kept.
pub const CAPACITY: usize = 500;

/// Thread-safe ring buffer. Shared via `Arc` between `AppState` and every
/// `C8yClient`. Pushing does not emit events by itself — the client emits
/// `activity-logged` after `push`.
#[derive(Debug, Default)]
pub struct ActivityLog {
    inner: Mutex<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    next_id: u32,
    entries: VecDeque<ActivityEntry>,
}

impl ActivityLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append an entry; assigns and returns its id. Oldest entries are dropped.
    pub fn push(&self, mut entry: ActivityEntry) -> ActivityEntry {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner.next_id = inner.next_id.wrapping_add(1);
        entry.id = inner.next_id;
        if inner.entries.len() >= CAPACITY {
            inner.entries.pop_front();
        }
        inner.entries.push_back(entry.clone());
        entry
    }

    /// Snapshot, oldest first.
    pub fn entries(&self) -> Vec<ActivityEntry> {
        let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner.entries.iter().cloned().collect()
    }

    pub fn get(&self, id: u32) -> Option<ActivityEntry> {
        let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner.entries.iter().find(|e| e.id == id).cloned()
    }

    pub fn clear(&self) {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner.entries.clear();
    }
}

/// Render an entry as a `curl` command. Needs only the entry (it carries base URL + user).
/// The password is always masked (`-u 'tenant/user:****'`); headers are not part of the log,
/// so `Accept` / `Content-Type` are derived again from method + path + body
/// ([`super::media::for_request`]), plus the fixed `UseXBasic: true`. A binary download is
/// the one request with its own `Accept: */*`. A body cut at 2 kB makes the command
/// incomplete (it ends with `# body truncated`); a binary upload is rendered as `-F` parts
/// with a placeholder file.
pub fn to_curl(entry: &ActivityEntry) -> String {
    let url = format!("{}{}", entry.base_url, entry.path);
    let truncated = entry
        .request_body
        .as_deref()
        .is_some_and(|b| b.ends_with('…'));
    let parsed: Option<serde_json::Value> = entry
        .request_body
        .as_deref()
        .and_then(|b| serde_json::from_str(b).ok());
    // A cut body no longer parses; keep the one hint the media type depends on.
    let truncated_collection = truncated
        && parsed.is_none()
        && entry
            .request_body
            .as_deref()
            .is_some_and(|b| b.trim_start().starts_with("{\"measurements\""));
    let collection_hint = serde_json::json!({ "measurements": [] });
    let body_hint = if truncated_collection {
        Some(&collection_hint)
    } else {
        parsed.as_ref().or(entry
            .request_body
            .as_ref()
            .map(|_| &serde_json::Value::Null))
    };
    let media = super::media::for_request(&entry.method, &entry.path, body_hint);
    let is_download = entry.method == "GET"
        && entry
            .path
            .split('?')
            .next()
            .is_some_and(|p| p.starts_with("/inventory/binaries/"));
    let accept = if is_download { "*/*" } else { media.accept };
    let mut parts = vec![
        format!("curl -X {}", entry.method),
        shell_quote(&url),
        format!("-u {}", shell_quote(&format!("{}:****", entry.user))),
        format!("-H {}", shell_quote(&format!("Accept: {accept}"))),
        "-H 'UseXBasic: true'".to_owned(),
    ];
    match (&entry.request_body, media.content_type) {
        (Some(body), Some(super::media::MULTIPART)) => {
            let object = body
                .split_once("object=")
                .and_then(|(_, rest)| rest.split_once(" file=").map(|(o, _)| o))
                .unwrap_or("{}");
            parts.push(format!("-F {}", shell_quote(&format!("object={object}"))));
            parts.push("-F 'file=@<file>'".to_owned());
        }
        (Some(body), ct) => {
            let ct = ct.unwrap_or(super::media::JSON);
            parts.push(format!(
                "-H {}",
                shell_quote(&format!("Content-Type: {ct}"))
            ));
            parts.push(format!("-d {}", shell_quote(body)));
        }
        (None, _) => {}
    }
    if truncated {
        parts.push("# body truncated".to_owned());
    }
    parts.join(" ")
}

/// POSIX single-quote quoting.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> ActivityEntry {
        ActivityEntry {
            id: 0,
            time: "2026-10-12T12:00:00Z".into(),
            method: "GET".into(),
            base_url: "https://test.example.com".into(),
            user: "t123/sl".into(),
            path: "/user/currentUser".into(),
            status: Some(200),
            duration_ms: 12,
            request_body: None,
            response_body: None,
            error: None,
        }
    }

    #[test]
    fn keeps_last_500() {
        let log = ActivityLog::new();
        for _ in 0..(CAPACITY + 10) {
            log.push(entry());
        }
        let all = log.entries();
        assert_eq!(all.len(), CAPACITY);
        assert_eq!(all[0].id, 11);
        assert_eq!(all.last().unwrap().id, (CAPACITY + 10) as u32);
    }

    #[test]
    fn curl_masks_password_and_quotes() {
        let mut e = entry();
        e.method = "POST".into();
        e.request_body = Some(r#"{"text":"it's"}"#.into());
        let curl = to_curl(&e);
        assert!(curl.starts_with("curl -X POST 'https://test.example.com/user/currentUser'"));
        assert!(curl.contains("-u 't123/sl:****'"));
        assert!(curl.contains(r#"-d '{"text":"it'\''s"}'"#));
        assert!(!curl.contains("Authorization"));
    }

    #[test]
    fn curl_uses_cumulocity_media_types() {
        let mut e = entry();
        e.method = "POST".into();
        e.path = "/measurement/measurements".into();
        e.request_body = Some(r#"{"measurements":[]}"#.into());
        let curl = to_curl(&e);
        assert!(curl.contains(
            "-H 'Content-Type: application/vnd.com.nsn.cumulocity.measurementcollection+json'"
        ));
        assert!(curl.contains("-H 'UseXBasic: true'"));

        e.method = "GET".into();
        e.path = "/alarm/alarms?source=1".into();
        e.request_body = None;
        let curl = to_curl(&e);
        assert!(curl.contains("Accept: application/vnd.com.nsn.cumulocity.alarmcollection+json"));
        assert!(!curl.contains("Content-Type"));
        assert!(!curl.contains(" -d "));
    }

    #[test]
    fn curl_download_accept_and_truncated_body() {
        let mut e = entry();
        e.path = "/inventory/binaries/42".into();
        assert!(to_curl(&e).contains("-H 'Accept: */*'"));

        let mut e = entry();
        e.method = "POST".into();
        e.path = "/measurement/measurements".into();
        e.request_body = Some(format!(
            r#"{{"measurements":[{{"type":"{}…"#,
            "x".repeat(50)
        ));
        let curl = to_curl(&e);
        assert!(curl.contains("measurementcollection+json"), "{curl}");
        assert!(curl.ends_with("# body truncated"));
    }

    #[test]
    fn curl_for_binary_upload() {
        let mut e = entry();
        e.method = "POST".into();
        e.path = "/inventory/binaries".into();
        e.request_body = Some(
            r#"[multipart] object={"name":"a.png","type":"image/png"} file=a.png (3 bytes)"#.into(),
        );
        let curl = to_curl(&e);
        assert!(curl.contains(r#"-F 'object={"name":"a.png","type":"image/png"}'"#));
        assert!(curl.contains("-F 'file=@<file>'"));
        assert!(curl.contains("-u 't123/sl:****'"));
    }
}
