//! Apply a demo pack to a device (PRD §11, §12, D6).
//!
//! 1. Resolve the selection (`tags ∪ includes − excludes` + the device's custom items) and
//!    assign slaves / registers, keeping every existing assignment.
//! 2. Write `lw_configGroups`, `lw_configDigitalInputs`, `lw_configModbusSlaves` and `lw_demo`.
//!    A fresh advisory lock of another presenter stops an apply (unless `force`).
//! 3. Optionally upload the floorplans (binary → `lw_floorplan` → child addition). A floorplan
//!    with the same name is replaced only when the request confirms it; otherwise it is skipped
//!    with a warning. Created ids are recorded in `lw_demo.floorplanIds` **after each
//!    floorplan**, so a failure part-way leaves nothing behind that Reset / Delete cannot find.

use std::path::Path;

use serde_json::Value;

use super::setup::{self, ResolvedSetup};
use super::{crud, demo_state, floorplan};
use crate::c8y::C8yClient;
use crate::error::{AppError, AppResult};
use crate::library::{Library, validate};
use crate::model::{
    ApplyDemoPreview, ApplyDemoRequest, ApplyDemoResult, CreateDeviceRequest, CreateDeviceResult,
    DaliGroup, DemoPack, DigitalInputEntry, FloorplanLayout, FloorplanRef, LwDemoState,
};
use crate::payload::config::{config_fragments, digital_input_entry};
use crate::payload::floorplan::FloorplanContext;

/// Everything an apply writes, computed without I/O.
#[derive(Debug, Clone)]
pub struct ApplyPlan {
    pub state: LwDemoState,
    pub resolved: ResolvedSetup,
    pub inputs: Vec<DigitalInputEntry>,
    /// `lw_configGroups` / `lw_configDigitalInputs` / `lw_configModbusSlaves` patch.
    pub config: Value,
    pub warnings: Vec<String>,
}

/// The new `lw_demo` for applying `demo` over `existing` (device-local edits survive: custom
/// slaves / items, realistic mode, floorplan ids, slave assignments; generators, snapshot and
/// scenario edits only when the same demo is re-applied).
pub fn new_state(
    library: &Library,
    demo: &DemoPack,
    existing: Option<&LwDemoState>,
) -> LwDemoState {
    let mut st = existing.cloned().unwrap_or_else(demo_state::empty_state);
    let same_demo = st.demo_id.as_deref() == Some(demo.id.as_str());
    st.schema_version = demo_state::SCHEMA_VERSION;
    st.demo_id = Some(demo.id.clone());
    st.library_version = Some(library.version.clone());
    st.tags = demo.tags.clone();
    st.includes = demo.includes.clone();
    st.excludes = demo.excludes.clone();
    st.dali_lamp_count = demo.dali_lamp_count;
    st.send_interval_sec = demo.send_interval_sec;
    if !same_demo {
        st.generators.clear();
        st.snapshot = None;
        // Scenario edits belong to the demo they were made for.
        st.custom_scenarios.clear();
        st.hidden_scenarios.clear();
    }
    st
}

/// Compute an apply (pure).
pub fn plan(
    library: &Library,
    demo: &DemoPack,
    existing: Option<&LwDemoState>,
) -> AppResult<ApplyPlan> {
    let mut state = new_state(library, demo, existing);
    // Custom entries that a released export put into the library are dropped (library wins).
    let mut warnings = setup::drop_library_duplicates(library, &mut state, None).warnings;
    warnings.extend(
        validate::validate_custom(library, &state.custom_slaves, &state.custom_items)
            .iter()
            .map(ToString::to_string),
    );
    let resolved = setup::resolve(library, &state)?;
    resolved.store_assignments(&mut state);
    state.applied_items = setup::item_digests(library, &resolved.items);
    let items = resolved.item_refs();
    state.applied_demo = setup::demo_digests(demo, &items)?;
    let config = config_fragments(demo, &items, &resolved.slave_refs())?;
    let inputs = demo
        .digital_inputs
        .iter()
        .map(|c| digital_input_entry(c, &items))
        .collect::<AppResult<Vec<_>>>()?;
    for path in &demo.floorplans {
        if library.floorplan(path).is_none() {
            warnings.push(format!("floorplan {path} is missing in the library"));
        }
    }
    Ok(ApplyPlan {
        state,
        resolved,
        inputs,
        config,
        warnings,
    })
}

/// Name → existing floorplan of the device.
fn conflicts(existing: &[Value], layouts: &[&FloorplanLayout]) -> AppResult<Vec<FloorplanRef>> {
    let mut out = Vec::new();
    for mo in existing {
        let r = floorplan::reference(mo)?;
        if layouts.iter().any(|l| l.name == r.name) {
            out.push(r);
        }
    }
    Ok(out)
}

