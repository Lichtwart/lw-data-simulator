//! `lw_demo` fragment I/O, advisory lock, and the device-side config edits (D6).

use std::time::Duration;

use serde_json::{Value, json};

use super::crud;
use super::setup::{self, parse_digital_inputs, parse_groups};
use crate::c8y::{C8yClient, now_iso};
use crate::error::{AppError, AppResult};
use crate::library::{Library, validate};
use crate::model::{
    Control, DaliGroup, DemoSetupPatch, DemoSnapshot, DeviceConfig, DigitalInputEntry,
    DigitalInputType, LibraryItem, LockInfo, LwDemoState,
};
use crate::payload::config::modbus_slaves_fragment;
use crate::payload::config::{
    DI_UNIT_MAX_LEN, config_core_fragment_for, digital_inputs_fragment, groups_fragment,
    truncate_bytes,
};
use crate::sim::scheduler::{
    DEFAULT_SEND_INTERVAL_SEC, MAX_SEND_INTERVAL_SEC, MIN_SEND_INTERVAL_SEC,
};

/// Fragment of the advisory lock. Cumulocity replaces a top-level fragment as a whole, so the
/// lock lives outside `lw_demo`: refreshing it never rewrites (and reverts) the setup.
pub const LOCK_FRAGMENT: &str = "lw_demoLock";
/// Fragment of the saved snapshot (same reason as [`LOCK_FRAGMENT`]).
pub const SNAPSHOT_FRAGMENT: &str = "lw_demoSnapshot";

/// Current `lw_demo.schemaVersion`.
pub const SCHEMA_VERSION: u32 = 1;
/// Locks older than this are ignored.
pub const LOCK_TTL: Duration = Duration::from_secs(10 * 60);
/// A running session rewrites `lw_demo.lock.since` this often (well inside [`LOCK_TTL`]),
/// so a live session never looks abandoned to a second presenter.
pub const LOCK_REFRESH_INTERVAL: Duration = Duration::from_secs(3 * 60);

/// Empty setup (raw device that gets its first setup edit).
pub fn empty_state() -> LwDemoState {
    LwDemoState {
        schema_version: SCHEMA_VERSION,
        demo_id: None,
        library_version: None,
        tags: Vec::new(),
        includes: Vec::new(),
        excludes: Vec::new(),
        dali_lamp_count: 0,
        slaves: Vec::new(),
        custom_slaves: Vec::new(),
        custom_items: Vec::new(),
        custom_scenarios: Vec::new(),
        hidden_scenarios: Vec::new(),
        generators: Default::default(),
        send_interval_sec: DEFAULT_SEND_INTERVAL_SEC,
        realistic_mode: false,
        floorplan_ids: Vec::new(),
        snapshot: None,
        lock: None,
        applied_items: Default::default(),
        applied_floorplans: Default::default(),
        applied_demo: Default::default(),
        applied_floorplan_bodies: Default::default(),
        retired_slaves: Vec::new(),
    }
}

/// `lw_demo` of a managed object (the lock and the snapshot come from their own fragments,
/// [`LOCK_FRAGMENT`] / [`SNAPSHOT_FRAGMENT`]; a copy inside `lw_demo` written by an older app is
/// used when the fragment is absent). `Ok(None)` when absent or `{}`; an error for a newer
/// `schemaVersion` (written by a newer app) or a malformed fragment.
pub fn read(mo: &Value) -> AppResult<Option<LwDemoState>> {
    let Some(v) = mo.get("lw_demo") else {
        return Ok(None);
    };
    if v.is_null() || v.as_object().is_some_and(|o| o.is_empty()) {
        return Ok(None);
    }
    let version = v.get("schemaVersion").and_then(Value::as_u64).unwrap_or(0);
    if version > u64::from(SCHEMA_VERSION) {
        return Err(AppError::InvalidInput(format!(
            "lw_demo schemaVersion {version} is newer than this app ({SCHEMA_VERSION})"
        )));
    }
    let mut state: LwDemoState = serde_json::from_value(v.clone())
        .map_err(|e| AppError::InvalidInput(format!("lw_demo: {e}")))?;
    if let Some(l) = mo.get(LOCK_FRAGMENT) {
        state.lock = serde_json::from_value(l.clone()).ok();
    }
    if let Some(s) = mo.get(SNAPSHOT_FRAGMENT) {
        state.snapshot = serde_json::from_value(s.clone()).ok();
    }
    Ok(Some(state))
}

