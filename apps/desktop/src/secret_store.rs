//! Platform secrets never cross IPC. Unsupported platforms fail closed.
use anyhow::{Result, bail, ensure};
use cirra_device::SecretStore;
use std::sync::Mutex;
use zeroize::Zeroizing;

const SERVICE: &str = "app.cirra.desktop";

/// Serialize operations on OS credentials, including writes and deletion.
#[derive(Default)]
pub struct OsSecretStore {
    access: Mutex<()>,
}

fn validate_name(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty()
            && name.len() <= 200
            && name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_./".contains(&c)),
        "Invalid credential name"
    );
    Ok(())
}

#[cfg(target_os = "macos")]
impl SecretStore for OsSecretStore {
    fn get(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
        validate_name(name)?;
        let _guard = self
            .access
            .lock()
            .map_err(|_| anyhow::anyhow!("Credential store unavailable"))?;
        let entry = keyring::Entry::new(SERVICE, name)
            .map_err(|_| anyhow::anyhow!("Credential store unavailable"))?;
        match entry.get_secret() {
            Ok(secret) => Ok(Some(Zeroizing::new(secret))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => bail!("Unable to read the device credential from macOS Keychain"),
        }
    }

    fn set(&self, name: &str, secret: &[u8]) -> Result<()> {
        validate_name(name)?;
        ensure!(!secret.is_empty(), "Empty device credential rejected");
        let _guard = self
            .access
            .lock()
            .map_err(|_| anyhow::anyhow!("Credential store unavailable"))?;
        keyring::Entry::new(SERVICE, name)
            .and_then(|entry| entry.set_secret(secret))
            .map_err(|_| anyhow::anyhow!("Unable to save the device credential in macOS Keychain"))
    }

    fn delete(&self, name: &str) -> Result<()> {
        validate_name(name)?;
        let _guard = self
            .access
            .lock()
            .map_err(|_| anyhow::anyhow!("Credential store unavailable"))?;
        match keyring::Entry::new(SERVICE, name).and_then(|entry| entry.delete_credential()) {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => bail!("Unable to remove the device credential from macOS Keychain"),
        }
    }
}

#[cfg(not(target_os = "macos"))]
impl SecretStore for OsSecretStore {
    fn get(&self, _name: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
        bail!("This desktop build requires macOS Keychain")
    }
    fn set(&self, _name: &str, _secret: &[u8]) -> Result<()> {
        bail!("This desktop build requires macOS Keychain")
    }
    fn delete(&self, _name: &str) -> Result<()> {
        bail!("This desktop build requires macOS Keychain")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reject_names_before_accessing_os_credentials() {
        let store = OsSecretStore::default();
        for name in ["", "secret\nname", "secret\0name", "💬", &"a".repeat(201)] {
            assert!(store.get(name).is_err());
            assert!(store.set(name, &[1; 32]).is_err());
            assert!(store.delete(name).is_err());
        }
    }

    #[test]
    fn reject_empty_secret_without_creating_credential() {
        assert!(
            OsSecretStore::default()
                .set("unit-test-unused", &[])
                .is_err()
        );
    }

    /// Opt in explicitly; creates then removes one uniquely named real Keychain item.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires unlocked macOS Keychain and may prompt for access"]
    fn keychain_roundtrip() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let name = format!(
            "test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let store = OsSecretStore::default();
        assert!(store.get(&name).unwrap().is_none());
        store.set(&name, &[7; 32]).unwrap();
        assert_eq!(&**store.get(&name).unwrap().unwrap(), &[7; 32]);
        store.delete(&name).unwrap();
        assert!(store.get(&name).unwrap().is_none());
    }
}
