//! Demo device CRUD (PRD §6).

use serde_json::{Value, json};

use super::{demo_state, floorplan, setup};
use crate::c8y::{C8yClient, ManagedObjectQuery, id_of};
use crate::error::{AppError, AppResult};
use crate::library::Library;
use crate::model::{
    CreateDeviceRequest, DemoDevice, DemoDeviceSummary, GroupRef, Position, ResetDeviceRequest,
    UpdateDeviceRequest,
};
use crate::payload::config::{
    DEMO_DEVICE_TYPE_MARKER, NewDemoDevice, demo_device_body, demo_serial_number,
    missing_required_fragments,
};

/// Inventory query of all demo devices.
pub fn demo_devices_query() -> String {
    format!("$filter=(c8y_DeviceTypes eq '{DEMO_DEVICE_TYPE_MARKER}') $orderby=name")
}

fn str_field(mo: &Value, key: &str) -> Option<String> {
    mo.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// Whether a parent reference says it is a device group (`c8y_IsDeviceGroup` or a group `type`).
fn is_group(parent: &Value) -> bool {
    parent.get("c8y_IsDeviceGroup").is_some()
        || matches!(
            parent.get("type").and_then(Value::as_str),
            Some("c8y_DeviceGroup" | "c8y_DeviceSubgroup")
        )
}

/// The device's parent group (`withParents=true` → `assetParents.references[].managedObject`):
/// the first parent that is a device group. A device can also sit under other assets (a
/// building, a site); they are not shown as a group. If no parent says what it is, the first
/// one is taken (the references may carry only id and name).
pub fn parent_group(mo: &Value) -> Option<GroupRef> {
    let parents: Vec<&Value> = mo
        .pointer("/assetParents/references")
        .and_then(Value::as_array)?
        .iter()
        .filter_map(|r| r.get("managedObject"))
        .collect();
    let to_ref = |p: &Value| {
        Some(GroupRef {
            id: id_of(p).ok()?,
            name: str_field(p, "name").unwrap_or_default(),
        })
    };
    parents
        .iter()
        .find(|p| is_group(p))
        .and_then(|p| to_ref(p))
        .or_else(|| {
            parents
                .iter()
                .filter(|p| p.get("type").is_none())
                .find_map(|p| to_ref(p))
        })
}

/// Case-insensitive AND match of all `query` words on name / description.
pub fn matches_text(mo: &Value, query: &str) -> bool {
    let hay = format!(
        "{} {}",
        str_field(mo, "name").unwrap_or_default(),
        str_field(mo, "description").unwrap_or_default()
    )
    .to_lowercase();
    query
        .split_whitespace()
        .all(|w| hay.contains(&w.to_lowercase()))
}

fn position(mo: &Value) -> Option<Position> {
    let p = mo.get("c8y_Position")?;
    Some(Position {
        lat: p.get("lat")?.as_f64()?,
        lng: p.get("lng")?.as_f64()?,
        alt: p.get("alt").and_then(Value::as_f64).unwrap_or(0.0),
    })
}

fn summary(library: &Library, mo: &Value) -> AppResult<DemoDeviceSummary> {
    // A broken `lw_demo` must not hide the device from the list.
    let demo = demo_state::read(mo).ok().flatten();
    Ok(DemoDeviceSummary {
        id: id_of(mo)?,
        name: str_field(mo, "name").unwrap_or_default(),
        description: str_field(mo, "description"),
        parent_group: parent_group(mo),
        demo_id: demo.as_ref().and_then(|d| d.demo_id.clone()),
        library_version: demo.as_ref().and_then(|d| d.library_version.clone()),
        lock: demo.as_ref().and_then(|d| d.lock.clone()),
        last_updated: str_field(mo, "lastUpdated"),
        upgrade_available: setup::upgrade_available(library, demo.as_ref()),
    })
}

/// Demo devices, optionally filtered by full-text `query` on name/description.
pub async fn list(
    client: &C8yClient,
    library: &Library,
    query: Option<&str>,
) -> AppResult<Vec<DemoDeviceSummary>> {
    let mos = client
        .query_managed_objects(&ManagedObjectQuery {
            query: Some(demo_devices_query()),
            with_parents: true,
            ..Default::default()
        })
        .await?;
    mos.iter()
        .filter(|mo| query.is_none_or(|q| matches_text(mo, q)))
        .map(|mo| summary(library, mo))
        .collect()
}

/// All `c8y_DeviceGroup`s (for the parent picker).
pub async fn list_groups(client: &C8yClient) -> AppResult<Vec<GroupRef>> {
    let mos = client
        .query_managed_objects(&ManagedObjectQuery {
            query: Some("$filter=(has(c8y_IsDeviceGroup)) $orderby=name".into()),
            ..Default::default()
        })
        .await?;
    mos.iter()
        .map(|mo| {
            Ok(GroupRef {
                id: id_of(mo)?,
                name: str_field(mo, "name").unwrap_or_default(),
            })
        })
        .collect()
}

/// `DemoDevice` view of a managed object plus its floorplans.
pub fn device_of(library: &Library, mo: &Value, floorplans: &[Value]) -> AppResult<DemoDevice> {
    let demo = demo_state::read(mo)?;
    Ok(DemoDevice {
        id: id_of(mo)?,
        name: str_field(mo, "name").unwrap_or_default(),
        description: str_field(mo, "description"),
        position: position(mo),
        parent_group: parent_group(mo),
        serial_number: mo
            .pointer("/c8y_Hardware/serialNumber")
            .and_then(Value::as_str)
            .map(str::to_owned),
        firmware_version: mo
            .pointer("/c8y_Firmware/version")
            .and_then(Value::as_str)
            .map(str::to_owned),
        upgrade_available: setup::upgrade_available(library, demo.as_ref()),
        demo,
        floorplans: floorplans
            .iter()
            .map(floorplan::reference)
            .collect::<AppResult<_>>()?,
    })
}

/// Load a demo device (must carry the `demo` marker: the app never touches real devices).
pub async fn load_demo_mo(client: &C8yClient, id: &str) -> AppResult<Value> {
    let mo = client
        .get_json(&format!("/inventory/managedObjects/{id}?withParents=true"))
        .await?;
    ensure_demo(&mo)?;
    Ok(mo)
}

/// `InvalidInput` unless `c8y_DeviceTypes` contains `demo`.
pub fn ensure_demo(mo: &Value) -> AppResult<()> {
    let is_demo = mo
        .get("c8y_DeviceTypes")
        .and_then(Value::as_array)
        .is_some_and(|a| a.iter().any(|t| t == DEMO_DEVICE_TYPE_MARKER));
    if is_demo {
        Ok(())
    } else {
        Err(AppError::InvalidInput(format!(
            "{} is not a demo device",
            mo.get("id").and_then(Value::as_str).unwrap_or("?")
        )))
    }
}

/// Add the fragments a demo device created by an older app version lacks
/// (`com_cumulocity_model_Agent`, current `c8y_SupportedOperations`, see
/// [`missing_required_fragments`]). One PUT only when something is missing; `mo` must be a
/// demo device (loaded via [`load_demo_mo`]). The local copy is updated too.
pub async fn ensure_required_fragments(client: &C8yClient, mo: &mut Value) -> AppResult<()> {
    ensure_demo(mo)?;
    let Some(patch) = missing_required_fragments(mo) else {
        return Ok(());
    };
    let id = id_of(mo)?;
    client.update_managed_object(&id, &patch).await?;
    if let (Some(m), Value::Object(p)) = (mo.as_object_mut(), patch) {
        m.extend(p);
    }
    Ok(())
}

/// Opening a device migrates it ([`ensure_required_fragments`]) on a best-effort basis: a
/// read-only user (403) or a transient error must not stop the device from opening. Session
/// start and apply keep the migration mandatory (they write anyway).
pub async fn get(client: &C8yClient, library: &Library, id: &str) -> AppResult<DemoDevice> {
    let mut mo = load_demo_mo(client, id).await?;
    if let Err(e) = ensure_required_fragments(client, &mut mo).await {
        log::warn!("device {id} could not be migrated, opening it unchanged: {e}");
    }
    let fps = floorplan::list(client, id).await?;
    device_of(library, &mo, &fps)
}

fn random_u32() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut rng =
        crate::sim::generators::SeededRng::new(nanos as u64 ^ u64::from(std::process::id()));
    use crate::sim::generators::RandomSource;
    (rng.next_unit() * f64::from(u32::MAX)) as u32
}

