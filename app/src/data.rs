//! The data folder as a whole (plan.md §10): how much it holds and "Delete everything".

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use crate::engine::Engine;
use crate::keys::KeyVault;

/// Bytes in the folder: database, models and skills.
pub fn storage_used(dir: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| match entry.file_type() {
            Ok(kind) if kind.is_dir() => storage_used(&entry.path()),
            Ok(kind) if kind.is_file() => entry.metadata().map_or(0, |meta| meta.len()),
            _ => 0,
        })
        .sum()
}

/// Stops everything, closes the database, empties the data folder (models included), then
/// deletes the key. The key goes last: a folder that could not be emptied stays readable. A
/// shutdown that fails does not stop the wipe; its error is reported once the data is gone.
pub fn delete_everything(dir: &Path, engine: Option<&Engine>, vault: &dyn KeyVault) -> Result<()> {
    let shutdown = engine.map_or(Ok(()), Engine::shutdown);
    empty_dir(dir)?;
    vault.delete()?;
    shutdown.context("stopping Sextant before the wipe")
}

fn empty_dir(dir: &Path) -> Result<()> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err).context("reading the data folder"),
    };
    for entry in entries {
        let entry = entry.context("reading the data folder")?;
        let path = entry.path();
        // A link is removed, never followed.
        if entry
            .file_type()
            .context("reading the data folder")?
            .is_dir()
        {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        }
        .with_context(|| format!("deleting {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::path::PathBuf;

    use sextant_core::Store;
    use sextant_core::fixtures::{TEST_KEY, TEST_SECRET, WEEK_NOW, load, local, week};

    use super::*;
    use crate::{home, prefs};

    struct FakeVault(Cell<bool>);

    impl KeyVault for FakeVault {
        fn delete(&self) -> Result<()> {
            self.0.set(false);
            Ok(())
        }
    }

    fn temp(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sextant-data-{}-{label}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("test setup");
        dir
    }

    /// A folder as the app leaves it: database with a fixture week, settings, models, a skill.
    fn populated(label: &str) -> PathBuf {
        let dir = temp(label);
        let mut store =
            Store::open(&home::db_path(&dir), &TEST_KEY, &TEST_SECRET).expect("test setup");
        load(&mut store, &week(), WEEK_NOW).expect("test setup");
        store.refresh(local(WEEK_NOW)).expect("test setup");
        drop(store);
        prefs::update(&dir, |s| s.onboarded = true).expect("test setup");
        fs::create_dir_all(dir.join("models")).expect("test setup");
        fs::write(dir.join("models/encoder.onnx"), vec![0u8; 4096]).expect("test setup");
        fs::create_dir_all(dir.join("skills/invoices")).expect("test setup");
        fs::write(dir.join("skills/invoices/SKILL.md"), "# Invoices").expect("test setup");
        dir
    }

    #[test]
    fn delete_everything_leaves_an_empty_folder_and_no_key() {
        let dir = populated("delete");
        let engine = Engine::start(&dir, &TEST_KEY, |_, _| {}).expect("test setup");
        assert!(
            engine
                .read(|store| store.counts())
                .expect("test setup")
                .events
                > 0
        );
        assert!(storage_used(&dir) > 4096);
        let vault = FakeVault(Cell::new(true));

        delete_everything(&dir, Some(&engine), &vault).expect("deletes");

        assert_eq!(fs::read_dir(&dir).expect("the folder stays").count(), 0);
        assert!(!vault.0.get(), "the key is gone");
        assert_eq!(storage_used(&dir), 0);
        assert!(
            engine.read(|store| store.workflows()).is_err(),
            "the database is closed"
        );
        assert!(
            !prefs::load(&dir).expect("defaults").onboarded,
            "onboarding starts again"
        );
        fs::remove_dir_all(dir).expect("cleanup");
    }

    #[test]
    fn a_failed_shutdown_still_wipes_the_folder_and_the_key_then_reports_it() {
        let dir = populated("shutdown-fails");
        let engine = Engine::start(&dir, &TEST_KEY, |_, _| {}).expect("test setup");
        engine.poison_capture_state();
        let vault = FakeVault(Cell::new(true));

        let result = delete_everything(&dir, Some(&engine), &vault);

        assert!(result.is_err(), "the shutdown error is reported");
        assert_eq!(fs::read_dir(&dir).expect("the folder stays").count(), 0);
        assert!(!vault.0.get(), "the key is gone");
        fs::remove_dir_all(dir).expect("cleanup");
    }

    #[test]
    fn a_folder_that_does_not_exist_is_already_empty() {
        let dir = temp("missing");
        fs::remove_dir_all(&dir).expect("test setup");
        let vault = FakeVault(Cell::new(true));
        delete_everything(&dir, None, &vault).expect("deletes");
        assert!(!vault.0.get());
    }
}
