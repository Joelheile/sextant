//! The database key (plan.md §10): 32 random bytes in the macOS Keychain. Debug builds use their
//! own Keychain service, and `SEXTANT_DEV_KEY` (64 hex digits) skips the Keychain for tests.

use anyhow::{Context, Result, bail};
use sextant_core::Key;

/// The existing key, or `None` before the app has run once.
pub fn load() -> Result<Option<Key>> {
    match dev_key()? {
        Some(key) => Ok(Some(key)),
        None => keychain::load(),
    }
}

pub fn load_or_create() -> Result<Key> {
    match load()? {
        Some(key) => Ok(key),
        None => keychain::create(),
    }
}

fn dev_key() -> Result<Option<Key>> {
    if !cfg!(debug_assertions) {
        return Ok(None);
    }
    let Some(hex) = std::env::var_os("SEXTANT_DEV_KEY").filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    let hex = hex.to_string_lossy();
    if hex.len() != 64 || !hex.is_ascii() {
        bail!("SEXTANT_DEV_KEY must be 64 hex digits");
    }
    let mut key = [0u8; 32];
    for (byte, pair) in key.iter_mut().zip(hex.as_bytes().chunks(2)) {
        let pair = std::str::from_utf8(pair).context("SEXTANT_DEV_KEY is not hex")?;
        *byte = u8::from_str_radix(pair, 16).context("SEXTANT_DEV_KEY is not hex")?;
    }
    Ok(Some(key))
}

#[cfg(target_os = "macos")]
mod keychain {
    use anyhow::{Context, Result, anyhow};
    use keyring::{Entry, Error};
    use sextant_core::Key;

    const SERVICE: &str = if cfg!(debug_assertions) {
        "sextant-dev"
    } else {
        "sextant"
    };
    const ACCOUNT: &str = "database-key";

    fn entry() -> Result<Entry> {
        Entry::new(SERVICE, ACCOUNT).context("opening the Keychain entry")
    }

    pub fn load() -> Result<Option<Key>> {
        match entry()?.get_secret() {
            Ok(bytes) => {
                Ok(Some(bytes.try_into().map_err(|_| {
                    anyhow!("the key in the Keychain has the wrong length")
                })?))
            }
            Err(Error::NoEntry) => Ok(None),
            Err(err) => Err(err).context("reading the key from the Keychain"),
        }
    }

    pub fn create() -> Result<Key> {
        let mut key = [0u8; 32];
        getrandom::fill(&mut key).map_err(|err| anyhow!("no random numbers: {err}"))?;
        entry()?
            .set_secret(&key)
            .context("saving the key to the Keychain")?;
        Ok(key)
    }
}

#[cfg(not(target_os = "macos"))]
mod keychain {
    use anyhow::{Result, bail};
    use sextant_core::Key;

    pub fn load() -> Result<Option<Key>> {
        bail!("the key store needs macOS")
    }

    pub fn create() -> Result<Key> {
        bail!("the key store needs macOS")
    }
}
