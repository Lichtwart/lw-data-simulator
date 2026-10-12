//! Demo device commands (PRD §6, §12–§14, D6).

use tauri::{AppHandle, State};

use crate::device;
use crate::error::{AppError, AppResult};
use crate::model::{
    ApplyDemoPreview, ApplyDemoRequest, ApplyDemoResult, CreateDeviceRequest, CreateDeviceResult,
    DaliGroup, DemoDevice, DemoDeviceSummary, DemoSetupPatch, DeviceConfig, DigitalInputEntry,
    ExportRequest, ExportResult, FaultDefinition, GroupRef, LwDemoState, ResetDeviceRequest,
    UpdateDeviceRequest, UpgradePreview,
};
use crate::sim;
use crate::state::AppState;

/// Demo devices (`c8y_DeviceTypes` ∋ `demo`), optional full-text filter.
#[tauri::command]
#[specta::specta]
pub async fn list_demo_devices(
    state: State<'_, AppState>,
    query: Option<String>,
) -> AppResult<Vec<DemoDeviceSummary>> {
    device::crud::list(&state.client().await?, &*state.library()?, query.as_deref()).await
}

/// Device groups for the parent picker.
#[tauri::command]
#[specta::specta]
pub async fn list_device_groups(state: State<'_, AppState>) -> AppResult<Vec<GroupRef>> {
    device::crud::list_groups(&state.client().await?).await
}

/// One demo device incl. `lw_demo`, floorplans, upgrade flag.
#[tauri::command]
#[specta::specta]
pub async fn get_demo_device(
    state: State<'_, AppState>,
    device_id: String,
) -> AppResult<DemoDevice> {
    device::crud::get(&state.client().await?, &*state.library()?, &device_id).await
}

/// Create a demo device, optionally applying a demo pack right away (rolled back on failure).
#[tauri::command]
#[specta::specta]
pub async fn create_demo_device(
    state: State<'_, AppState>,
    request: CreateDeviceRequest,
) -> AppResult<CreateDeviceResult> {
    device::apply::create_device(&state.client().await?, &*state.library()?, &request).await
}

/// Edit name, description, position.
#[tauri::command]
#[specta::specta]
pub async fn update_demo_device(
    state: State<'_, AppState>,
    device_id: String,
    request: UpdateDeviceRequest,
) -> AppResult<DemoDevice> {
    device::crud::update(
        &state.client().await?,
        &*state.library()?,
        &device_id,
        &request,
    )
    .await
}

/// Stop the session (if on this device), clear alarms, optionally delete floorplans.
#[tauri::command]
#[specta::specta]
pub async fn reset_demo_device(
    app: AppHandle,
    state: State<'_, AppState>,
    device_id: String,
    request: ResetDeviceRequest,
) -> AppResult<()> {
    state.stop_session_for(&app, &device_id).await?;
    device::crud::reset(&state.client().await?, &device_id, &request).await
}

/// Delete the device and its app-created floorplans (after UI confirmation).
#[tauri::command]
#[specta::specta]
pub async fn delete_demo_device(
    app: AppHandle,
    state: State<'_, AppState>,
    device_id: String,
) -> AppResult<()> {
    state.stop_session_for(&app, &device_id).await?;
    device::crud::delete(&state.client().await?, &device_id).await
}

/// What applying a demo would change (slaves, floorplan conflicts).
#[tauri::command]
#[specta::specta]
pub async fn preview_apply_demo(
    state: State<'_, AppState>,
    device_id: String,
    request: ApplyDemoRequest,
) -> AppResult<ApplyDemoPreview> {
    device::apply::preview(
        &state.client().await?,
        &*state.library()?,
        &device_id,
        &request,
    )
    .await
}

/// Apply a demo pack (stops a running session on this device first).
#[tauri::command]
#[specta::specta]
pub async fn apply_demo(
    app: AppHandle,
    state: State<'_, AppState>,
    device_id: String,
    request: ApplyDemoRequest,
) -> AppResult<ApplyDemoResult> {
    state.stop_session_for(&app, &device_id).await?;
    device::apply::apply(
        &state.client().await?,
        &*state.library()?,
        &device_id,
        &request,
    )
    .await
}

/// Change the `lw_demo` setup (includes/excludes, generators, interval, realistic mode,
/// custom slaves / items — D6). A running session on this device picks the change up.
#[tauri::command]
#[specta::specta]
pub async fn update_demo_setup(
    app: AppHandle,
    state: State<'_, AppState>,
    device_id: String,
    patch: DemoSetupPatch,
) -> AppResult<LwDemoState> {
    let demo = device::demo_state::patch_setup(
        &state.client().await?,
        &*state.library()?,
        &device_id,
        &patch,
    )
    .await?;
    // A running session on this device picks up interval, realistic mode and generators.
    let _ = state
        .send_to_session_for(
            &app,
            &device_id,
            sim::SessionCommand::SetupChanged {
                demo: Box::new(demo.clone()),
            },
        )
        .await;
    Ok(demo)
}

