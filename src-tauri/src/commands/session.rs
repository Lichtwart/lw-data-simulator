//! Session commands (PRD §7–§9).
//!
//! Commands that talk to the task go through `AppState::session_sender`: the sender is
//! cloned under the `session` lock and the guard is dropped before the reply is awaited.

use tauri::{AppHandle, State};
use tauri_specta::Event;

use crate::error::{AppError, AppResult};
use crate::events::SessionChanged;
use crate::model::{
    FaultTarget, GeneratorConfig, GroupMode, OperationDecision, SessionSnapshot,
    SetItemValueRequest, StartSessionRequest,
};
use crate::sim::{SessionCommand, SessionHandle};
use crate::state::AppState;

async fn send(app: &AppHandle, state: &AppState, cmd: SessionCommand) -> AppResult<()> {
    let sender = state.session_sender(app).await?;
    sender.send(cmd).await
}

/// Start simulating a device. Fails with `sessionActive` if another session runs
/// (the UI asks "stop and switch?" and calls `stop_session` first) and with
/// `deviceLocked` on a fresh foreign lock (retry with `force`).
#[tauri::command]
#[specta::specta]
pub async fn start_session(
    app: AppHandle,
    state: State<'_, AppState>,
    request: StartSessionRequest,
) -> AppResult<SessionSnapshot> {
    let client = state.client().await?;
    let library = state.library()?;
    // Preparing a session makes many HTTP calls: the session lock is held only to look and to
    // store, and a claim keeps a second concurrent start out (`get_session`, `stop_session`,
    // `logout` and the 401 path never wait for a slow tenant).
    let claim = {
        let mut guard = state.session.lock().await;
        if guard.as_ref().is_some_and(SessionHandle::is_finished) {
            *guard = None;
        }
        if let Some(running) = guard.as_ref() {
            return Err(AppError::SessionActive(running.device_id.clone()));
        }
        state.claim_start(&request.device_id)?
    };
    // A stop that is still draining (lock release, final statuses) must be done first.
    state.wait_for_stops().await;
    let handle = SessionHandle::start(app.clone(), client.clone(), library, request).await?;
    // A logout (or a 401) while the session was starting: do not keep a session that runs
    // with a client the app has forgotten.
    if !state.is_current_login(&client).await {
        let _ = handle.stop().await;
        return Err(AppError::NotAuthenticated);
    }
    let snapshot = handle.snapshot();
    *state.session.lock().await = Some(handle);
    drop(claim);
    let _ = SessionChanged(Some(snapshot.clone())).emit(&app);
    Ok(snapshot)
}

/// Stop the running session (no-op if none).
#[tauri::command]
#[specta::specta]
pub async fn stop_session(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    state.stop_session(&app).await?;
    Ok(())
}