/// The inventory patch for `state`: `lw_demo` (without lock and snapshot) plus the snapshot
/// fragment. The lock is never part of it (see [`lock_body`]).
pub fn patch_body(state: &LwDemoState) -> AppResult<Value> {
    let mut demo = serde_json::to_value(state)?;
    if let Some(o) = demo.as_object_mut() {
        o.remove("lock");
        o.remove("snapshot");
    }
    Ok(json!({
        "lw_demo": demo,
        SNAPSHOT_FRAGMENT: serde_json::to_value(&state.snapshot)?,
    }))
}

/// The inventory patch that sets (`Some`) or removes (`None`) the lock.
pub fn lock_body(lock: Option<&LockInfo>) -> AppResult<Value> {
    Ok(json!({ LOCK_FRAGMENT: serde_json::to_value(lock)? }))
}

/// The inventory patch that stores (`Some`) or removes (`None`) the snapshot.
pub fn snapshot_body(snapshot: Option<&DemoSnapshot>) -> AppResult<Value> {
    Ok(json!({ SNAPSHOT_FRAGMENT: serde_json::to_value(snapshot)? }))
}

/// Write `lw_demo` (and the snapshot fragment) (`PUT /inventory/managedObjects/{id}`).
pub async fn write(client: &C8yClient, device_id: &str, state: &LwDemoState) -> AppResult<()> {
    client
        .update_managed_object(device_id, &patch_body(state)?)
        .await
        .map(|_| ())
}

/// Take the advisory lock for `user` now; returns it.
pub async fn write_lock(client: &C8yClient, device_id: &str, user: &str) -> AppResult<LockInfo> {
    let lock = new_lock(user);
    client
        .update_managed_object(device_id, &lock_body(Some(&lock))?)
        .await?;
    Ok(lock)
}

/// Remove the lock.
pub async fn clear_lock(client: &C8yClient, device_id: &str) -> AppResult<()> {
    client
        .update_managed_object(device_id, &lock_body(None)?)
        .await
        .map(|_| ())
}

// ───────────────────────────── lock ─────────────────────────────

/// Age of a lock at `now`; `None` if `since` does not parse.
fn lock_age(lock: &LockInfo, now: chrono::DateTime<chrono::Utc>) -> Option<chrono::Duration> {
    chrono::DateTime::parse_from_rfc3339(&lock.since)
        .ok()
        .map(|since| now - since.with_timezone(&chrono::Utc))
}

/// A lock counts while it is younger than [`LOCK_TTL`] (an unparsable `since` is ignored).
pub fn lock_is_fresh(lock: &LockInfo, now: chrono::DateTime<chrono::Utc>) -> bool {
    lock_age(lock, now).is_some_and(|age| {
        age < chrono::Duration::from_std(LOCK_TTL).unwrap_or(chrono::Duration::zero())
    })
}

/// `DeviceLocked` if a fresh lock of another user is present and `force` is off.
pub fn check_lock(
    state: &LwDemoState,
    me: &str,
    force: bool,
    now: chrono::DateTime<chrono::Utc>,
) -> AppResult<()> {
    match &state.lock {
        Some(l) if !force && l.user != me && lock_is_fresh(l, now) => Err(AppError::DeviceLocked {
            user: l.user.clone(),
            since: l.since.clone(),
        }),
        _ => Ok(()),
    }
}