/// The device's editable config fragments: DALI groups (`lw_configGroups`) and digital inputs
/// (`lw_configDigitalInputs`, incl. titles). D6.
#[tauri::command]
#[specta::specta]
pub async fn get_device_config(
    state: State<'_, AppState>,
    device_id: String,
) -> AppResult<DeviceConfig> {
    device::demo_state::get_config(&state.client().await?, &device_id).await
}

/// Replace `lw_configGroups` on the device (≤ 16 groups). Returns what was written. D6. A
/// running session on this device takes the change at once.
#[tauri::command]
#[specta::specta]
pub async fn set_dali_groups(
    app: AppHandle,
    state: State<'_, AppState>,
    device_id: String,
    groups: Vec<DaliGroup>,
) -> AppResult<Vec<DaliGroup>> {
    let written =
        device::demo_state::set_groups(&state.client().await?, &device_id, &groups).await?;
    // A running session on this device takes the new groups at once.
    let _ = state
        .send_to_session_for(
            &app,
            &device_id,
            sim::SessionCommand::ConfigChanged {
                groups: Some(written.clone()),
                inputs: None,
            },
        )
        .await;
    Ok(written)
}

/// Replace `lw_configDigitalInputs` on the device (≤ 10 inputs, titles included). D6. A running
/// session on this device takes the change at once.
#[tauri::command]
#[specta::specta]
pub async fn set_digital_inputs(
    app: AppHandle,
    state: State<'_, AppState>,
    device_id: String,
    inputs: Vec<DigitalInputEntry>,
) -> AppResult<Vec<DigitalInputEntry>> {
    let written =
        device::demo_state::set_digital_inputs(&state.client().await?, &device_id, &inputs).await?;
    // A running session on this device takes the new inputs (series, unit, actions) at once.
    let _ = state
        .send_to_session_for(
            &app,
            &device_id,
            sim::SessionCommand::ConfigChanged {
                groups: None,
                inputs: Some(written.clone()),
            },
        )
        .await;
    Ok(written)
}

/// Selectable faults for the device's setup (firmware texts + severities).
#[tauri::command]
#[specta::specta]
pub async fn fault_catalog(
    state: State<'_, AppState>,
    device_id: String,
) -> AppResult<Vec<FaultDefinition>> {
    let lib = state.library()?;
    let dev = device::crud::get(&state.client().await?, &lib, &device_id).await?;
    let demo = dev
        .demo
        .ok_or_else(|| AppError::NotFound(format!("lw_demo on {device_id}")))?;
    Ok(sim::faults::catalog(&lib, &demo))
}

/// Export the device as a `library/`-shaped demo pack folder.
#[tauri::command]
#[specta::specta]
pub async fn export_demo_pack(
    state: State<'_, AppState>,
    request: ExportRequest,
) -> AppResult<ExportResult> {
    device::export::export(&state.client().await?, &*state.library()?, &request).await
}

/// Upgrade preview, `None` if up to date.
#[tauri::command]
#[specta::specta]
pub async fn check_upgrade(
    state: State<'_, AppState>,
    device_id: String,
) -> AppResult<Option<UpgradePreview>> {
    device::upgrade::check(&state.client().await?, &*state.library()?, &device_id).await
}

/// Apply the library upgrade (stops a running session on this device first). `force` takes
/// over a fresh lock of another presenter.
#[tauri::command]
#[specta::specta]
pub async fn apply_upgrade(
    app: AppHandle,
    state: State<'_, AppState>,
    device_id: String,
    force: bool,
) -> AppResult<DemoDevice> {
    state.stop_session_for(&app, &device_id).await?;
    device::upgrade::apply(
        &state.client().await?,
        &*state.library()?,
        &device_id,
        force,
    )
    .await
}

/// This file's commands (see `commands/mod.rs`).
macro_rules! devices_commands {
    ([$($acc:tt)*] $($rest:ident)*) => {
        $crate::commands::collect_all!(
            [$($acc)*
                $crate::commands::devices::list_demo_devices,
                $crate::commands::devices::list_device_groups,
                $crate::commands::devices::get_demo_device,
                $crate::commands::devices::create_demo_device,
                $crate::commands::devices::update_demo_device,
                $crate::commands::devices::reset_demo_device,
                $crate::commands::devices::delete_demo_device,
                $crate::commands::devices::preview_apply_demo,
                $crate::commands::devices::apply_demo,
                $crate::commands::devices::update_demo_setup,
                $crate::commands::devices::get_device_config,
                $crate::commands::devices::set_dali_groups,
                $crate::commands::devices::set_digital_inputs,
                $crate::commands::devices::fault_catalog,
                $crate::commands::devices::export_demo_pack,
                $crate::commands::devices::check_upgrade,
                $crate::commands::devices::apply_upgrade,
            ]
            $($rest)*
        )
    };
}
pub(crate) use devices_commands;
