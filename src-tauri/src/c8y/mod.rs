//! Cumulocity I/O. **All** HTTP and realtime traffic of the app goes through this
//! module (PRD §4: no CORS, no proxy — React never talks to Cumulocity).
//!
//! Ownership:
//! - [`client`]: `C8yClient` — reqwest client with Basic auth (`UseXBasic: true`,
//!   `tenantId/user` on foreign domains), generic JSON helpers, error mapping; logs every
//!   request to the [`activity`] log and, on 401, reports `auth-expired` through its [`sink`].
//! - [`api`]: typed helpers on `C8yClient` for inventory (+ paging, child additions,
//!   binaries), measurements, alarms, events, operations.
//! - [`media`]: `application/vnd.com.nsn.cumulocity.*` media types per request.
//! - [`sink`]: `EventSink` (`activity-logged`, `auth-expired`), implemented by `TauriSink`.
//! - [`credentials`]: OS keychain storage for "remember me" (per tenant).
//! - [`realtime`]: Cumulocity realtime notifications (CometD `/notification/realtime`)
//!   for `/operations/<deviceId>`; feeds `sim::operations`.
//! - [`activity`]: in-memory ring buffer of the last 500 requests + "copy as curl"
//!   (password masked).
//!
//! Not here: payload naming (`payload`), what to send when (`sim`), device
//! fragments (`device`).

pub mod activity;
pub mod api;
pub mod client;
pub mod credentials;
/// In-memory fake Cumulocity server for tests (see `docs/ARCHITECTURE.md` §15).
#[cfg(test)]
pub mod fake;
#[cfg(test)]
mod integration_tests;
pub mod media;
pub mod realtime;
pub mod sink;

pub use activity::ActivityLog;
pub use api::{AlarmStatus, ManagedObjectQuery, id_of};
pub use client::{C8yClient, C8yResponse, is_http_status, now_iso, server_now};
pub use sink::{EventSink, NoopSink, TauriSink};
