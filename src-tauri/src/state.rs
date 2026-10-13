//! Global backend state, registered with `app.manage(AppState)` and injected into
//! commands as `tauri::State<'_, AppState>`.
//!
//! Concurrency model:
//! - `library`: immutable after startup → `Arc`, no lock.
//! - `auth`: `RwLock<Option<Authenticated>>` — written on login/logout, read (and the
//!   client cloned) by every Cumulocity command. Never hold the guard across `.await`
//!   on network I/O: clone the client first ([`AppState::client`]).
//! - `session`: `Mutex<Option<SessionHandle>>` — at most one session. Live values
//!   live inside the session task, not here. Never hold the guard while awaiting the task:
//!   clone the [`SessionSender`] (or take the handle), drop the guard, then await. A handle
//!   whose task has finished counts as "no session" and is pruned on access.
//! - `starting`: the device a `start_session` is preparing right now (it does many HTTP calls;
//!   the session lock is **not** held meanwhile, the claim keeps a second start out).
//! - `activity`: shared ring buffer (`Arc`) also held by every `C8yClient`.
//! - `raw_history`: small in-memory list.

use std::sync::Arc;
use std::time::Duration;

use tauri::AppHandle;
use tauri_specta::Event;
use tokio::sync::{Mutex, RwLock};

use crate::c8y::{ActivityLog, C8yClient};
use crate::error::{AppError, AppResult};
use crate::events::SessionChanged;
use crate::library::Library;
use crate::model::{AuthState, SessionSnapshot};
use crate::raw::RawHistory;
use crate::sim::{SessionHandle, SessionSender};

/// How long quitting the app waits for the session to stop (lock release, final statuses).
pub const QUIT_TIMEOUT: Duration = Duration::from_secs(6);

/// Held while a `start_session` prepares; releases the claim on drop.
pub struct StartClaim<'a> {
    state: &'a AppState,
}

impl Drop for StartClaim<'_> {
    fn drop(&mut self) {
        *self
            .state
            .starting
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = None;
    }
}

/// A logged-in user: the client plus the validated user info.
#[derive(Debug, Clone)]
pub struct Authenticated {
    pub client: C8yClient,
    pub user: AuthState,
}

pub struct AppState {
    /// `Err` if the bundled library failed to load (reported by library commands).
    pub library: Result<Arc<Library>, AppError>,
    pub auth: RwLock<Option<Authenticated>>,
    pub session: Mutex<Option<SessionHandle>>,
    /// Device of the `start_session` that is preparing right now.
    starting: std::sync::Mutex<Option<String>>,
    /// Held for the whole of a stop (the handle is out of `session` while the task drains for
    /// up to seconds). A start waits for it, so the old task cannot clear the new lock.
    stop_gate: Mutex<()>,
    pub activity: Arc<ActivityLog>,
    pub raw_history: Mutex<RawHistory>,
}

impl AppState {
    pub fn new(library: Result<Library, AppError>) -> Self {
        Self {
            library: library.map(Arc::new),
            auth: RwLock::new(None),
            session: Mutex::new(None),
            starting: std::sync::Mutex::new(None),
            stop_gate: Mutex::new(()),
            activity: Arc::new(ActivityLog::new()),
            raw_history: Mutex::new(RawHistory::default()),
        }
    }

    /// The loaded library or the load error.
    pub fn library(&self) -> AppResult<Arc<Library>> {
        self.library.clone()
    }

    /// Clone of the logged-in client, or `NotAuthenticated`.
    pub async fn client(&self) -> AppResult<C8yClient> {
        self.auth
            .read()
            .await
            .as_ref()
            .map(|a| a.client.clone())
            .ok_or(AppError::NotAuthenticated)
    }

    // ───────────── session helpers (shared by every command that touches the session) ─────────────