/// The fresh lock of **another** user on a managed object (`lw_demoLock`, else a copy inside
/// `lw_demo` from an older app), if any.
pub fn foreign_lock(mo: &Value, me: &str, now: chrono::DateTime<chrono::Utc>) -> Option<LockInfo> {
    let raw = match mo.get(LOCK_FRAGMENT) {
        Some(v) => v,
        None => mo.pointer("/lw_demo/lock")?,
    };
    let lock: LockInfo = serde_json::from_value(raw.clone()).ok()?;
    (lock.user != me && lock_is_fresh(&lock, now)).then_some(lock)
}

/// A lock held by `user` from now.
pub fn new_lock(user: &str) -> LockInfo {
    LockInfo {
        user: user.to_owned(),
        since: now_iso(),
    }
}

// ───────────────────────────── setup patch ─────────────────────────────

fn check_select_controls(items: &[LibraryItem]) -> AppResult<()> {
    for item in items {
        if let Some(Control::Select { options, default }) = item.control() {
            for (i, o) in options.iter().enumerate() {
                if options[..i].iter().any(|p| p.value == o.value) {
                    return Err(AppError::InvalidInput(format!(
                        "{}: select option value {} is used twice",
                        item.id(),
                        o.value
                    )));
                }
            }
            if !options.iter().any(|o| o.value == *default) {
                return Err(AppError::InvalidInput(format!(
                    "{}: select default {default} is not an option value",
                    item.id()
                )));
            }
        }
    }
    Ok(())
}

/// Apply `patch` to `state` (pure, validated). `generators` replaces the whole map.
pub fn apply_patch(
    library: &Library,
    state: &mut LwDemoState,
    patch: &DemoSetupPatch,
) -> AppResult<()> {
    if let Some(v) = &patch.tags {
        state.tags = v.clone();
    }
    if let Some(v) = &patch.includes {
        state.includes = v.clone();
    }
    if let Some(v) = &patch.excludes {
        state.excludes = v.clone();
    }
    if let Some(n) = patch.dali_lamp_count {
        if n > 64 {
            return Err(AppError::InvalidInput(format!("daliLampCount {n} (0–64)")));
        }
        state.dali_lamp_count = n;
    }
    // What was custom before this patch (see `drop_library_duplicates`).
    let known: std::collections::BTreeSet<String> = state
        .custom_items
        .iter()
        .map(|i| i.id().to_owned())
        .chain(state.custom_slaves.iter().map(|s| s.key.clone()))
        .collect();
    if let Some(v) = &patch.custom_slaves {
        state.custom_slaves = v.clone();
    }
    if let Some(v) = &patch.custom_items {
        state.custom_items = v.clone();
    }
    if let Some(v) = &patch.generators {
        for (id, g) in v {
            if let Some(p) = validate::generator_problem(g) {
                return Err(AppError::InvalidInput(format!("generator of {id}: {p}")));
            }
        }
        state.generators = v.clone();
    }
    if let Some(s) = patch.send_interval_sec {
        if !(MIN_SEND_INTERVAL_SEC..=MAX_SEND_INTERVAL_SEC).contains(&s) {
            return Err(AppError::InvalidInput(format!(
                "sendIntervalSec {s} ({MIN_SEND_INTERVAL_SEC}–{MAX_SEND_INTERVAL_SEC})"
            )));
        }
        state.send_interval_sec = s;
    }
    if let Some(r) = patch.realistic_mode {
        state.realistic_mode = r;
    }
    // A pack exported from this device and released: the library has the entries now.
    for w in setup::drop_library_duplicates(library, state, Some(&known)).warnings {
        log::warn!("setup: {w}");
    }
    let findings = validate::validate_custom(library, &state.custom_slaves, &state.custom_items);
    if !findings.is_empty() {
        let msg: Vec<String> = findings.iter().map(ToString::to_string).collect();
        return Err(AppError::InvalidInput(msg.join("; ")));
    }
    check_select_controls(&state.custom_items)?;
    // New slaves / registers get addresses; existing ones are kept.
    let items = setup::select_items(library, state);
    let (slaves, retired) = setup::assign_all(&state.slaves, &state.retired_slaves, &items)?;
    state.slaves = slaves;
    state.retired_slaves = retired;
    // Scenarios are checked when they are edited (a later selection change must not block
    // other edits; a step on an item that is gone fails when the scenario is applied).
    if let Some(v) = &patch.hidden_scenarios {
        state.hidden_scenarios = v.clone();
    }
    if let Some(v) = &patch.custom_scenarios {
        state.custom_scenarios = v.clone();
        let refs: Vec<&LibraryItem> = items.iter().collect();
        let lamps = crate::sim::faults::lamp_count(state.dali_lamp_count, &refs);
        let findings = validate::validate_device_scenarios(
            library,
            &state.custom_slaves,
            &items,
            lamps,
            &state.custom_scenarios,
            &setup::effective_scenarios(library, state),
        );
        if !findings.is_empty() {
            let msg: Vec<String> = findings.iter().map(ToString::to_string).collect();
            return Err(AppError::InvalidInput(msg.join("; ")));
        }
    }
    Ok(())
}