fn layouts<'a>(library: &'a Library, demo: &DemoPack) -> Vec<(&'a str, &'a FloorplanLayout)> {
    demo.floorplans
        .iter()
        .filter_map(|p| library.floorplans.get_key_value(p.as_str()))
        .map(|(p, l)| (p.as_str(), l))
        .collect()
}

fn demo_of<'a>(library: &'a Library, id: &str) -> AppResult<&'a DemoPack> {
    library
        .demo(id)
        .ok_or_else(|| AppError::NotFound(format!("demo {id}")))
}

pub async fn preview(
    client: &C8yClient,
    library: &Library,
    device_id: &str,
    req: &ApplyDemoRequest,
) -> AppResult<ApplyDemoPreview> {
    let demo = demo_of(library, &req.demo_id)?;
    let mo = crud::load_demo_mo(client, device_id).await?;
    let existing = demo_state::read(&mo)?;
    let mut warnings = Vec::new();
    let (item_count, slaves) = match plan(library, demo, existing.as_ref()) {
        Ok(p) => {
            warnings.extend(p.warnings);
            (p.resolved.items.len() as u32, p.state.slaves)
        }
        Err(e) => {
            warnings.push(e.to_string());
            (0, Vec::new())
        }
    };
    let floorplan_conflicts = if req.apply_floorplans {
        let existing_fps = floorplan::list(client, device_id).await?;
        let ls: Vec<&FloorplanLayout> =
            layouts(library, demo).into_iter().map(|(_, l)| l).collect();
        conflicts(&existing_fps, &ls)?
    } else {
        Vec::new()
    };
    Ok(ApplyDemoPreview {
        demo_id: demo.id.clone(),
        item_count,
        slaves,
        floorplan_conflicts,
        warnings,
    })
}

/// What may happen to a floorplan of the device with the same name as a layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Replace {
    /// Skip the layout with a warning.
    No,
    /// Replace floorplans the app created (ids in `lw_demo.floorplanIds`) when nobody edited
    /// them; keep an edited one and create the layout next to it as `<name> (v<version>)`.
    /// A same-name floorplan the app did not create is never touched: the layout is skipped.
    /// This is what an upgrade does.
    OwnedOnly,
    /// Replace every floorplan with that name (the user confirmed the conflicts).
    All,
}

/// Which floorplans of a demo to (re)create.
pub struct FloorplanJob<'a> {
    /// Library path + layout.
    pub layouts: Vec<(&'a str, &'a FloorplanLayout)>,
    pub replace: Replace,
}

/// Outcome of [`apply_floorplans`].
#[derive(Debug, Default)]
pub struct FloorplanOutcome {
    pub created: Vec<FloorplanRef>,
    pub replaced: Vec<FloorplanRef>,
    pub warnings: Vec<String>,
}

/// Create the floorplans of `job` for the device and record them in `state`
/// (`floorplanIds`, `appliedFloorplans`, `appliedFloorplanBodies`). `lw_demo` is written after
/// every created / replaced floorplan, so an error leaves a consistent record. A replaced
/// floorplan is deleted (with its binary) only after the new one is created, linked and recorded.
#[allow(clippy::too_many_arguments)]
pub async fn apply_floorplans(
    client: &C8yClient,
    library: &Library,
    device_id: &str,
    resolved: &ResolvedSetup,
    groups: &[DaliGroup],
    inputs: &[DigitalInputEntry],
    state: &mut LwDemoState,
    job: FloorplanJob<'_>,
) -> AppResult<FloorplanOutcome> {
    let mut out = FloorplanOutcome::default();
    let existing = floorplan::list(client, device_id).await?;
    let items = resolved.item_refs();
    let slaves = resolved.slave_refs();
    let ctx = FloorplanContext {
        items: &items,
        slaves: &slaves,
        groups,
        digital_inputs: inputs,
    };
    for (path, layout) in job.layouts {
        let old: Vec<&Value> = existing
            .iter()
            .filter(|mo| mo.get("name").and_then(Value::as_str) == Some(layout.name.as_str()))
            .collect();
        let mut to_replace: Vec<FloorplanRef> = Vec::new();
        let mut name_override: Option<String> = None;
        if !old.is_empty() {
            match job.replace {
                Replace::No => {
                    out.warnings.push(format!(
                        "floorplan \"{}\" exists and was not replaced",
                        layout.name
                    ));
                    continue;
                }
                Replace::All => {
                    to_replace = old
                        .iter()
                        .map(|mo| floorplan::reference(mo))
                        .collect::<AppResult<_>>()?;
                }
                Replace::OwnedOnly => {
                    let mut owned = Vec::new();
                    for mo in &old {
                        let r = floorplan::reference(mo)?;
                        if state.floorplan_ids.contains(&r.id) {
                            owned.push((r, *mo));
                        }
                    }
                    if owned.is_empty() {
                        out.warnings.push(format!(
                            "floorplan \"{}\" exists on the device but was not created by the app: not touched, layout skipped",
                            layout.name
                        ));
                        continue;
                    }
                    let edited = owned.iter().any(|(r, mo)| {
                        state
                            .applied_floorplan_bodies
                            .get(&r.id)
                            .is_some_and(|d| *d != floorplan::body_digest(mo))
                    });
                    if edited {
                        // Somebody edited it in the plugin: keep it (it is theirs now) and put
                        // the new layout next to it.
                        for (r, _) in &owned {
                            state.floorplan_ids.retain(|id| *id != r.id);
                            state.applied_floorplan_bodies.remove(&r.id);
                        }
                        name_override = Some(format!("{} (v{})", layout.name, library.version));
                        out.warnings.push(format!(
                            "floorplan \"{}\" was edited on the device: kept, the new layout was created next to it",
                            layout.name
                        ));
                    } else {
                        to_replace = owned.into_iter().map(|(r, _)| r).collect();
                    }
                }
            }
        }
        let dir = library
            .root
            .join(Path::new(path).parent().unwrap_or(Path::new("")));
        let created = match &name_override {
            Some(name) => {
                let mut renamed = layout.clone();
                renamed.name.clone_from(name);
                floorplan::create(client, device_id, &renamed, &dir, &ctx).await?
            }
            None => floorplan::create(client, device_id, layout, &dir, &ctx).await?,
        };
        state.floorplan_ids.push(created.reference.id.clone());
        state
            .applied_floorplans
            .insert(layout.name.clone(), setup::digest(layout));
        state
            .applied_floorplan_bodies
            .insert(created.reference.id.clone(), created.body_digest.clone());
        out.created.push(created.reference);
        demo_state::write(client, device_id, state).await?;
        for o in to_replace {
            floorplan::remove(client, &o.id).await?;
            state.floorplan_ids.retain(|id| *id != o.id);
            state.applied_floorplan_bodies.remove(&o.id);
            demo_state::write(client, device_id, state).await?;
            out.replaced.push(o);
        }
    }
    Ok(out)
}

