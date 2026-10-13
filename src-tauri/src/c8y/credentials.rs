//! "Remember me" credential storage in the OS keychain (PRD §5).
//!
//! One keychain entry per tenant: service [`SERVICE`], account = tenant key
//! (`<tenantId>@<host>`), secret = the password. Tenant id, URL and user name live in a small
//! config file ([`SAVED_USERS_FILE`]) so that listing remembered logins never reads a secret
//! (no keychain prompts). Nothing tenant-specific ships with the app: the saved logins are the
//! tenant list.
//! Uses `keyring-core` with the native store per platform (macOS Keychain,
//! Windows Credential Manager); on other platforms storage is unavailable and
//! every call fails with `AppError::Keychain`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult};
use serde_json::Value;

use crate::model::SavedLogin;

/// Keychain service name (== bundle identifier).
pub const SERVICE: &str = "io.lichtwart.demo-simulator";

/// File (in the app config dir) that lists the remembered logins:
/// `{ "<tenantKey>": { "tenant": { "key", "tenantId", "baseUrl" }, "username" } }`. None of it
/// is secret; keeping it out of the keychain means listing it never triggers a keychain prompt.
pub const SAVED_USERS_FILE: &str = "saved-logins.json";

/// Install the platform credential store as keyring-core default.
/// Called once from app setup; failure is logged, not fatal.
pub fn init_store() -> AppResult<()> {
    #[cfg(target_os = "macos")]
    {
        let store = apple_native_keyring_store::keychain::Store::new().map_err(map_err)?;
        keyring_core::set_default_store(store);
        Ok(())
    }
    #[cfg(target_os = "windows")]
    {
        let store = windows_native_keyring_store::Store::new().map_err(map_err)?;
        keyring_core::set_default_store(store);
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Err(AppError::Keychain("unsupported platform".into()))
    }
}

fn map_err(e: keyring_core::Error) -> AppError {
    AppError::Keychain(e.to_string())
}

fn entry(tenant_key: &str) -> AppResult<keyring_core::Entry> {
    keyring_core::Entry::new(SERVICE, tenant_key).map_err(map_err)
}

fn users_path(config_dir: &Path) -> PathBuf {
    config_dir.join(SAVED_USERS_FILE)
}

/// Entries that do not parse are left out (the 0.1 format, `{ "<key>": "<username>" }`, named
/// tenants that no longer exist in the app).
fn read_users(config_dir: &Path) -> BTreeMap<String, SavedLogin> {
    std::fs::read_to_string(users_path(config_dir))
        .ok()
        .and_then(|t| serde_json::from_str::<BTreeMap<String, Value>>(&t).ok())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(k, v)| {
            let login: SavedLogin = serde_json::from_value(v).ok()?;
            (login.tenant.key == k).then_some((k, login))
        })
        .collect()
}

fn write_users(config_dir: &Path, users: &BTreeMap<String, SavedLogin>) -> AppResult<()> {
    std::fs::create_dir_all(config_dir)?;
    // Write a temp file, then rename: a crash halfway never leaves a half-written list.
    let path = users_path(config_dir);
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(users)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// Remembered logins (no secrets), sorted by tenant key. Reads the config file only — never
/// the keychain. Unreadable file = empty list.
pub fn saved_logins(config_dir: &Path) -> Vec<SavedLogin> {
    read_users(config_dir).into_values().collect()
}

/// The login remembered for a tenant key, if any.
pub fn saved_login(config_dir: &Path, tenant_key: &str) -> Option<SavedLogin> {
    read_users(config_dir).remove(tenant_key)
}

/// Remember a login: password into the keychain (account = tenant key), tenant and user name
/// into the config file. If the keychain write fails nothing is recorded.
pub fn save(config_dir: &Path, login: &SavedLogin, password: &str) -> AppResult<()> {
    entry(&login.tenant.key)?
        .set_password(password)
        .map_err(map_err)?;
    let mut users = read_users(config_dir);
    users.insert(login.tenant.key.clone(), login.clone());
    write_users(config_dir, &users)
}

/// Read the stored password — the only place that touches the keychain secret
/// (used by `login_saved`). `Ok(None)` if nothing is stored.
pub fn load_password(tenant_key: &str) -> AppResult<Option<String>> {
    match entry(tenant_key)?.get_password() {
        Ok(p) => Ok(Some(p)),
        Err(keyring_core::Error::NoEntry) => Ok(None),
        Err(e) => Err(map_err(e)),
    }
}

/// Remove the remembered login of a tenant (keychain secret + user name; no error if absent).
pub fn delete(config_dir: &Path, tenant_key: &str) -> AppResult<()> {
    // Secret first: if the keychain fails the user name stays listed, so the user can retry
    // (the other order would leave an orphaned secret nobody can see).
    match entry(tenant_key)?.delete_credential() {
        Ok(()) | Err(keyring_core::Error::NoEntry) => {}
        Err(e) => return Err(map_err(e)),
    }
    let mut users = read_users(config_dir);
    if users.remove(tenant_key).is_some() {
        write_users(config_dir, &users)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Tenant;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lw-sim-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn login(tenant_id: &str, host: &str, user: &str) -> SavedLogin {
        SavedLogin {
            tenant: Tenant {
                key: format!("{tenant_id}@{host}"),
                tenant_id: tenant_id.into(),
                base_url: format!("https://{host}"),
            },
            username: user.into(),
        }
    }

    #[test]
    fn lists_logins_without_keychain() {
        let dir = temp_dir("users");
        assert!(saved_logins(&dir).is_empty());

        let a = login("t1", "a.example.com", "sl");
        let b = login("t2", "b.example.com", "demo");
        let users = BTreeMap::from([
            (b.tenant.key.clone(), b.clone()),
            (a.tenant.key.clone(), a.clone()),
        ]);
        write_users(&dir, &users).unwrap();

        assert_eq!(saved_logins(&dir), vec![a.clone(), b]);
        assert_eq!(saved_login(&dir, "t1@a.example.com"), Some(a));
        assert_eq!(saved_login(&dir, "t9@a.example.com"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn old_format_entries_are_skipped() {
        // 0.1 stored `{ "<tenantKey>": "<username>" }` for tenants bundled with the app.
        let dir = temp_dir("legacy");
        std::fs::create_dir_all(&dir).unwrap();
        let ok = login("t1", "a.example.com", "sl");
        let mut file = serde_json::json!({ "test": "sl", "production": "x" });
        file[&ok.tenant.key] = serde_json::to_value(&ok).unwrap();
        // An entry stored under a key that does not match its tenant is not trusted either.
        file["t2@b.example.com"] = serde_json::to_value(&ok).unwrap();
        std::fs::write(users_path(&dir), file.to_string()).unwrap();
        assert_eq!(saved_logins(&dir), vec![ok]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_is_atomic_and_leaves_no_temp_file() {
        let dir = temp_dir("atomic");
        let a = login("t1", "a.example.com", "sl");
        let mut users = BTreeMap::from([(a.tenant.key.clone(), a)]);
        write_users(&dir, &users).unwrap();
        let b = login("t2", "b.example.com", "x");
        users.insert(b.tenant.key.clone(), b);
        write_users(&dir, &users).unwrap();
        assert_eq!(read_users(&dir), users);
        assert!(!dir.join("saved-logins.json.tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_file_is_an_empty_list() {
        let dir = temp_dir("corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(users_path(&dir), "not json").unwrap();
        assert!(saved_logins(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