/// Apply a setup patch and persist it; returns the new state. Custom slaves / items (D6) get
/// ids, addresses and register addresses via `payload::naming::assign_slaves` /
/// `assign_registers` (existing ones are never renumbered); `lw_configModbusSlaves` is
/// rewritten to match.
pub async fn patch_setup(
    client: &C8yClient,
    library: &Library,
    device_id: &str,
    patch: &DemoSetupPatch,
) -> AppResult<LwDemoState> {
    // Only demo devices: the next `lw_fetch*` would push this config to real hardware.
    let mo = crud::load_demo_mo(client, device_id).await?;
    let mut state = read(&mo)?.unwrap_or_else(empty_state);
    let lamps_before = state.dali_lamp_count;
    apply_patch(library, &mut state, patch)?;
    let resolved = setup::resolve(library, &state)?;
    let slaves = modbus_slaves_fragment(&resolved.slave_refs(), &resolved.item_refs())?;
    let mut body = patch_body(&state)?;
    body["lw_configModbusSlaves"] = slaves;
    if state.dali_lamp_count != lamps_before {
        body["lw_configCore"] = core_with_lamps(&mo, state.dali_lamp_count);
    }
    client.update_managed_object(device_id, &body).await?;
    Ok(state)
}

/// The device's `lw_configCore` with `maxControlGears = max(55, lamps)` (other fields kept; the
/// full 13-field default when the device has none).
pub fn core_with_lamps(mo: &Value, lamps: u32) -> Value {
    let fresh = config_core_fragment_for(lamps);
    match mo.get("lw_configCore") {
        Some(Value::Object(o)) => {
            let mut o = o.clone();
            o.insert("maxControlGears".into(), fresh["maxControlGears"].clone());
            Value::Object(o)
        }
        _ => fresh,
    }
}

// ───────────────────────────── device config (D6) ─────────────────────────────

/// Read `lw_configGroups` and `lw_configDigitalInputs` from the device (D6).
pub async fn get_config(client: &C8yClient, device_id: &str) -> AppResult<DeviceConfig> {
    let mo = crud::load_demo_mo(client, device_id).await?;
    Ok(config_of(&mo))
}

/// Groups + digital inputs of a managed object.
pub fn config_of(mo: &Value) -> DeviceConfig {
    DeviceConfig {
        groups: parse_groups(mo.get("lw_configGroups")),
        digital_inputs: parse_digital_inputs(mo.get("lw_configDigitalInputs")),
    }
}

