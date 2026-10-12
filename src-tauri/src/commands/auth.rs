//! Auth commands (PRD §5).

use std::path::PathBuf;
use std::sync::Arc;

use tauri::{AppHandle, Manager, State};

use crate::c8y::{C8yClient, TauriSink, credentials};
use crate::error::{AppError, AppResult};
use crate::model::{AuthState, LoginRequest, SavedLogin, Tenant};
use crate::state::{AppState, Authenticated};

fn config_dir(app: &AppHandle) -> AppResult<PathBuf> {
    app.path()
        .app_config_dir()
        .map_err(|e| AppError::Io(e.to_string()))
}

/// Tenants baked into the app (`library/tenants.json`).
#[tauri::command]
#[specta::specta]
pub fn list_tenants(state: State<'_, AppState>) -> AppResult<Vec<Tenant>> {
    Ok(state.tenants()?.as_ref().clone())
}

/// Log in with username + password (Basic auth, validated via `GET /user/currentUser`).
/// With `remember`, stores the password in the OS keychain (and the user name in the app
/// config) for that tenant. Replaces any existing login (stops a running session and emits
/// `session-changed(null)`).
#[tauri::command]
#[specta::specta]
pub async fn login(
    app: AppHandle,
    state: State<'_, AppState>,
    request: LoginRequest,
) -> AppResult<AuthState> {
    do_login(&app, &state, request).await
}

async fn do_login(
    app: &AppHandle,
    state: &AppState,
    request: LoginRequest,
) -> AppResult<AuthState> {
    let username = request.username.trim().to_owned();
    if username.is_empty() || request.password.is_empty() {
        return Err(AppError::InvalidInput(
            "username and password are required".into(),
        ));
    }
    let request = LoginRequest {
        username,
        ..request
    };
    let tenant = state.tenant(&request.tenant_key)?;
    let client = C8yClient::new(
        tenant,
        request.username.clone(),
        request.password.clone(),
        state.activity.clone(),
        Arc::new(TauriSink::new(app.clone())),
    )?;
    let user = client.current_user().await?;
    if request.remember {
        // "Remember me" is optional (PRD §5): a refused keychain prompt or a locked keychain
        // must not block a valid login.
        let saved = config_dir(app).and_then(|dir| {
            credentials::save(
                &dir,
                &request.tenant_key,
                &request.username,
                &request.password,
            )
        });
        if let Err(e) = saved {
            log::warn!(
                "remember-me failed for tenant {}: {e}; logged in anyway",
                request.tenant_key
            );
        }
    }
    state.stop_session(app).await?;
    *state.auth.write().await = Some(Authenticated {
        client,
        user: user.clone(),
    });
    Ok(user)
}

/// Log in with the login remembered for `tenant_key`. The keychain secret is read here and
/// only here.
#[tauri::command]
#[specta::specta]
pub async fn login_saved(
    app: AppHandle,
    state: State<'_, AppState>,
    tenant_key: String,
) -> AppResult<AuthState> {
    let username = credentials::saved_username(&config_dir(&app)?, &tenant_key)
        .ok_or_else(|| AppError::NotFound(format!("saved login {tenant_key}")))?;
    let password = credentials::load_password(&tenant_key)?
        .ok_or_else(|| AppError::NotFound(format!("saved login {tenant_key}")))?;
    do_login(
        &app,
        &state,
        LoginRequest {
            tenant_key,
            username,
            password,
            remember: true,
        },
    )
    .await
}

/// Log out: stop the session and drop the in-memory credentials
/// (keychain entries stay; see `forget_login`).
#[tauri::command]
#[specta::specta]
pub async fn logout(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    state.stop_session(&app).await?;
    *state.auth.write().await = None;
    Ok(())
}

/// The current login, if any.
#[tauri::command]
#[specta::specta]
pub async fn current_auth(state: State<'_, AppState>) -> AppResult<Option<AuthState>> {
    Ok(state.auth.read().await.as_ref().map(|a| a.user.clone()))
}

/// Tenants with a remembered login (user names only). Reads the app config file, never the
/// keychain, so it cannot trigger keychain prompts; any problem yields an empty list.
#[tauri::command]
#[specta::specta]
pub fn saved_logins(app: AppHandle, state: State<'_, AppState>) -> AppResult<Vec<SavedLogin>> {
    let (Ok(dir), Ok(tenants)) = (config_dir(&app), state.tenants()) else {
        return Ok(Vec::new());
    };
    let keys: Vec<String> = tenants.iter().map(|t| t.key.clone()).collect();
    Ok(credentials::saved_logins(&dir, &keys))
}

/// Delete the remembered login of a tenant (keychain secret + stored user name).
#[tauri::command]
#[specta::specta]
pub fn forget_login(app: AppHandle, tenant_key: String) -> AppResult<()> {
    credentials::delete(&config_dir(&app)?, &tenant_key)
}

/// This file's commands (see `commands/mod.rs`).
macro_rules! auth_commands {
    ([$($acc:tt)*] $($rest:ident)*) => {
        $crate::commands::collect_all!(
            [$($acc)*
                $crate::commands::auth::list_tenants,
                $crate::commands::auth::login,
                $crate::commands::auth::login_saved,
                $crate::commands::auth::logout,
                $crate::commands::auth::current_auth,
                $crate::commands::auth::saved_logins,
                $crate::commands::auth::forget_login,
            ]
            $($rest)*
        )
    };
}
pub(crate) use auth_commands;
