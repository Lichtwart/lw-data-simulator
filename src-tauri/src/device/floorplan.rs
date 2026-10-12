//! Floorplan upload / link / delete (PRD §12, docs/research/floorplan-plugin.md §1–§2).
//!
//! `POST /inventory/binaries` (image) → `POST /inventory/managedObjects` (`lw_floorplan`, built
//! by `payload::floorplan::floorplan_body`) → `POST /inventory/managedObjects/{device}/
//! childAdditions`. If linking fails the new managed object and binary are deleted again (as the
//! plugin does).

use std::path::Path;

use serde_json::Value;

use crate::c8y::{C8yClient, id_of};
use crate::error::{AppError, AppResult};
use crate::model::{FloorplanLayout, FloorplanRef};
use crate::payload::floorplan::{FloorplanContext, floorplan_body};

/// Managed object type of floorplans.
pub const FLOORPLAN_TYPE: &str = crate::payload::floorplan::FLOORPLAN_TYPE;

/// MIME type of a floorplan image by file extension (PNG, JPG, SVG only).
pub fn image_mime(file: &str) -> AppResult<&'static str> {
    let ext = Path::new(file)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    match ext.as_str() {
        "png" => Ok("image/png"),
        "jpg" | "jpeg" => Ok("image/jpeg"),
        "svg" => Ok("image/svg+xml"),
        _ => Err(AppError::InvalidInput(format!(
            "floorplan image {file}: only .png, .jpg, .svg"
        ))),
    }
}

/// File extension for an image MIME type (export).
pub fn mime_extension(mime: &str) -> &'static str {
    match mime.split(';').next().unwrap_or("").trim() {
        "image/jpeg" | "image/jpg" => "jpg",
        "image/svg+xml" => "svg",
        _ => "png",
    }
}

/// `lw_floorplan` managed objects linked to `device_id`.
pub async fn list(client: &C8yClient, device_id: &str) -> AppResult<Vec<Value>> {
    Ok(client
        .list_child_additions(device_id)
        .await?
        .into_iter()
        .filter(|mo| mo.get("type").and_then(Value::as_str) == Some(FLOORPLAN_TYPE))
        .collect())
}

/// `{id, name}` of a floorplan managed object.
pub fn reference(mo: &Value) -> AppResult<FloorplanRef> {
    Ok(FloorplanRef {
        id: id_of(mo)?,
        name: mo
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
    })
}

/// A floorplan the app just created.
#[derive(Debug, Clone)]
pub struct CreatedFloorplan {
    pub reference: FloorplanRef,
    /// Digest of the stored `lw_floorplan` body (to tell later whether somebody edited it).
    pub body_digest: String,
}

/// Digest of the `lw_floorplan` fragment of a floorplan managed object.
pub fn body_digest(mo: &Value) -> String {
    super::setup::digest(mo.get(FLOORPLAN_TYPE).unwrap_or(&Value::Null))
}

/// Upload the image of `layout` (read from `layout_dir`), create the `lw_floorplan` and link it
/// to the device. Returns the created reference and the digest of what was stored.
pub async fn create(
    client: &C8yClient,
    device_id: &str,
    layout: &FloorplanLayout,
    layout_dir: &Path,
    ctx: &FloorplanContext<'_>,
) -> AppResult<CreatedFloorplan> {
    let mime = image_mime(&layout.image)?;
    let path = layout_dir.join(&layout.image);
    let bytes =
        std::fs::read(&path).map_err(|e| AppError::Io(format!("{}: {e}", path.display())))?;
    // Resolve the widgets before uploading, so a broken layout leaves nothing behind.
    floorplan_body(layout, ctx, "0")?;
    let binary = client.upload_binary(&layout.image, mime, bytes).await?;
    let binary_id = id_of(&binary)?;
    let body = floorplan_body(layout, ctx, &binary_id)?;
    let mo = match client.create_managed_object(&body).await {
        Ok(mo) => mo,
        Err(e) => {
            let _ = client.delete_binary(&binary_id).await;
            return Err(e);
        }
    };
    let fp = reference(&mo)?;
    if let Err(e) = client.add_child_addition(device_id, &fp.id).await {
        let _ = client.delete_managed_object(&fp.id).await;
        let _ = client.delete_binary(&binary_id).await;
        return Err(e);
    }
    let stored = if mo.get(FLOORPLAN_TYPE).is_some() {
        &mo
    } else {
        &body
    };
    Ok(CreatedFloorplan {
        reference: fp,
        body_digest: body_digest(stored),
    })
}

/// Delete a floorplan managed object and its image binary, **without** checking what it is. Only
/// for objects that are known to be linked floorplans of the device (taken from [`list`]). A
/// floorplan that is already gone is fine (404 ignored).
pub async fn remove(client: &C8yClient, floorplan_id: &str) -> AppResult<()> {
    let Some(mo) = client.find_managed_object(floorplan_id).await? else {
        return Ok(());
    };
    client.delete_managed_object(floorplan_id).await?;
    if let Some(binary) = mo.pointer("/lw_floorplan/imageBinaryId").and_then(|v| {
        v.as_str()
            .map(str::to_owned)
            .or_else(|| v.as_u64().map(|n| n.to_string()))
    }) && let Err(e) = client.delete_binary(&binary).await
        && !crate::c8y::is_http_status(&e, 404)
    {
        return Err(e);
    }
    Ok(())
}

/// Delete the floorplans `ids` (from `lw_demo.floorplanIds`, which is plain inventory data
/// anyone can edit) of `device_id`. An id is only deleted when it is an `lw_floorplan` that is
/// linked to this device as a child addition; anything else is skipped with a warning. Returns
/// the ids that are gone afterwards (deleted or already missing).
pub async fn delete_many(
    client: &C8yClient,
    device_id: &str,
    ids: &[String],
) -> AppResult<Vec<String>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let linked = list(client, device_id).await?;
    let mut gone = Vec::new();
    for id in ids {
        if linked
            .iter()
            .any(|mo| id_of(mo).ok().as_deref() == Some(id))
        {
            remove(client, id).await?;
            gone.push(id.clone());
        } else if client.find_managed_object(id).await?.is_none() {
            gone.push(id.clone());
        } else {
            log::warn!("floorplan {id} is not a floorplan of device {device_id}: not deleted");
        }
    }
    Ok(gone)
}

/// Delete one floorplan (see [`delete_many`]).
pub async fn delete(client: &C8yClient, device_id: &str, floorplan_id: &str) -> AppResult<()> {
    delete_many(client, device_id, &[floorplan_id.to_owned()])
        .await
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mimes() {
        assert_eq!(image_mime("a.PNG").unwrap(), "image/png");
        assert_eq!(image_mime("a.jpeg").unwrap(), "image/jpeg");
        assert_eq!(image_mime("a.svg").unwrap(), "image/svg+xml");
        assert!(image_mime("a.gif").is_err());
        assert_eq!(mime_extension("image/svg+xml"), "svg");
    }
}
