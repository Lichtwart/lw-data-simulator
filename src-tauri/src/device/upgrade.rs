//! Library upgrades of a device (PRD §14). Never silent, never destructive to foreign data.
//!
//! The diff compares what was applied (digests in `lw_demo.appliedItems` /
//! `appliedFloorplans` / `appliedDemo`, written by apply / upgrade) with what the bundled
//! library resolves to for the device's current setup. A device applied before digests existed
//! reports every selected item / floorplan as changed and gets no demo-level changes.
//!
//! Applying keeps local overrides: custom slaves and items, generators, snapshot, realistic mode
//! and every slave / register assignment (D6). The demo-pack level fields (`tags`, `includes`,
//! `excludes`, `daliLampCount`, `daliGroups`, `digitalInputs`, `sendIntervalSec`) are merged
//! three-way per field: when the device still has what was applied, the new library value is
//! taken; otherwise the local value is kept and listed as "kept local override".
//!
//! Floorplans: removed / replaced ones are deleted only when the app created them
//! (`lw_demo.floorplanIds`) and nobody edited them in the plugin. A same-name floorplan the app
//! did not create is never touched (the layout is skipped and listed as a conflict); an edited
//! one is kept and the new layout is created next to it as `<name> (v<version>)`.
//!
//! Order of writes (so that a failure part-way never marks the device as up to date):
//! `lw_config*` + `lw_demo` **with the old `libraryVersion`**, then floorplan by floorplan
//! (`lw_demo` persisted after each), and only the final write sets the new `libraryVersion`.

use std::collections::BTreeMap;

use serde_json::Value;

use super::apply::{FloorplanJob, Replace, apply_floorplans};
use super::setup::{self, CustomDedupe, ResolvedSetup};
use super::{crud, demo_state, floorplan};
use crate::c8y::C8yClient;
use crate::error::{AppError, AppResult};
use crate::library::Library;
use crate::model::{
    DaliGroup, DemoDevice, DemoPack, DeviceConfig, DigitalInputEntry, FloorplanLayout,
    FloorplanRef, LwDemoState, UpgradePreview,
};
use crate::payload::config::modbus_slaves_fragment;
use crate::payload::config::{digital_input_entry, digital_inputs_fragment, groups_fragment};

/// Diff of digests: (added, removed, changed), each sorted.
fn diff(
    old: &BTreeMap<String, String>,
    new: &BTreeMap<String, String>,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let added = new
        .keys()
        .filter(|k| !old.contains_key(*k))
        .cloned()
        .collect();
    let removed = old
        .keys()
        .filter(|k| !new.contains_key(*k))
        .cloned()
        .collect();
    let changed = new
        .iter()
        .filter(|(k, v)| old.get(*k).is_some_and(|o| o != *v))
        .map(|(k, _)| k.clone())
        .collect();
    (added, removed, changed)
}

fn floorplan_digests<'a>(
    library: &'a Library,
    demo: &DemoPack,
) -> BTreeMap<String, (&'a str, &'a FloorplanLayout, String)> {
    demo.floorplans
        .iter()
        .filter_map(|p| library.floorplans.get_key_value(p.as_str()))
        .map(|(p, l)| (l.name.clone(), (p.as_str(), l, setup::digest(l))))
        .collect()
}

/// Everything an upgrade changes in the device setup, computed without I/O.
struct UpgradePlan {
    /// The new `lw_demo` (merged, new assignments); `libraryVersion` is still the old one.
    next: LwDemoState,
    resolved: ResolvedSetup,
    /// New `lw_configGroups` / `lw_configDigitalInputs` when the library value is taken.
    groups: Option<Vec<DaliGroup>>,
    inputs: Option<Vec<DigitalInputEntry>>,
    changes: Vec<String>,
    kept: Vec<String>,
    dedupe: CustomDedupe,
}

