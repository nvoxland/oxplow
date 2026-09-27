//! Provider API keys. They live in the OS keychain (service
//! `net.voxland.oxplow`, account = provider id), never in `ai.yaml` or the
//! project. Behind a trait so tests never touch the real keychain.

use std::collections::HashMap;
use std::sync::Mutex;

/// Keychain service name for every oxplow secret.
pub const KEYCHAIN_SERVICE: &str = "net.voxland.oxplow";

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum SecretError {
    #[error("keychain unavailable: {0}")]
    Unavailable(String),
    #[error("keychain: {0}")]
    Other(String),
}

pub trait SecretStore: Send + Sync {
    /// The secret stored under `name`, if any.
    fn get(&self, name: &str) -> Result<Option<String>, SecretError>;
    fn set(&self, name: &str, value: &str) -> Result<(), SecretError>;
    /// Remove it; removing a missing secret is fine.
    fn delete(&self, name: &str) -> Result<(), SecretError>;
}

/// In-process store, for tests.
#[derive(Default)]
pub struct MemorySecrets(Mutex<HashMap<String, String>>);

impl SecretStore for MemorySecrets {
    fn get(&self, name: &str) -> Result<Option<String>, SecretError> {
        Ok(self.lock().get(name).cloned())
    }
    fn set(&self, name: &str, value: &str) -> Result<(), SecretError> {
        self.lock().insert(name.to_string(), value.to_string());
        Ok(())
    }
    fn delete(&self, name: &str) -> Result<(), SecretError> {
        self.lock().remove(name);
        Ok(())
    }
}

impl MemorySecrets {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The OS keychain (macOS Keychain, Windows Credential Manager, Linux
/// Secret Service). Unsigned dev builds on macOS re-prompt for access
/// after each rebuild; that's the OS tying items to the code signature.
pub struct KeychainSecrets;

impl SecretStore for KeychainSecrets {
    fn get(&self, name: &str) -> Result<Option<String>, SecretError> {
        match entry(name)?.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(SecretError::Other(e.to_string())),
        }
    }
    fn set(&self, name: &str, value: &str) -> Result<(), SecretError> {
        entry(name)?
            .set_password(value)
            .map_err(|e| SecretError::Other(e.to_string()))
    }
    fn delete(&self, name: &str) -> Result<(), SecretError> {
        match entry(name)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(SecretError::Other(e.to_string())),
        }
    }
}

fn entry(name: &str) -> Result<keyring::Entry, SecretError> {
    keyring::Entry::new(KEYCHAIN_SERVICE, name).map_err(|e| SecretError::Unavailable(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_round_trips() {
        let s = MemorySecrets::default();
        assert_eq!(s.get("anthropic").unwrap(), None);
        s.set("anthropic", "sk-1").unwrap();
        assert_eq!(s.get("anthropic").unwrap().as_deref(), Some("sk-1"));
        s.delete("anthropic").unwrap();
        s.delete("anthropic").unwrap();
        assert_eq!(s.get("anthropic").unwrap(), None);
    }
}