fn check_name(name: &str) -> AppResult<()> {
    if name.trim().is_empty() {
        return Err(AppError::InvalidInput("device name is empty".into()));
    }
    Ok(())
}

fn check_position(p: &Position) -> AppResult<()> {
    if !(-90.0..=90.0).contains(&p.lat) || !(-180.0..=180.0).contains(&p.lng) || !p.alt.is_finite()
    {
        return Err(AppError::InvalidInput(format!(
            "position {}, {}",
            p.lat, p.lng
        )));
    }
    Ok(())
}

/// Create the §6.1 managed object (serial `DEMO-<random>`), link to the parent group.
/// Applying a demo is done by the caller via `apply::apply`.
pub async fn create(
    client: &C8yClient,
    library: &Library,
    req: &CreateDeviceRequest,
) -> AppResult<DemoDevice> {
    check_name(&req.name)?;
    if let Some(p) = &req.position {
        check_position(p)?;
    }
    let serial = demo_serial_number(random_u32());
    let body = demo_device_body(
        &NewDemoDevice {
            name: req.name.trim(),
            description: req.description.as_deref().filter(|d| !d.is_empty()),
            position: req.position,
            serial_number: &serial,
        },
        None,
    )?;
    let mo = client.create_managed_object(&body).await?;
    let id = id_of(&mo)?;
    if let Some(group) = &req.parent_group_id {
        let link: AppResult<Value> = client
            .post_json(
                &format!("/inventory/managedObjects/{group}/childAssets"),
                &json!({ "managedObject": { "id": id } }),
            )
            .await;
        if let Err(e) = link {
            let _ = client.delete_managed_object(&id).await;
            return Err(e);
        }
    }
    get(client, library, &id).await
}

