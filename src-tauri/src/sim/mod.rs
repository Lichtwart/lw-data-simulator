//! Simulation engine (PRD §7–§9). One active session at a time.
//!
//! A session is a tokio task (spawned with `tauri::async_runtime::spawn`) that owns
//! all live state of the simulated device. The rest of the app talks to it only
//! through [`SessionHandle`] (an mpsc command channel + a `watch` snapshot), so
//! there is no shared mutable state and no locking of live values.
//!
//! Ownership:
//! - [`session`]: `SessionHandle`, the task loop, lifecycle (lock, stale operations,
//!   start scenario, stop), emits `session-changed` / `item-values-changed` /
//!   `fault-changed` / `session-error`.
//! - [`outbound`]: the outbound worker — the only place a session sends HTTP; the state loop
//!   enqueues, never awaits Cumulocity (a normal lane in order, a priority lane that sends
//!   concurrently: EXECUTING statuses and the lock).
//! - [`generators`]: `randomWalk`, `sine`, `dailyProfile`, `counter` (pure).
//! - [`scheduler`]: periodic send every `sendIntervalSec`, immediate send on change (manual
//!   changes coalesced), batching via `payload::measurement`.
//! - [`operations`]: realtime operation handling — auto-answer config ops, prompt for
//!   visible ops (60 s timeout, auto-accept), apply effects; emits `operation-*`.
//! - [`faults`]: raise / clear faults (DALI, slave timeout, slave alarms, device-level custom alarms),
//!   realistic mode (no cloud clear).
//! - [`scenario`]: apply a scenario's steps.
//! - [`core`]: the live state and every mutation (no I/O), driven by the loop in [`session`].
//! - [`emit`]: `SessionEmitter` (Tauri events in the app, a recorder in tests).

pub mod core;
pub mod emit;
pub mod faults;
pub mod generators;
pub mod operations;
pub mod outbound;
pub mod scenario;
pub mod scheduler;
pub mod session;

#[cfg(test)]
mod tests;

pub use session::{SessionCommand, SessionHandle, SessionSender};
