//! Demo devices in Cumulocity (PRD §6, §12–§14).
//!
//! Ownership:
//! - [`crud`]: list (only `c8y_DeviceTypes` ∋ `demo`, grouped by parent
//!   `c8y_DeviceGroup`), create (§6.1 skeleton), edit, reset, delete.
//! - [`demo_state`]: read / write `lw_demo` (setup on change, values on snapshot),
//!   advisory lock (10 min TTL), DALI group / digital input edits in `lw_demo` (D6, D10).
//! - [`setup`]: resolve a setup (library + custom items / slaves), slave / register assignment,
//!   the device's groups / inputs, digests, version compare.
//! - [`apply`]: apply a demo pack (slave assignment, `lw_demo`, removal of old `lw_config*`
//!   fragments, floorplans).
//! - [`floorplan`]: upload image binary, create `lw_floorplan`, link as child
//!   addition, delete app-created floorplans.
//! - [`export`]: device → `library/`-shaped folder (§13).
//! - [`upgrade`]: diff against the bundled library + apply (§14).
//!
//! All functions take a `&C8yClient` and are called from `commands`; they
//! never touch the running session (commands stop/notify the session first).

pub mod apply;
pub mod crud;
pub mod demo_state;
pub mod export;
pub mod floorplan;
pub mod setup;
pub mod upgrade;

#[cfg(test)]
mod tests;
