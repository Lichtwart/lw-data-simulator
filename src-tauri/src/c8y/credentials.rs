//! "Remember me" credential storage in the OS keychain (PRD §5).
//!
//! One keychain entry per tenant: service [`SERVICE`], account = tenant key, secret = the
//! password. The user name lives in a small config file ([`SAVED_USERS_FILE`]) so that listing
//! remembered logins never reads a secret (no keychain prompts).
//! Uses `keyring-core` with the native store per platform (macOS Keychain,
//! Windows Credential Manager); on other platforms storage is unavailable and
//! every call fails with `AppError::Keychain`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult};
use crate::model::SavedLogin;

/// Keychain service name (== bundle identifier).
pub const SERVICE: &str = "io.lichtwart.demo-simulator";

/// File (in the app config dir) that lists which tenants have a remembered login:
/// `{ "<tenantKey>": "<username>" }`. User names are not secret; keeping them out of the
/// keychain means listing them never triggers a keychain prompt.
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

fn read_users(config_dir: &Path) -> BTreeMap<String, String> {
    std::fs::read_to_string(users_path(config_dir))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn write_users(config_dir: &Path, users: &BTreeMap<String, String>) -> AppResult<()> {
    std::fs::create_dir_all(config_dir)?;
    // Write a temp file, then rename: a crash halfway never leaves a half-written list.
    let path = users_path(config_dir);
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(users)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// Remembered logins (user names only) for the given tenant keys. Reads the config file only —
/// never the keychain. Unreadable file = empty list.
pub fn saved_logins(config_dir: &Path, tenant_keys: &[String]) -> Vec<SavedLogin> {
    let users = read_users(config_dir);
    tenant_keys
        .iter()
        .filter_map(|k| {
            users.get(k).map(|u| SavedLogin {
                tenant_key: k.clone(),
                username: u.clone(),
            })
        })
        .collect()
}

/// User name remembered for a tenant, if any.
pub fn saved_username(config_dir: &Path, tenant_key: &str) -> Option<String> {
    read_users(config_dir).remove(tenant_key)
}

/// Remember a login: password into the keychain (account = tenant key), user name into the
/// config file. If the keychain write fails nothing is recorded.
pub fn save(config_dir: &Path, tenant_key: &str, username: &str, password: &str) -> AppResult<()> {
    entry(tenant_key)?.set_password(password).map_err(map_err)?;
    let mut users = read_users(config_dir);
    users.insert(tenant_key.to_owned(), username.to_owned());
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

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lw-sim-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn lists_users_without_keychain() {
        let dir = temp_dir("users");
        let keys = vec!["test".to_owned(), "production".to_owned()];
        assert!(saved_logins(&dir, &keys).is_empty());

        let mut users = BTreeMap::new();
        users.insert("test".to_owned(), "sl".to_owned());
        users.insert("gone".to_owned(), "x".to_owned());
        write_users(&dir, &users).unwrap();

        let list = saved_logins(&dir, &keys);
        assert_eq!(list.len(), 1);
        assert_eq!(
            (list[0].tenant_key.as_str(), list[0].username.as_str()),
            ("test", "sl")
        );
        assert_eq!(saved_username(&dir, "test").as_deref(), Some("sl"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_is_atomic_and_leaves_no_temp_file() {
        let dir = temp_dir("atomic");
        let mut users = BTreeMap::new();
        users.insert("test".to_owned(), "sl".to_owned());
        write_users(&dir, &users).unwrap();
        users.insert("prod".to_owned(), "x".to_owned());
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
        assert!(saved_logins(&dir, &["test".to_owned()]).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