pub async fn update(
    client: &C8yClient,
    library: &Library,
    id: &str,
    req: &UpdateDeviceRequest,
) -> AppResult<DemoDevice> {
    load_demo_mo(client, id).await?;
    let mut body = serde_json::Map::new();
    if let Some(n) = &req.name {
        check_name(n)?;
        body.insert("name".into(), json!(n.trim()));
    }
    if let Some(d) = &req.description {
        body.insert("description".into(), json!(d));
    }
    if let Some(p) = &req.position {
        check_position(p)?;
        body.insert(
            "c8y_Position".into(),
            json!({ "lat": p.lat, "lng": p.lng, "alt": p.alt }),
        );
    }
    if !body.is_empty() {
        client
            .update_managed_object(id, &Value::Object(body))
            .await?;
    }
    get(client, library, id).await
}

/// Delete the app-created floorplans listed in `lw_demo.floorplanIds` (+ binaries). Only ids
/// that really are floorplans of this device are touched.
async fn delete_app_floorplans(
    client: &C8yClient,
    device_id: &str,
    ids: &[String],
) -> AppResult<()> {
    floorplan::delete_many(client, device_id, ids)
        .await
        .map(|_| ())
}

/// Clear ACTIVE/ACKNOWLEDGED alarms, optionally delete app-created floorplans.
/// The caller stops a running session first.
pub async fn reset(client: &C8yClient, id: &str, req: &ResetDeviceRequest) -> AppResult<()> {
    let mo = load_demo_mo(client, id).await?;
    client.clear_open_alarms(id).await?;
    if req.delete_floorplans
        && let Some(mut demo) = demo_state::read(&mo)?
    {
        delete_app_floorplans(client, id, &demo.floorplan_ids).await?;
        demo.floorplan_ids.clear();
        demo.applied_floorplans.clear();
        demo.applied_floorplan_bodies.clear();
        demo_state::write(client, id, &demo).await?;
    }
    Ok(())
}

/// Delete the device incl. app-created floorplans and their binaries.
pub async fn delete(client: &C8yClient, id: &str) -> AppResult<()> {
    let mo = load_demo_mo(client, id).await?;
    if let Some(demo) = demo_state::read(&mo).ok().flatten() {
        delete_app_floorplans(client, id, &demo.floorplan_ids).await?;
    }
    client.delete_managed_object(id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_filter_ands_words() {
        let mo = json!({"name": "Tankstelle Musterstadt", "description": "Shop demo"});
        assert!(matches_text(&mo, "tank shop"));
        assert!(!matches_text(&mo, "tank retail"));
        assert!(matches_text(&mo, ""));
    }

    #[test]
    fn parent_group_from_asset_parents() {
        let mo = json!({"assetParents": {"references": [{"managedObject": {"id": "5", "name": "Nord"}}]}});
        assert_eq!(
            parent_group(&mo),
            Some(GroupRef {
                id: "5".into(),
                name: "Nord".into()
            })
        );
        assert_eq!(parent_group(&json!({})), None);
    }

    /// F20: the group is the parent that is a device group, not just the first asset.
    #[test]
    fn parent_group_skips_parents_that_are_not_groups() {
        let refs = |parents: Vec<Value>| {
            let r: Vec<Value> = parents
                .into_iter()
                .map(|p| json!({ "managedObject": p }))
                .collect();
            json!({ "assetParents": { "references": r } })
        };
        let mo = refs(vec![
            json!({ "id": "1", "name": "Gebaeude", "type": "c8y_Building" }),
            json!({ "id": "2", "name": "Nord", "c8y_IsDeviceGroup": {} }),
        ]);
        assert_eq!(parent_group(&mo).unwrap().id, "2");
        let mo = refs(vec![
            json!({ "id": "3", "name": "Sued", "type": "c8y_DeviceGroup" }),
            json!({ "id": "4", "name": "Ost", "c8y_IsDeviceGroup": {} }),
        ]);
        assert_eq!(parent_group(&mo).unwrap().id, "3");
        // Only a building above the device: no group.
        let mo = refs(vec![
            json!({ "id": "1", "name": "G", "type": "c8y_Building" }),
        ]);
        assert_eq!(parent_group(&mo), None);
    }
}