/// Current session snapshot, if any (a finished task counts as no session).
#[tauri::command]
#[specta::specta]
pub async fn get_session(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<Option<SessionSnapshot>> {
    Ok(state.session_snapshot(&app).await)
}

/// Manually set an item value (overrides its generator until `resume_generator`).
#[tauri::command]
#[specta::specta]
pub async fn set_item_value(
    app: AppHandle,
    state: State<'_, AppState>,
    request: SetItemValueRequest,
) -> AppResult<()> {
    let SetItemValueRequest { item_id, value } = request;
    send(&app, &state, SessionCommand::SetValue { item_id, value }).await
}

/// Hand an item back to its generator.
#[tauri::command]
#[specta::specta]
pub async fn resume_generator(
    app: AppHandle,
    state: State<'_, AppState>,
    item_id: String,
) -> AppResult<()> {
    send(&app, &state, SessionCommand::ResumeGenerator { item_id }).await
}

/// Replace (or remove with `null`) an item's generator for this session.
/// Persisting it to `lw_demo.generators` is done via `update_demo_setup`.
#[tauri::command]
#[specta::specta]
pub async fn set_item_generator(
    app: AppHandle,
    state: State<'_, AppState>,
    item_id: String,
    generator: Option<GeneratorConfig>,
) -> AppResult<()> {
    send(
        &app,
        &state,
        SessionCommand::SetGenerator { item_id, generator },
    )
    .await
}

/// Apply a scenario of the device's demo pack.
#[tauri::command]
#[specta::specta]
pub async fn apply_scenario(
    app: AppHandle,
    state: State<'_, AppState>,
    scenario_id: String,
) -> AppResult<()> {
    send(&app, &state, SessionCommand::ApplyScenario { scenario_id }).await
}

/// Switch a DALI group locally (same effect as a confirmed `lw_switchGroupMode`).
#[tauri::command]
#[specta::specta]
pub async fn set_group_mode(
    app: AppHandle,
    state: State<'_, AppState>,
    group_id: u32,
    mode: GroupMode,
) -> AppResult<()> {
    send(
        &app,
        &state,
        SessionCommand::SetGroupMode { group_id, mode },
    )
    .await
}

/// Fire a fault once: post its alarm (again, if it is open: Cumulocity counts up). The effect
/// lasts until the alarm is cleared in Cumulocity (DECISIONS D9).
#[tauri::command]
#[specta::specta]
pub async fn fire_fault(
    app: AppHandle,
    state: State<'_, AppState>,
    target: FaultTarget,
) -> AppResult<()> {
    send(
        &app,
        &state,
        SessionCommand::SetFault {
            target,
            active: true,
        },
    )
    .await
}

/// Clear every open (ACTIVE / ACKNOWLEDGED) alarm of the device in Cumulocity, also alarms the
/// simulator did not raise. A running session on the device does it itself (every effect ends,
/// D9); without one it is a plain REST call.
#[tauri::command]
#[specta::specta]
pub async fn clear_all_alarms(
    app: AppHandle,
    state: State<'_, AppState>,
    device_id: String,
) -> AppResult<()> {
    if state
        .send_to_session_for(&app, &device_id, SessionCommand::ClearAllAlarms)
        .await?
    {
        return Ok(());
    }
    state.client().await?.clear_open_alarms(&device_id).await?;
    Ok(())
}

/// Change the periodic send interval for this session (max 899 s).
#[tauri::command]
#[specta::specta]
pub async fn set_send_interval(
    app: AppHandle,
    state: State<'_, AppState>,
    seconds: u32,
) -> AppResult<()> {
    send(&app, &state, SessionCommand::SetSendInterval { seconds }).await
}

/// Toggle auto-accept of visible operations.
#[tauri::command]
#[specta::specta]
pub async fn set_auto_accept(
    app: AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
) -> AppResult<()> {
    send(&app, &state, SessionCommand::SetAutoAccept { enabled }).await
}

/// Send all current values now.
#[tauri::command]
#[specta::specta]
pub async fn send_all_now(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    send(&app, &state, SessionCommand::SendAllNow).await
}

/// Write current live values to `lw_demo.snapshot`.
#[tauri::command]
#[specta::specta]
pub async fn save_snapshot(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    send(&app, &state, SessionCommand::SaveSnapshot).await
}

/// Confirm or abort a prompted visible operation.
#[tauri::command]
#[specta::specta]
pub async fn answer_operation(
    app: AppHandle,
    state: State<'_, AppState>,
    operation_id: String,
    decision: OperationDecision,
) -> AppResult<()> {
    send(
        &app,
        &state,
        SessionCommand::AnswerOperation {
            operation_id,
            decision,
        },
    )
    .await
}

/// This file's commands (see `commands/mod.rs`).
macro_rules! session_commands {
    ([$($acc:tt)*] $($rest:ident)*) => {
        $crate::commands::collect_all!(
            [$($acc)*
                $crate::commands::session::start_session,
                $crate::commands::session::stop_session,
                $crate::commands::session::get_session,
                $crate::commands::session::set_item_value,
                $crate::commands::session::resume_generator,
                $crate::commands::session::set_item_generator,
                $crate::commands::session::apply_scenario,
                $crate::commands::session::set_group_mode,
                $crate::commands::session::fire_fault,
                $crate::commands::session::clear_all_alarms,
                $crate::commands::session::set_send_interval,
                $crate::commands::session::set_auto_accept,
                $crate::commands::session::send_all_now,
                $crate::commands::session::save_snapshot,
                $crate::commands::session::answer_operation,
            ]
            $($rest)*
        )
    };
}
pub(crate) use session_commands;
