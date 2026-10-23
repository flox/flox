//! Saved selectors only: no environment resolution, authentication, or builds.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use flox_core::canonical_path::CanonicalPath;
use flox_core::data::environment_ref::RemoteEnvironmentRef;
use flox_core::{open_lock_file, write_atomically};
use flox_rust_sdk::models::environment::DotFlox;
use serde::{Deserialize, Serialize};

use super::DefaultsError;
use crate::commands::EnvironmentSelect;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StartupSelector {
    Remote { reference: RemoteEnvironmentRef },
    Local { path: PathBuf },
}

impl StartupSelector {
    pub fn from_input(
        references: &[String],
        directories: &[PathBuf],
    ) -> Result<Self, DefaultsError> {
        match (references, directories) {
            ([reference], []) => {
                let reference = parse_reference(reference)?;
                Ok(Self::Remote { reference })
            },
            ([], [path]) => {
                let path = CanonicalPath::new(path)
                    .map_err(|err| DefaultsError::InvalidSelector(err.to_string()))?
                    .into_inner();
                if path.to_str().is_none() {
                    return Err(DefaultsError::InvalidSelector(
                        "local paths must be valid UTF-8".into(),
                    ));
                }
                // Metadata only, at exactly this directory: no ancestor search,
                // migration, concrete environment, lockfile, or source build.
                DotFlox::open_in(&path)
                    .map_err(|err| DefaultsError::InvalidSelector(err.to_string()))?;
                Ok(Self::Local { path })
            },
            ([], []) => Err(DefaultsError::InvalidSelector(
                "provide OWNER/NAME or --dir PATH".into(),
            )),
            _ => Err(DefaultsError::ConflictingSelectors),
        }
    }

    fn validate(&self) -> Result<(), String> {
        match self {
            Self::Remote { reference } => parse_reference(&reference.to_string())
                .map(|_| ())
                .map_err(|err| err.to_string()),
            Self::Local { path } if path.is_absolute() && path.to_str().is_some() => Ok(()),
            Self::Local { .. } => Err("saved local path must be absolute and valid UTF-8".into()),
        }
    }

    pub fn into_environment(self) -> EnvironmentSelect {
        match self {
            Self::Remote { reference } => EnvironmentSelect::Remote(reference),
            Self::Local { path } => EnvironmentSelect::Dir(path),
        }
    }

    pub fn description(&self) -> String {
        match self {
            Self::Remote { reference } => reference.to_string(),
            Self::Local { path } => path.display().to_string(),
        }
    }
}