fn plan_upgrade(
    library: &Library,
    demo: &DemoPack,
    state: &LwDemoState,
    config: &DeviceConfig,
) -> AppResult<UpgradePlan> {
    let mut next = state.clone();
    let mut changes = Vec::new();
    let mut kept = Vec::new();

    // Three-way merge of the demo-pack level fields: applied (base), device (local), library.
    // A device applied by an older app has no base: nothing is merged then.
    let merge = !state.applied_demo.is_empty();
    let cur = setup::current_digests(state, config);
    // Digests of the library values need the new selection (digital inputs read item titles):
    // fields that decide the selection come first.
    let selection_items: Vec<&crate::model::LibraryItem> = library.demo_items(demo);
    let lib_now = setup::demo_digests(demo, &selection_items)?;
    let mut decide = |field: &str| -> bool {
        if !merge {
            return false;
        }
        let applied = state.applied_demo.get(field);
        if applied == lib_now.get(field) {
            return false; // the library did not change this field
        }
        if applied == cur.get(field) {
            changes.push(field.to_owned());
            true
        } else {
            kept.push(field.to_owned());
            false
        }
    };
    if decide("tags") {
        next.tags.clone_from(&demo.tags);
    }
    if decide("includes") {
        next.includes.clone_from(&demo.includes);
    }
    if decide("excludes") {
        next.excludes.clone_from(&demo.excludes);
    }
    if decide("daliLampCount") {
        next.dali_lamp_count = demo.dali_lamp_count;
    }
    if decide("sendIntervalSec") {
        next.send_interval_sec = demo.send_interval_sec;
    }
    let take_groups = decide("daliGroups");
    let take_inputs = decide("digitalInputs");
    let order = |f: &String| setup::DEMO_FIELDS.iter().position(|d| d == f);
    changes.sort_by_key(order);
    kept.sort_by_key(order);

    // A released export: custom entries the library has now.
    let dedupe = setup::drop_library_duplicates(library, &mut next, None);
    let resolved = setup::resolve(library, &next)?;
    resolved.store_assignments(&mut next);
    next.applied_items = setup::item_digests(library, &resolved.items);
    next.applied_demo = setup::demo_digests(demo, &resolved.item_refs())?;

    let groups = take_groups.then(|| demo.dali_groups.clone());
    let inputs = if take_inputs {
        let items = resolved.item_refs();
        Some(
            demo.digital_inputs
                .iter()
                .map(|c| digital_input_entry(c, &items))
                .collect::<AppResult<Vec<_>>>()?,
        )
    } else {
        None
    };
    Ok(UpgradePlan {
        next,
        resolved,
        groups,
        inputs,
        changes,
        kept,
        dedupe,
    })
}

/// Pure upgrade preview of `state` against `library`; `None` when not applicable. Floorplan
/// conflicts and edits need the device and are added by [`check`].
pub fn preview_of(
    library: &Library,
    state: &LwDemoState,
    config: &DeviceConfig,
) -> AppResult<Option<UpgradePreview>> {
    if !setup::upgrade_available(library, Some(state)) {
        return Ok(None);
    }
    let demo = library
        .demo(state.demo_id.as_deref().unwrap_or_default())
        .ok_or_else(|| AppError::NotFound("demo".into()))?;
    let plan = plan_upgrade(library, demo, state, config)?;
    let new_items = setup::item_digests(library, &plan.resolved.items);
    let (mut items_added, items_removed, mut items_changed) =
        diff(&state.applied_items, &new_items);
    if state.applied_items.is_empty() {
        items_changed = new_items.keys().cloned().collect();
        items_added.clear();
    }
    let fps: BTreeMap<String, String> = floorplan_digests(library, demo)
        .into_iter()
        .map(|(n, (_, _, d))| (n, d))
        .collect();
    let (mut floorplans_added, floorplans_removed, mut floorplans_changed) =
        diff(&state.applied_floorplans, &fps);
    if state.applied_floorplans.is_empty() && !state.floorplan_ids.is_empty() {
        floorplans_changed = fps.keys().cloned().collect();
        floorplans_added.clear();
    }
    Ok(Some(UpgradePreview {
        from_version: state.library_version.clone().unwrap_or_default(),
        to_version: library.version.clone(),
        items_added,
        items_removed,
        items_changed,
        floorplans_added,
        floorplans_removed,
        floorplans_changed,
        floorplan_conflicts: Vec::new(),
        floorplans_edited: Vec::new(),
        demo_changes: plan.changes,
        kept_overrides: plan.kept,
        custom_now_in_library: plan.dedupe.now_in_library,
    }))
}

/// Floorplans of the device that the upgrade will not simply replace: same-name ones the app did
/// not create (conflicts) and app-created ones somebody edited (names).
fn floorplan_hazards(
    preview: &UpgradePreview,
    state: &LwDemoState,
    existing: &[Value],
) -> AppResult<(Vec<FloorplanRef>, Vec<String>)> {
    let mut conflicts = Vec::new();
    let mut edited = Vec::new();
    for name in preview
        .floorplans_added
        .iter()
        .chain(&preview.floorplans_changed)
    {
        let mut owned = Vec::new();
        let mut foreign = Vec::new();
        for mo in existing
            .iter()
            .filter(|mo| mo.get("name").and_then(Value::as_str) == Some(name.as_str()))
        {
            let r = floorplan::reference(mo)?;
            if state.floorplan_ids.contains(&r.id) {
                owned.push((r, mo));
            } else {
                foreign.push(r);
            }
        }
        if owned.is_empty() {
            // Nothing of ours to replace: the layout is skipped, the foreign one stays.
            conflicts.extend(foreign);
        } else if owned.iter().any(|(r, mo)| {
            state
                .applied_floorplan_bodies
                .get(&r.id)
                .is_some_and(|d| *d != floorplan::body_digest(mo))
        }) {
            edited.push(name.clone());
        }
    }
    edited.dedup();
    Ok((conflicts, edited))
}

