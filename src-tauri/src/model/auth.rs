//! Tenants, login and credential types (PRD §5).

use serde::{Deserialize, Serialize};
use specta::Type;

use super::library::LocalizedText;

/// A tenant baked into the app via `library/tenants.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Tenant {
    /// Stable key used in commands and keychain entries, e.g. `test`, `production`.
    pub key: String,
    /// Display label.
    pub label: LocalizedText,
    /// Cumulocity tenant id, e.g. `t819292026`.
    pub tenant_id: String,
    /// Base URL without trailing slash, e.g. `https://test.ram.m2m.telekom.com`.
    pub base_url: String,
}

/// `library/tenants.json` file format.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TenantsFile {
    /// Always `lw-tenants`.
    #[serde(rename = "type")]
    pub file_type: String,
    pub schema_version: u32,
    pub tenants: Vec<Tenant>,
}

/// Arguments of the `login` command. `Debug` is written by hand: it must never print the password.
#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LoginRequest {
    pub tenant_key: String,
    /// Plain user name; the backend prefixes `tenantId/` when needed.
    pub username: String,
    pub password: String,
    /// Store the credentials in the OS keychain for this tenant.
    pub remember: bool,
}

impl std::fmt::Debug for LoginRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoginRequest")
            .field("tenant_key", &self.tenant_key)
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
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SavedLogin {
    pub tenant_key: String,
    pub username: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_request_debug_hides_the_password() {
        let req = LoginRequest {
            tenant_key: "test".into(),
            username: "sl".into(),
            password: "s3cret-pw".into(),
            remember: false,
        };
        let dump = format!("{req:?}");
        assert!(!dump.contains("s3cret-pw"));
        assert!(dump.contains("****") && dump.contains("sl"));
    }
}