/// Validate DALI groups: ≤ 16, ids 1–16 unique, lamps 1–64, levels 0–100 with min ≤ max.
pub fn check_groups(groups: &[DaliGroup]) -> AppResult<()> {
    let bad = |m: String| Err(AppError::InvalidInput(m));
    if groups.len() > 16 {
        return bad(format!("{} groups, max 16", groups.len()));
    }
    for (i, g) in groups.iter().enumerate() {
        if !(1..=16).contains(&g.id) {
            return bad(format!("group id {} (1–16)", g.id));
        }
        if groups[..i].iter().any(|o| o.id == g.id) {
            return bad(format!("group id {} is used twice", g.id));
        }
        if g.name.trim().is_empty() {
            return bad(format!("group {} has no name", g.id));
        }
        if let Some(l) = g.converters.iter().find(|l| !(1..=64).contains(*l)) {
            return bad(format!("group {}: lamp {l} (1–64)", g.id));
        }
        if g.max_level > 100 || g.min_level > g.max_level {
            return bad(format!(
                "group {}: levels {}–{} (0 ≤ min ≤ max ≤ 100)",
                g.id, g.min_level, g.max_level
            ));
        }
        if g.lux_threshold.is_some_and(|l| l > 200_000) {
            return bad(format!("group {}: luxThreshold (0–200000)", g.id));
        }
        if g.slave_of
            .is_some_and(|m| m == g.id || !(1..=16).contains(&m))
        {
            return bad(format!("group {}: slaveOf", g.id));
        }
    }
    Ok(())
}

/// Replace `lw_configGroups` (≤ 16 groups, lamps 1–64) and return what was written (D6).
pub async fn set_groups(
    client: &C8yClient,
    device_id: &str,
    groups: &[DaliGroup],
) -> AppResult<Vec<DaliGroup>> {
    check_groups(groups)?;
    crud::load_demo_mo(client, device_id).await?;
    let frag = groups_fragment(groups);
    client
        .update_managed_object(device_id, &json!({ "lw_configGroups": frag }))
        .await?;
    Ok(parse_groups(Some(&frag)))
}

/// Validate (and normalise) digital inputs: ≤ 10, index 1–10 unique, unit cut to 19 chars.
pub fn normalize_digital_inputs(inputs: &[DigitalInputEntry]) -> AppResult<Vec<DigitalInputEntry>> {
    if inputs.len() > 10 {
        return Err(AppError::InvalidInput(format!(
            "{} digital inputs, max 10",
            inputs.len()
        )));
    }
    let mut out: Vec<DigitalInputEntry> = Vec::with_capacity(inputs.len());
    for e in inputs {
        if !(1..=10).contains(&e.index) {
            return Err(AppError::InvalidInput(format!(
                "digital input {} (1–10)",
                e.index
            )));
        }
        if out.iter().any(|o| o.index == e.index) {
            return Err(AppError::InvalidInput(format!(
                "digital input {} is used twice",
                e.index
            )));
        }
        let mut e = e.clone();
        if let Some(u) = &e.settings.unit {
            e.settings.unit = Some(truncate_bytes(u, DI_UNIT_MAX_LEN).to_owned());
        }
        for a in [
            &mut e.settings.high_level_action,
            &mut e.settings.low_level_action,
        ]
        .into_iter()
        .flatten()
        {
            let lower = a.trim().to_ascii_lowercase();
            if !matches!(lower.as_str(), "on" | "off" | "auto") {
                return Err(AppError::InvalidInput(format!(
                    "digital input {}: action \"{a}\" (on | off | auto)",
                    e.index
                )));
            }
            *a = lower;
        }
        if e.settings.input_type == DigitalInputType::ImpulseCounter
            && e.settings.increment == Some(0)
        {
            return Err(AppError::InvalidInput(format!(
                "digital input {}: increment 1–65535",
                e.index
            )));
        }
        if let Some(g) = e.settings.groups.iter().find(|g| !(1..=16).contains(*g)) {
            return Err(AppError::InvalidInput(format!(
                "digital input {}: group {g} (1–16)",
                e.index
            )));
        }
        out.push(e);
    }
    Ok(out)
}