fn parse_reference(input: &str) -> Result<RemoteEnvironmentRef, DefaultsError> {
    // The shared parser also accepts empty owner/name components. A saved
    // selection requires both, and explicit path syntax needs --dir.
    if !matches!(input.split_once('/'), Some((owner, name))
        if !owner.is_empty() && !name.is_empty() && !matches!(owner, "." | ".." | "~"))
    {
        return Err(DefaultsError::InvalidSelector(
            "use a full OWNER/NAME reference, or --dir PATH for a local environment".into(),
        ));
    }
    input.parse().map_err(
        |err: flox_core::data::environment_ref::RemoteEnvironmentRefError| {
            DefaultsError::InvalidSelector(err.to_string())
        },
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    selection: Option<StartupSelector>,
}

#[derive(Debug, Clone)]
pub struct SelectionStore {
    path: PathBuf,
    lock_path: PathBuf,
}

impl SelectionStore {
    pub fn new(config_dir: &Path) -> Self {
        Self {
            path: config_dir.join("defaults.toml"),
            lock_path: config_dir.join("defaults.lock"),
        }
    }

    pub fn read(&self) -> Result<Option<StartupSelector>, DefaultsError> {
        let text = match fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(DefaultsError::io(&self.path, err)),
        };
        let invalid = |reason: String| DefaultsError::ConfigInvalid {
            path: self.path.clone(),
            reason,
        };
        let document: Document = toml::from_str(&text).map_err(|err| invalid(err.to_string()))?;
        if document.version != 1 {
            return Err(invalid(format!("unsupported version {}", document.version)));
        }
        if let Some(selection) = &document.selection {
            selection.validate().map_err(invalid)?;
        }
        Ok(document.selection)
    }

    /// Serialize writers through a stable sibling lock. Readers see one whole
    /// old or new document; the lock is never held through activation.
    pub fn save(&self, selection: Option<StartupSelector>) -> Result<bool, DefaultsError> {
        let lock = open_lock_file(&self.lock_path)
            .map_err(|err| DefaultsError::io(&self.lock_path, err))?;
        lock.lock()
            .map_err(|err| DefaultsError::io(&self.lock_path, err))?;
        if self.read()? == selection {
            return Ok(false);
        }
        let text = toml::to_string_pretty(&Document {
            version: 1,
            selection,
        })
        .map_err(|err| DefaultsError::io(&self.path, err))?;
        write_atomically(&self.path, text).map_err(|err| DefaultsError::io(&self.path, err))?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

    #[test]
    fn no_selection_and_idempotent_mutations() {
        let dir = tempfile::tempdir().unwrap();
        let store = SelectionStore::new(dir.path());
        assert_eq!(store.read().unwrap(), None);
        assert!(!store.save(None).unwrap());
        assert!(!store.path.exists());
        let fred = StartupSelector::from_input(&["acme/fred".into()], &[]).unwrap();
        assert!(store.save(Some(fred.clone())).unwrap());
        let before = fs::metadata(&store.path).unwrap().modified().unwrap();
        assert!(!store.save(Some(fred.clone())).unwrap());
        assert_eq!(
            fs::metadata(&store.path).unwrap().modified().unwrap(),
            before
        );
        assert_eq!(store.read().unwrap(), Some(fred));
        assert!(store.save(None).unwrap());
        assert_eq!(store.read().unwrap(), None);
    }

    #[test]
    fn corrupt_configuration_is_not_absence_or_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let store = SelectionStore::new(dir.path());
        for text in [
            "",
            "version = 2",
            "version = 1\n[selection]",
            "version = 1\n[selection]\nkind = 'local'\npath = 'relative'",
        ] {
            fs::write(&store.path, text).unwrap();
            assert!(matches!(
                store.read(),
                Err(DefaultsError::ConfigInvalid { .. })
            ));
            assert!(store.save(None).is_err());
            assert_eq!(fs::read_to_string(&store.path).unwrap(), text);
        }
    }

    #[test]
    fn local_inventory_and_clear_do_not_reopen_source() {
        let dir = tempfile::tempdir().unwrap();
        let store = SelectionStore::new(dir.path());
        let target = StartupSelector::Local {
            path: dir.path().join("deleted source"),
        };
        store.save(Some(target.clone())).unwrap();
        assert_eq!(store.read().unwrap(), Some(target));
        assert!(store.save(None).unwrap());
    }

    #[test]
    fn rejects_ambiguous_and_multiple_selectors() {
        for reference in ["fred", "/fred", "acme/", "./fred", "../fred", "a/b/c"] {
            assert!(StartupSelector::from_input(&[reference.into()], &[]).is_err());
        }
        assert!(matches!(
            StartupSelector::from_input(&["a/b".into(), "a/c".into()], &[]),
            Err(DefaultsError::ConflictingSelectors)
        ));
        assert!(matches!(
            StartupSelector::from_input(&["a/b".into()], &["local".into()]),
            Err(DefaultsError::ConflictingSelectors)
        ));
    }

    #[test]
    fn writers_wait_for_the_sibling_lock() {
        let dir = tempfile::tempdir().unwrap();
        let store = SelectionStore::new(dir.path());
        let lock = open_lock_file(&store.lock_path).unwrap();
        lock.lock().unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            let selector = StartupSelector::from_input(&["acme/fred".into()], &[]).unwrap();
            done_tx.send(store.save(Some(selector))).unwrap();
        });
        started_rx.recv().unwrap();
        assert!(done_rx.recv_timeout(Duration::from_millis(100)).is_err());
        drop(lock);
        assert!(
            done_rx
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .unwrap()
        );
        worker.join().unwrap();
    }
}
