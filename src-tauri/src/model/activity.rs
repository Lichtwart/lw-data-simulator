//! Activity log types (PRD §16).

use serde::{Deserialize, Serialize};
use specta::Type;

/// One HTTP request made to Cumulocity. The log keeps the last 500, in memory only.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEntry {
    /// Monotonic id within this app run.
    pub id: u32,
    /// ISO-8601 request start time.
    pub time: String,
    pub method: String,
    /// Base URL of the tenant, e.g. `https://example.cumulocity.com` (for `to_curl`).
    pub base_url: String,
    /// Basic-auth user name as sent (`tenantId/user` on foreign domains). The password and
    /// request headers are never stored.
    pub user: String,
    /// Path + query, without host.
    pub path: String,
    /// HTTP status, `None` on transport error.
    pub status: Option<u16>,
    pub duration_ms: u32,
    /// Shortened request body (≤ 2 kB; a curl built from a truncated body is incomplete).
    pub request_body: Option<String>,
    /// Shortened response body (≤ 2 kB).
    pub response_body: Option<String>,
    /// Transport error message, if any.
    pub error: Option<String>,
}
