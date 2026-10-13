//! Demo device types (PRD §6, §12–§14).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use specta::Type;

use super::library::{
    DaliGroup, DigitalInputEntry, GeneratorConfig, LibraryItem, ModbusRegisterType, SlaveDefinition,
};

/// `c8y_Position`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Position {
    #[specta(type = specta_typescript::Number)]
    pub lat: f64,
    #[specta(type = specta_typescript::Number)]
    pub lng: f64,
    #[serde(default)]
    #[specta(type = specta_typescript::Number)]
    pub alt: f64,
}

/// Reference to a `c8y_DeviceGroup`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GroupRef {
    pub id: String,
    pub name: String,
}

/// Reference to an `lw_floorplan` managed object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FloorplanRef {
    pub id: String,
    pub name: String,
}

/// Advisory lock (PRD §6.3), stored in its own `lw_demoLock` fragment (older apps wrote it into
/// `lw_demo.lock`; both are read). Ignored after 10 min.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LockInfo {
    pub user: String,
    /// ISO-8601.
    pub since: String,
}

/// Address assigned to one Modbus register (item). Persisted so upgrades never renumber.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RegisterAssignment {
    /// Item id (== register name).
    pub item_id: String,
    pub register_type: ModbusRegisterType,
    /// Raw register address 0–65535 (`registers[].address`, `lw_modbusWrite.registerAddress`).
    pub address: u32,
}

/// Modbus slave assignment (IDs count down from 20, addresses from 240) plus the register
/// addresses assigned within it. Stored in `lw_demo.slaves`; existing entries are never
/// renumbered (`payload::naming::assign_slaves` / `assign_registers`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SlaveAssignment {
    /// Slave key (library [`SlaveDefinition::key`] or a custom slave).
    pub key: String,
    /// Slave id 1–20.
    pub id: u32,
    /// Modbus slave address 1–247.
    pub address: u32,
    #[serde(default)]
    pub registers: Vec<RegisterAssignment>,
}

/// Saved live values (written only on "Save snapshot").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DemoSnapshot {
    pub saved_at: String,
    #[specta(type = BTreeMap<String, specta_typescript::Number>)]
    pub values: BTreeMap<String, f64>,
}

/// The `lw_demo` fragment on a demo device (PRD §6.3). All setup lives here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LwDemoState {
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub demo_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library_version: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub includes: Vec<String>,
    #[serde(default)]
    pub excludes: Vec<String>,
    #[serde(default)]
    pub dali_lamp_count: u32,
    /// Assigned slave ids / addresses and register addresses (persisted, never renumbered).
    #[serde(default)]
    pub slaves: Vec<SlaveAssignment>,
    /// Assignments of slaves / registers that are not selected right now (an excluded item).
    /// Kept so that ids / addresses are never reused or renumbered (D6); a slave entry holds
    /// only its retired registers unless the whole slave is retired.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub retired_slaves: Vec<SlaveAssignment>,
    /// Slaves created on this device (demo editor, D6). Merged with the library's slaves.
    #[serde(default)]
    pub custom_slaves: Vec<SlaveDefinition>,
    /// Items created on this device (demo editor, "generate series" helper, D6).
    /// Merged with the library's items; ids must not collide with library ids.
    #[serde(default)]
    pub custom_items: Vec<LibraryItem>,
    /// DALI groups of the device (D6, D10: kept here, no `lw_configGroups`). `None` on devices
    /// set up by an older app version: the library demo's groups apply until the next write.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dali_groups: Option<Vec<DaliGroup>>,
    /// Digital inputs of the device incl. titles (D6, D10: kept here, no
    /// `lw_configDigitalInputs`). `None` like `daliGroups`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digital_inputs: Option<Vec<DigitalInputEntry>>,
    /// IANA time zone of the device (`dailyProfile` runs in device time); `None` =
    /// `Europe/Berlin`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_zone: Option<String>,
    #[serde(default)]
    pub generators: BTreeMap<String, GeneratorConfig>,
    pub send_interval_sec: u32,
    #[serde(default)]
    pub realistic_mode: bool,
    #[serde(default)]
    pub floorplan_ids: Vec<String>,
    /// Stored in its own `lw_demoSnapshot` fragment on the device (read into this field).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<DemoSnapshot>,
    /// Stored in its own `lw_demoLock` fragment on the device (read into this field).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lock: Option<LockInfo>,
    /// Digest per library item id selected at the last apply / upgrade (upgrade diff, PRD §14).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub applied_items: BTreeMap<String, String>,
    /// Digest per floorplan name applied at the last apply / upgrade (upgrade diff, PRD §14).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub applied_floorplans: BTreeMap<String, String>,
    /// Digest of every demo-pack level field at the last apply / upgrade (`tags`, `includes`,
    /// `excludes`, `daliLampCount`, `daliGroups`, `digitalInputs`, `sendIntervalSec`): the base of
    /// the three-way merge of an upgrade (PRD §14). Empty on devices applied before this existed.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub applied_demo: BTreeMap<String, String>,
    /// Digest of the `lw_floorplan` body of each app-created floorplan (id → digest), to tell
    /// whether someone edited it in the plugin.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub applied_floorplan_bodies: BTreeMap<String, String>,
}