/// Replace `lw_configDigitalInputs` (≤ 10 inputs, titles included, see
/// `payload::config::digital_inputs_fragment`) and return what was written (D6).
pub async fn set_digital_inputs(
    client: &C8yClient,
    device_id: &str,
    inputs: &[DigitalInputEntry],
) -> AppResult<Vec<DigitalInputEntry>> {
    let inputs = normalize_digital_inputs(inputs)?;
    crud::load_demo_mo(client, device_id).await?;
    let frag = digital_inputs_fragment(&inputs);
    client
        .update_managed_object(device_id, &json!({ "lw_configDigitalInputs": frag }))
        .await?;
    Ok(parse_digital_inputs(Some(&frag)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(s: &str) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339(s)
            .unwrap()
            .with_timezone(&chrono::Utc)
    }

    #[test]
    fn lock_ttl_is_ten_minutes() {
        let mut st = empty_state();
        st.lock = Some(LockInfo {
            user: "other".into(),
            since: "2026-10-12T12:00:00Z".into(),
        });
        let now = at("2026-10-12T12:09:00Z");
        assert!(matches!(
            check_lock(&st, "me", false, now),
            Err(AppError::DeviceLocked { .. })
        ));
        assert!(check_lock(&st, "me", true, now).is_ok());
        assert!(check_lock(&st, "other", false, now).is_ok());
        assert!(check_lock(&st, "me", false, at("2026-10-12T12:10:01Z")).is_ok());
        let _ = chrono::Utc.timestamp_opt(0, 0);
    }

    #[test]
    fn read_rejects_newer_schema_and_treats_empty_as_none() {
        assert!(read(&json!({})).unwrap().is_none());
        assert!(read(&json!({"lw_demo": {}})).unwrap().is_none());
        assert!(read(&json!({"lw_demo": {"schemaVersion": 2, "sendIntervalSec": 30}})).is_err());
        let st = read(&json!({"lw_demo": {"schemaVersion": 1, "sendIntervalSec": 30}}))
            .unwrap()
            .unwrap();
        assert_eq!(st.send_interval_sec, 30);
    }

    #[test]
    fn groups_are_validated() {
        let g = |id, min, max, conv: Vec<u32>| DaliGroup {
            id,
            name: "G".into(),
            converters: conv,
            min_level: min,
            max_level: max,
            lux_threshold: None,
            slave_of: None,
            dynamic_control: false,
            automatic_switch: false,
        };
        assert!(check_groups(&[g(1, 0, 100, vec![1, 64])]).is_ok());
        assert!(check_groups(&[g(0, 0, 100, vec![])]).is_err());
        assert!(check_groups(&[g(1, 0, 100, vec![65])]).is_err());
        assert!(check_groups(&[g(1, 50, 40, vec![])]).is_err());
        assert!(check_groups(&[g(1, 0, 100, vec![]), g(1, 0, 100, vec![])]).is_err());
    }

    #[test]
    fn generators_patch_replaces_map() {
        let lib = Library::default();
        let mut st = empty_state();
        st.generators.insert(
            "a".into(),
            crate::model::GeneratorConfig::Counter { rate_per_hour: 1.0 },
        );
        let mut map = std::collections::BTreeMap::new();
        map.insert(
            "b".into(),
            crate::model::GeneratorConfig::Counter { rate_per_hour: 2.0 },
        );
        apply_patch(
            &lib,
            &mut st,
            &DemoSetupPatch {
                generators: Some(map),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(st.generators.keys().collect::<Vec<_>>(), vec!["b"]);
        assert!(
            apply_patch(
                &lib,
                &mut st,
                &DemoSetupPatch {
                    send_interval_sec: Some(900),
                    ..Default::default()
                }
            )
            .is_err()
        );
    }
}
