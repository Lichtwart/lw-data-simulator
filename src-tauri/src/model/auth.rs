//! Tenants, login and credential types (PRD §5).

use serde::{Deserialize, Serialize};
use specta::Type;

/// The Cumulocity tenant a login goes to. Entered by the user (tenant id + URL); nothing
/// tenant-specific is bundled with the app.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Tenant {
    /// Stable key used in commands and keychain entries: `<tenantId>@<host>`.
    pub key: String,
    /// Cumulocity tenant id, e.g. `t12345`.
    pub tenant_id: String,
    /// Base URL without path or trailing slash, e.g. `https://example.cumulocity.com`.
    pub base_url: String,
}

/// Arguments of the `login` command. `Debug` is written by hand: it must never print the password.
#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LoginRequest {
    /// Cumulocity tenant id as entered.
    pub tenant_id: String,
    /// Tenant URL as entered (`https://` is added when missing; a path is ignored).
    pub base_url: String,
    /// Plain user name; the backend prefixes `tenantId/` when needed.
    pub username: String,
    pub password: String,
    /// Store the credentials in the OS keychain for this tenant.
    pub remember: bool,
}

impl std::fmt::Debug for LoginRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoginRequest")
            .field("tenant_id", &self.tenant_id)
            .field("base_url", &self.base_url)
            .field("username", &self.username)
            .field("password", &"****")
            .field("remember", &self.remember)
            .finish()
    }
}

/// The logged-in user. Returned by `login` / `login_saved` / `current_auth`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AuthState {
    pub tenant: Tenant,
    pub username: String,
    /// `id` from `GET /user/currentUser`.
    pub user_id: String,
    pub display_name: Option<String>,
}

/// A credential stored in the keychain (the password never leaves Rust).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SavedLogin {
    pub tenant: Tenant,
    pub username: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_request_debug_hides_the_password() {
        let req = LoginRequest {
            tenant_id: "t1".into(),
            base_url: "https://example.cumulocity.com".into(),
            username: "sl".into(),
            password: "s3cret-pw".into(),
            remember: false,
        };
        let dump = format!("{req:?}");
        assert!(!dump.contains("s3cret-pw"));
        assert!(dump.contains("****") && dump.contains("sl"));
    }
}