/// One row in the device list.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DemoDeviceSummary {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    /// First parent `c8y_DeviceGroup`, if any (used for grouping).
    pub parent_group: Option<GroupRef>,
    pub demo_id: Option<String>,
    pub library_version: Option<String>,
    pub lock: Option<LockInfo>,
    pub last_updated: Option<String>,
    /// Same rule as `DemoDevice::upgrade_available`.
    pub upgrade_available: bool,
}

/// Full demo device, as shown in the workspace.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DemoDevice {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub position: Option<Position>,
    pub parent_group: Option<GroupRef>,
    pub serial_number: Option<String>,
    pub firmware_version: Option<String>,
    /// `None` when the device has no `lw_demo` yet (raw use only).
    pub demo: Option<LwDemoState>,
    /// `lw_floorplan`s linked to the device as child additions (app-created ones are listed in
    /// `demo.floorplanIds`).
    pub floorplans: Vec<FloorplanRef>,
    /// `lw_demo.libraryVersion` is older than the bundled library.
    pub upgrade_available: bool,
}

/// Arguments of `create_demo_device`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CreateDeviceRequest {
    pub name: String,
    pub description: Option<String>,
    pub position: Option<Position>,
    pub parent_group_id: Option<String>,
    /// Apply this demo pack right after creation.
    pub apply_demo: Option<ApplyDemoRequest>,
}

/// Arguments of `update_demo_device`. `None` = unchanged.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateDeviceRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub position: Option<Position>,
}

/// Arguments of `reset_demo_device`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ResetDeviceRequest {
    /// Also delete floorplans the app created (`lw_demo.floorplanIds`).
    pub delete_floorplans: bool,
}

/// Arguments of `apply_demo` / `preview_apply_demo`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ApplyDemoRequest {
    pub demo_id: String,
    /// Upload + link the demo's floorplans.
    pub apply_floorplans: bool,
    /// Replace existing floorplans with the same name (user confirmed).
    pub replace_existing_floorplans: bool,
    /// Take over a fresh advisory lock held by another user (else `deviceLocked`).
    #[serde(default)]
    pub force: bool,
}

/// What `apply_demo` would do (for the confirmation dialog).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ApplyDemoPreview {
    pub demo_id: String,
    pub item_count: u32,
    pub slaves: Vec<SlaveAssignment>,
    /// Existing floorplans that would be replaced.
    pub floorplan_conflicts: Vec<FloorplanRef>,
    pub warnings: Vec<String>,
}