/// Select items, assign slaves, write `lw_config*` + `lw_demo`, apply floorplans.
/// The caller stops a running session on the device first.
pub async fn apply(
    client: &C8yClient,
    library: &Library,
    device_id: &str,
    req: &ApplyDemoRequest,
) -> AppResult<ApplyDemoResult> {
    let demo = demo_of(library, &req.demo_id)?;
    let mo = crud::load_demo_mo(client, device_id).await?;
    let existing = demo_state::read(&mo)?;
    if let Some(e) = &existing {
        // Another presenter may be simulating this device right now.
        demo_state::check_lock(e, client.username(), req.force, chrono::Utc::now())?;
    }
    let mut p = plan(library, demo, existing.as_ref())?;
    let mut body = p.config.clone();
    if let (Some(b), Value::Object(demo_patch)) =
        (body.as_object_mut(), demo_state::patch_body(&p.state)?)
    {
        b.extend(demo_patch);
    }
    body["lw_configCore"] = demo_state::core_with_lamps(&mo, p.state.dali_lamp_count);
    // Devices from older app versions: add the agent fragment etc. in the same PUT.
    if let (Some(b), Some(Value::Object(missing))) = (
        body.as_object_mut(),
        crate::payload::config::missing_required_fragments(&mo),
    ) {
        b.extend(missing);
    }
    client.update_managed_object(device_id, &body).await?;

    let mut outcome = FloorplanOutcome::default();
    if req.apply_floorplans {
        outcome = apply_floorplans(
            client,
            library,
            device_id,
            &p.resolved,
            &demo.dali_groups,
            &p.inputs,
            &mut p.state,
            FloorplanJob {
                layouts: layouts(library, demo),
                replace: if req.replace_existing_floorplans {
                    Replace::All
                } else {
                    Replace::No
                },
            },
        )
        .await?;
    }
    let mut warnings = p.warnings;
    warnings.extend(outcome.warnings);
    Ok(ApplyDemoResult {
        device: crud::get(client, library, device_id).await?,
        floorplans_created: outcome.created,
        floorplans_replaced: outcome.replaced,
        warnings,
    })
}

/// Create a demo device and, when the request asks for it, apply a demo pack right away. A
/// failed apply deletes the device again: a half-configured device whose id the UI never saw
/// is worse than none (as for a failed parent link).
pub async fn create_device(
    client: &C8yClient,
    library: &Library,
    request: &CreateDeviceRequest,
) -> AppResult<CreateDeviceResult> {
    let created = crud::create(client, library, request).await?;
    let Some(pack) = &request.apply_demo else {
        return Ok(CreateDeviceResult {
            device: created,
            warnings: Vec::new(),
        });
    };
    match apply(client, library, &created.id, pack).await {
        Ok(applied) => Ok(CreateDeviceResult {
            device: applied.device,
            warnings: applied.warnings,
        }),
        Err(e) => {
            if let Err(d) = crud::delete(client, &created.id).await {
                log::warn!("rolling back device {} failed: {d}", created.id);
            }
            Err(e)
        }
    }
}