    /// Claim the right to start a session on `device_id` (released when the guard drops).
    /// `SessionActive` while another start is in progress.
    pub fn claim_start(&self, device_id: &str) -> AppResult<StartClaim<'_>> {
        let mut g = self.starting.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(other) = g.as_ref() {
            return Err(AppError::SessionActive(other.clone()));
        }
        *g = Some(device_id.to_owned());
        Ok(StartClaim { state: self })
    }

    /// Wait until no stop is in progress (`start_session` calls this after its claim: a stop
    /// that began earlier is a session of the past, one that begins later finds none).
    pub async fn wait_for_stops(&self) {
        drop(self.stop_gate.lock().await);
    }

    /// Whether `client` is still the logged-in client (same tenant and user). A `logout` /
    /// `expire_auth` that ran while a start was preparing makes this false.
    pub async fn is_current_login(&self, client: &C8yClient) -> bool {
        self.auth.read().await.as_ref().is_some_and(|a| {
            a.client.tenant().key == client.tenant().key && a.client.username() == client.username()
        })
    }

    /// Send `cmd` to the running session if it simulates `device_id`; `Ok(false)` otherwise.
    pub async fn send_to_session_for(
        &self,
        app: &AppHandle,
        device_id: &str,
        cmd: crate::sim::SessionCommand,
    ) -> AppResult<bool> {
        let sender = {
            let mut guard = self.session.lock().await;
            if Self::prune_dead(&mut guard) {
                let _ = SessionChanged(None).emit(app);
            }
            guard
                .as_ref()
                .filter(|s| s.device_id == device_id)
                .map(SessionHandle::sender)
        };
        match sender {
            Some(s) => s.send(cmd).await.map(|()| true),
            None => Ok(false),
        }
    }

    /// Stop the session for an app exit: the lock is released and open prompts are failed
    /// ("Simulator stopped") so no floorplan control spins forever. Gives up after
    /// [`QUIT_TIMEOUT`] (the app must always be able to quit).
    pub async fn stop_for_exit(&self, app: &AppHandle) {
        match tokio::time::timeout(QUIT_TIMEOUT, self.stop_session(app)).await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => log::warn!("stopping the session on exit failed: {e}"),
            Err(_) => log::warn!("stopping the session on exit timed out"),
        }
    }

    /// Take the handle out of `guard` if its task has ended; returns whether it did.
    fn prune_dead(guard: &mut Option<SessionHandle>) -> bool {
        if guard.as_ref().is_some_and(SessionHandle::is_finished) {
            *guard = None;
            true
        } else {
            false
        }
    }

    /// Sender of the live session (cloned under the lock; the guard is dropped before the
    /// caller awaits). A dead task is pruned and reported as `NoSession`.
    pub async fn session_sender(&self, app: &AppHandle) -> AppResult<SessionSender> {
        let mut guard = self.session.lock().await;
        if Self::prune_dead(&mut guard) {
            let _ = SessionChanged(None).emit(app);
        }
        guard
            .as_ref()
            .map(SessionHandle::sender)
            .ok_or(AppError::NoSession)
    }

    /// Snapshot of the live session; a dead task counts as no session.
    pub async fn session_snapshot(&self, app: &AppHandle) -> Option<SessionSnapshot> {
        let mut guard = self.session.lock().await;
        if Self::prune_dead(&mut guard) {
            let _ = SessionChanged(None).emit(app);
        }
        guard.as_ref().map(SessionHandle::snapshot)
    }

    /// Stop the running session (any device) and emit `session-changed(null)`.
    /// Returns whether one was running. The single place login, logout, 401 handling and
    /// `stop_session` go through.
    pub async fn stop_session(&self, app: &AppHandle) -> AppResult<bool> {
        let _stopping = self.stop_gate.lock().await;
        let handle = self.session.lock().await.take();
        self.finish_stop(app, handle).await
    }

    /// Like [`Self::stop_session`], but only if the session simulates `device_id`.
    pub async fn stop_session_for(&self, app: &AppHandle, device_id: &str) -> AppResult<bool> {
        let _stopping = self.stop_gate.lock().await;
        let handle = {
            let mut guard = self.session.lock().await;
            if guard.as_ref().is_some_and(|s| s.device_id == device_id) {
                guard.take()
            } else {
                None
            }
        };
        self.finish_stop(app, handle).await
    }

    async fn finish_stop(&self, app: &AppHandle, handle: Option<SessionHandle>) -> AppResult<bool> {
        match handle {
            Some(h) => {
                let res = h.stop().await;
                let _ = SessionChanged(None).emit(app);
                res.map(|()| true)
            }
            None => Ok(false),
        }
    }

    /// 401 from Cumulocity: drop the login and stop the session. No-op if the current login
    /// belongs to a different tenant (stale client) or nobody is logged in. Returns `true` only
    /// if this call cleared the login (the caller emits `auth-expired` only then).
    pub async fn expire_auth(&self, app: &AppHandle, tenant_key: &str) -> bool {
        {
            let mut auth = self.auth.write().await;
            if auth
                .as_ref()
                .is_none_or(|a| a.user.tenant.key != tenant_key)
            {
                return false;
            }
            *auth = None;
        }
        if let Err(e) = self.stop_session(app).await {
            log::warn!("stopping session after auth expiry failed: {e}");
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::c8y::fake::FakeC8y;
    use crate::library::Library;

    fn state() -> AppState {
        AppState::new(Ok(Library::default()))
    }

    /// Finding 17: a start in progress keeps a second one out without holding the session lock.
    #[tokio::test]
    async fn only_one_start_at_a_time() {
        let state = state();
        let first = state.claim_start("100").unwrap();
        assert!(matches!(
            state.claim_start("200"),
            Err(AppError::SessionActive(d)) if d == "100"
        ));
        // The session lock is free while a start prepares.
        assert!(state.session.try_lock().is_ok());
        drop(first);
        assert!(state.claim_start("200").is_ok());
    }

    /// F18: a start waits for a stop that is still draining.
    #[tokio::test]
    async fn a_start_waits_for_a_running_stop() {
        let state = Arc::new(state());
        let stopping = state.stop_gate.lock().await;
        let s2 = state.clone();
        let waiter = tokio::spawn(async move { s2.wait_for_stops().await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!waiter.is_finished(), "still draining: the start waits");
        drop(stopping);
        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("released")
            .unwrap();
    }

    /// Finding 32: a logout (or a login as somebody else) while a start prepares is noticed.
    #[tokio::test]
    async fn login_is_rechecked_after_the_start() {
        let fake = FakeC8y::start().await;
        let state = state();
        let client = fake.client();
        assert!(!state.is_current_login(&client).await, "not logged in");
        let user = client.current_user().await.unwrap();
        *state.auth.write().await = Some(Authenticated {
            client: client.clone(),
            user,
        });
        assert!(state.is_current_login(&client).await);
        let other = fake.client_with("other", crate::c8y::fake::PASSWORD);
        assert!(!state.is_current_login(&other).await, "another user");
        *state.auth.write().await = None;
        assert!(!state.is_current_login(&client).await, "logged out");
    }
}