/// `None` when the device is up to date or has no demo.
pub async fn check(
    client: &C8yClient,
    library: &Library,
    device_id: &str,
) -> AppResult<Option<UpgradePreview>> {
    let mo = crud::load_demo_mo(client, device_id).await?;
    let Some(state) = demo_state::read(&mo)? else {
        return Ok(None);
    };
    let config = demo_state::config_of(&mo);
    let Some(mut preview) = preview_of(library, &state, &config)? else {
        return Ok(None);
    };
    let existing = floorplan::list(client, device_id).await?;
    let (conflicts, edited) = floorplan_hazards(&preview, &state, &existing)?;
    preview.floorplan_conflicts = conflicts;
    preview.floorplans_edited = edited;
    Ok(Some(preview))
}

/// Apply the upgrade, keeping local overrides where possible.
/// The caller stops a running session on the device first. A fresh lock of another presenter
/// refuses the upgrade (`DeviceLocked`) unless `force` is set, like an apply does.
pub async fn apply(
    client: &C8yClient,
    library: &Library,
    device_id: &str,
    force: bool,
) -> AppResult<DemoDevice> {
    let mo = crud::load_demo_mo(client, device_id).await?;
    let state = demo_state::read(&mo)?
        .ok_or_else(|| AppError::NotFound(format!("lw_demo on {device_id}")))?;
    demo_state::check_lock(&state, client.username(), force, chrono::Utc::now())?;
    let config = demo_state::config_of(&mo);
    let preview = preview_of(library, &state, &config)?
        .ok_or_else(|| AppError::InvalidInput("device is up to date".into()))?;
    let demo = library
        .demo(state.demo_id.as_deref().unwrap_or_default())
        .ok_or_else(|| AppError::NotFound("demo".into()))?;

    let UpgradePlan {
        next,
        resolved,
        groups,
        inputs,
        ..
    } = plan_upgrade(library, demo, &state, &config)?;
    let lamps_changed = next.dali_lamp_count != state.dali_lamp_count;
    let mut state = next;
    let new_version = library.version.clone();
    let old_version = state.library_version.clone();

    // 1. Config + lw_demo, still marked with the old version.
    let mut body = demo_state::patch_body(&state)?;
    body["lw_configModbusSlaves"] =
        modbus_slaves_fragment(&resolved.slave_refs(), &resolved.item_refs())?;
    if let Some(g) = &groups {
        body["lw_configGroups"] = groups_fragment(g);
    }
    if let Some(i) = &inputs {
        body["lw_configDigitalInputs"] = digital_inputs_fragment(i);
    }
    if lamps_changed {
        body["lw_configCore"] = demo_state::core_with_lamps(&mo, state.dali_lamp_count);
    }
    client.update_managed_object(device_id, &body).await?;

    // 2. Floorplans: delete removed app-created ones, (re)create added / changed.
    let existing = floorplan::list(client, device_id).await?;
    for name in &preview.floorplans_removed {
        for fp in &existing {
            let r = floorplan::reference(fp)?;
            if r.name == *name && state.floorplan_ids.contains(&r.id) {
                // Never delete what somebody edited: it is theirs now.
                let edited = state
                    .applied_floorplan_bodies
                    .get(&r.id)
                    .is_some_and(|d| *d != floorplan::body_digest(fp));
                if !edited {
                    floorplan::remove(client, &r.id).await?;
                }
                state.floorplan_ids.retain(|id| *id != r.id);
                state.applied_floorplan_bodies.remove(&r.id);
            }
        }
        state.applied_floorplans.remove(name);
        demo_state::write(client, device_id, &state).await?;
    }
    let wanted: Vec<&String> = preview
        .floorplans_added
        .iter()
        .chain(&preview.floorplans_changed)
        .collect();
    let digests = floorplan_digests(library, demo);
    let layouts: Vec<(&str, &FloorplanLayout)> = digests
        .iter()
        .filter(|(n, _)| wanted.contains(n))
        .map(|(_, (p, l, _))| (*p, *l))
        .collect();
    if !layouts.is_empty() {
        apply_floorplans(
            client,
            library,
            device_id,
            &resolved,
            groups.as_deref().unwrap_or(&config.groups),
            inputs.as_deref().unwrap_or(&config.digital_inputs),
            &mut state,
            FloorplanJob {
                layouts,
                replace: Replace::OwnedOnly,
            },
        )
        .await
        .inspect_err(|e| {
            log::warn!(
                "upgrade of {device_id} stopped ({e}); the device stays on {old_version:?} and can be upgraded again"
            );
        })?;
    }

    // 3. Everything worked: only now the device counts as upgraded.
    state.library_version = Some(new_version);
    demo_state::write(client, device_id, &state).await?;
    crud::get(client, library, device_id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_diff() {
        let m = |v: &[(&str, &str)]| -> BTreeMap<String, String> {
            v.iter().map(|(a, b)| ((*a).into(), (*b).into())).collect()
        };
        let (a, r, c) = diff(&m(&[("x", "1"), ("y", "1")]), &m(&[("y", "2"), ("z", "1")]));
        assert_eq!(a, vec!["z"]);
        assert_eq!(r, vec!["x"]);
        assert_eq!(c, vec!["y"]);
    }
}
