//! Provider API keys. They live in the OS keychain (service
//! `net.voxland.oxplow`, account = provider id), never in `ai.yaml` or the
//! project. Behind a trait so tests never touch the real keychain.
//!
//! Reading a secret from the macOS keychain asks the person (a password
//! prompt, once per read unless they Always Allow), so the keychain is read
//! through one process-wide [`CachedSecrets`]: each secret is read at most
//! once per run, and saving or removing one updates what is remembered.
//! Whether one is saved is answered without reading it ([`SecretStore::exists`]),
//! which asks nobody.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

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
    /// Whether a secret is stored under `name`, without reading it where the
    /// store can tell (reading may ask the person).
    fn exists(&self, name: &str) -> Result<bool, SecretError> {
        Ok(self.get(name)?.is_some())
    }
}

/// `inner`, each secret read at most once: what was read, saved or removed
/// is remembered, so a later read asks `inner` nothing. A failed read is not
/// remembered (it may succeed when asked again). Whether a secret exists is
/// answered from what is remembered, else by `inner` without reading it.
pub struct CachedSecrets<S> {
    inner: S,
    known: Mutex<HashMap<String, Option<String>>>,
    /// Held while a secret is read from `inner`: a read that starts while
    /// another waits (on the person's answer to the keychain) waits too,
    /// then finds the answer in `known`, so the person is asked once.
    reading: Mutex<()>,
}