/// Result of `apply_demo`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ApplyDemoResult {
    pub device: DemoDevice,
    pub floorplans_created: Vec<FloorplanRef>,
    pub floorplans_replaced: Vec<FloorplanRef>,
    pub warnings: Vec<String>,
}

/// Result of `create_demo_device`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CreateDeviceResult {
    pub device: DemoDevice,
    /// Warnings of the demo pack that was applied right away (empty without a pack).
    pub warnings: Vec<String>,
}

/// Partial update of the `lw_demo` setup. `None` = unchanged.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DemoSetupPatch {
    pub tags: Option<Vec<String>>,
    pub includes: Option<Vec<String>>,
    pub excludes: Option<Vec<String>>,
    pub dali_lamp_count: Option<u32>,
    /// Replaces the custom slaves (D6). New slaves get ids / addresses on write.
    pub custom_slaves: Option<Vec<SlaveDefinition>>,
    /// Replaces the custom items (D6). New Modbus items get register addresses on write.
    pub custom_items: Option<Vec<LibraryItem>>,
    /// Replaces the whole override map (an item missing from it falls back to its library
    /// generator).
    pub generators: Option<BTreeMap<String, GeneratorConfig>>,
    pub send_interval_sec: Option<u32>,
    pub realistic_mode: Option<bool>,
}

/// The device's DALI groups and digital inputs (`lw_demo.daliGroups` / `lw_demo.digitalInputs`,
/// D10; the library demo's values when the device has none yet). Returned by
/// `get_device_config`; written by `set_dali_groups` / `set_digital_inputs`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DeviceConfig {
    pub groups: Vec<DaliGroup>,
    pub digital_inputs: Vec<DigitalInputEntry>,
}

/// Arguments of `generate_item_series` ("generate series" helper, PRD §11.3).
/// Clones `template` `count` times, `n` starting at `start`: ids `<template id>-<n>` for native
/// kinds (`light-5`) and `<template id>_<n>` otherwise (≤ 19 chars), titles `<title> <n>`; a
/// native `index` is `n` (so `light-5` is lamp 5).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GenerateSeriesRequest {
    pub template: LibraryItem,
    pub count: u32,
    pub start: u32,
}

/// Arguments of `export_demo_pack`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportRequest {
    pub device_id: String,
    /// Demo id for the exported pack (`[a-z0-9-]`).
    pub demo_id: String,
    /// Target folder; the export writes a `library/`-shaped tree inside it.
    pub target_dir: String,
}

/// Result of `export_demo_pack`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub path: String,
    pub files_written: Vec<String>,
    pub warnings: Vec<String>,
}

/// Diff between a device's demo and the bundled library (PRD §14).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpgradePreview {
    pub from_version: String,
    pub to_version: String,
    pub items_added: Vec<String>,
    pub items_removed: Vec<String>,
    pub items_changed: Vec<String>,
    pub floorplans_added: Vec<String>,
    pub floorplans_removed: Vec<String>,
    pub floorplans_changed: Vec<String>,
    /// Floorplans of the device with the same name as an added / changed layout that the app did
    /// not create. The upgrade never touches them: the layout is skipped.
    pub floorplan_conflicts: Vec<FloorplanRef>,
    /// Names of app-created floorplans somebody edited on the device. The upgrade keeps them and
    /// creates the new layout next to them as `<name> (v<version>)`.
    pub floorplans_edited: Vec<String>,
    /// Demo-pack level fields the upgrade takes from the new library (the device still had the
    /// value that was applied): `tags`, `includes`, `excludes`, `daliLampCount`, `daliGroups`,
    /// `digitalInputs`, `sendIntervalSec`.
    pub demo_changes: Vec<String>,
    /// Fields whose library value changed but that were changed on the device: kept as they are.
    pub kept_overrides: Vec<String>,
    /// Custom slaves / items of the device (ids) that exist in the library now (an exported
    /// pack was released). The library version is used from now on.
    pub custom_now_in_library: Vec<String>,
}