impl<S> CachedSecrets<S> {
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            known: Mutex::new(HashMap::new()),
            reading: Mutex::new(()),
        }
    }

    fn known(&self) -> std::sync::MutexGuard<'_, HashMap<String, Option<String>>> {
        self.known.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl<S: SecretStore> SecretStore for CachedSecrets<S> {
    fn get(&self, name: &str) -> Result<Option<String>, SecretError> {
        if let Some(value) = self.known().get(name) {
            return Ok(value.clone());
        }
        let _reading = self.reading.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(value) = self.known().get(name) {
            return Ok(value.clone());
        }
        let value = self.inner.get(name)?;
        self.known().insert(name.to_string(), value.clone());
        Ok(value)
    }
    fn set(&self, name: &str, value: &str) -> Result<(), SecretError> {
        self.inner.set(name, value)?;
        self.known()
            .insert(name.to_string(), Some(value.to_string()));
        Ok(())
    }
    fn delete(&self, name: &str) -> Result<(), SecretError> {
        self.inner.delete(name)?;
        self.known().insert(name.to_string(), None);
        Ok(())
    }
    fn exists(&self, name: &str) -> Result<bool, SecretError> {
        if let Some(value) = self.known().get(name) {
            return Ok(value.is_some());
        }
        match self.inner.exists(name) {
            Ok(found) => Ok(found),
            // A store that can't tell without a read: read it once, cached.
            Err(_) => Ok(self.get(name)?.is_some()),
        }
    }
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
/// Secret Service), through the process's one [`CachedSecrets`]: every
/// `KeychainSecrets` shares what was read, so one that saves a key is seen
/// by the others. Unsigned dev builds on macOS re-prompt for access after
/// each rebuild; that's the OS tying items to the code signature.
pub struct KeychainSecrets;

static KEYCHAIN: LazyLock<CachedSecrets<OsKeychain>> =
    LazyLock::new(|| CachedSecrets::new(OsKeychain));

impl SecretStore for KeychainSecrets {
    fn get(&self, name: &str) -> Result<Option<String>, SecretError> {
        KEYCHAIN.get(name)
    }
    fn set(&self, name: &str, value: &str) -> Result<(), SecretError> {
        KEYCHAIN.set(name, value)
    }
    fn delete(&self, name: &str) -> Result<(), SecretError> {
        KEYCHAIN.delete(name)
    }
    fn exists(&self, name: &str) -> Result<bool, SecretError> {
        KEYCHAIN.exists(name)
    }
}

/// The keychain itself, read on every call.
struct OsKeychain;

impl SecretStore for OsKeychain {
    /// A search for the item's attributes, which decrypts nothing and so
    /// asks nobody. A store that can't search that way is an error, and the
    /// cache reads the secret instead (once).
    fn exists(&self, name: &str) -> Result<bool, SecretError> {
        entry(name)?;
        let spec = HashMap::from([("service", KEYCHAIN_SERVICE), ("user", name)]);
        match keyring_core::Entry::search(&spec) {
            Ok(found) => Ok(!found.is_empty()),
            Err(keyring_core::Error::NoEntry) => Ok(false),
            Err(error) => {
                tracing::warn!(%error, "the keychain can't say whether a secret is saved without reading it");
                Err(SecretError::Unavailable(error.to_string()))
            }
        }
    }
    /// Each call is a keychain read, which may ask the person: logged (the
    /// name, never the value) so a prompt can be traced to its reader.
    fn get(&self, name: &str) -> Result<Option<String>, SecretError> {
        tracing::info!(secret = name, "reading a secret from the keychain");
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

    /// Counts how often the store under the cache is read.
    #[derive(Default)]
    struct Counting {
        store: MemorySecrets,
        reads: std::sync::atomic::AtomicUsize,
        checks: std::sync::atomic::AtomicUsize,
        /// How long a read takes: a keychain read waits for the person.
        slow: Option<std::time::Duration>,
        /// A store that can't answer presence without a read.
        no_search: bool,
    }

    impl SecretStore for &Counting {
        fn get(&self, name: &str) -> Result<Option<String>, SecretError> {
            self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if let Some(slow) = self.slow {
                std::thread::sleep(slow);
            }
            self.store.get(name)
        }
        fn set(&self, name: &str, value: &str) -> Result<(), SecretError> {
            self.store.set(name, value)
        }
        fn delete(&self, name: &str) -> Result<(), SecretError> {
            self.store.delete(name)
        }
        fn exists(&self, name: &str) -> Result<bool, SecretError> {
            self.checks
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if self.no_search {
                return Err(SecretError::Unavailable("no search".into()));
            }
            Ok(self.store.lock().contains_key(name))
        }
    }

    /// A secret is read from the store once; a save or a removal is
    /// remembered, so no later read asks it; whether one exists asks it
    /// only for what was never read, and never reads.
    #[test]
    fn a_cached_store_reads_each_secret_once() {
        use std::sync::atomic::Ordering::SeqCst;
        let under = Counting::default();
        under.store.set("anthropic", "sk-1").unwrap();
        let cached = CachedSecrets::new(&under);

        assert!(cached.exists("anthropic").unwrap());
        assert!(!cached.exists("openai").unwrap());
        assert_eq!(
            (under.reads.load(SeqCst), under.checks.load(SeqCst)),
            (0, 2)
        );

        for _ in 0..3 {
            assert_eq!(cached.get("anthropic").unwrap().as_deref(), Some("sk-1"));
        }
        assert_eq!(under.reads.load(SeqCst), 1);
        assert!(cached.exists("anthropic").unwrap());
        assert_eq!(under.checks.load(SeqCst), 2, "known now: no check");

        cached.set("anthropic", "sk-2").unwrap();
        assert_eq!(cached.get("anthropic").unwrap().as_deref(), Some("sk-2"));
        cached.delete("anthropic").unwrap();
        assert_eq!(cached.get("anthropic").unwrap(), None);
        assert!(!cached.exists("anthropic").unwrap());
        assert_eq!(
            under.reads.load(SeqCst),
            1,
            "a save and a removal are remembered"
        );
    }

    /// Two reads at once ask the store once: the second waits for the
    /// first's answer, so the person is asked once, not once per reader.
    #[test]
    fn reads_at_once_ask_the_store_once() {
        let under = Counting {
            slow: Some(std::time::Duration::from_millis(100)),
            ..Counting::default()
        };
        under.store.set("anthropic", "sk-1").unwrap();
        let cached = CachedSecrets::new(&under);
        std::thread::scope(|s| {
            for _ in 0..3 {
                s.spawn(|| assert_eq!(cached.get("anthropic").unwrap().as_deref(), Some("sk-1")));
            }
        });
        assert_eq!(under.reads.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    /// A store that can't search answers presence by a read, and that
    /// read is cached like any other: asked twice, it is read once.
    #[test]
    fn presence_without_search_is_one_cached_read() {
        let under = Counting {
            no_search: true,
            ..Counting::default()
        };
        under.store.set("anthropic", "sk-1").unwrap();
        let cached = CachedSecrets::new(&under);
        assert!(cached.exists("anthropic").unwrap());
        assert!(cached.exists("anthropic").unwrap());
        assert_eq!(cached.get("anthropic").unwrap().as_deref(), Some("sk-1"));
        assert_eq!(under.reads.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

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
