//! Ownership of public names, independent of ordinary package transactions.
//!
//! The lock serializes cooperating publishers only. Neither the sequence of
//! links and checkpoints nor inspection followed by unlink is atomic. Ordinary
//! run links may move throughout reconciliation, and unrecorded links are never
//! adopted. The shared atomic writer does not provide power-loss durability.
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::ErrorKind;
use std::os::unix::fs::{MetadataExt, symlink};
use std::path::{Path, PathBuf};

use flox_core::{Version, open_lock_file, write_atomically};
use serde::{Deserialize, Serialize};

use super::{GlobalError, GlobalPaths, GlobalResult};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Directory {
    Bin,
    Sbin,
}

impl Directory {
    pub const ALL: [Self; 2] = [Self::Bin, Self::Sbin];

    pub fn name(self) -> &'static str {
        match self {
            Self::Bin => "bin",
            Self::Sbin => "sbin",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct CommandKey {
    pub directory: Directory,
    pub name: String,
}

impl CommandKey {
    pub fn new(directory: Directory, name: String) -> GlobalResult<Self> {
        if name.is_empty()
            || name == "."
            || name == ".."
            || name.contains('/')
            || name.contains('\0')
        {
            return Err(GlobalError::new(
                "invalid_state",
                "Invalid command name",
                None,
            ));
        }
        Ok(Self { directory, name })
    }

    pub fn in_directory(&self, parent: &Path) -> PathBuf {
        parent.join(self.directory.name()).join(&self.name)
    }
}

/// Decimal strings preserve the full unsigned Unix device/inode range in TOML.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub target: PathBuf,
    device: String,
    inode: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct State {
    version: Version<1>,
    #[serde(default)]
    bin: BTreeMap<String, Receipt>,
    #[serde(default)]
    sbin: BTreeMap<String, Receipt>,
}

impl State {
    fn entries(&self, directory: Directory) -> &BTreeMap<String, Receipt> {
        match directory {
            Directory::Bin => &self.bin,
            Directory::Sbin => &self.sbin,
        }
    }

    fn entries_mut(&mut self, directory: Directory) -> &mut BTreeMap<String, Receipt> {
        match directory {
            Directory::Bin => &mut self.bin,
            Directory::Sbin => &mut self.sbin,
        }
    }

    pub fn receipt(&self, key: &CommandKey) -> Option<&Receipt> {
        self.entries(key.directory).get(&key.name)
    }

    pub fn keys(&self) -> impl Iterator<Item = CommandKey> + '_ {
        Directory::ALL.into_iter().flat_map(|directory| {
            self.entries(directory).keys().map(move |name| CommandKey {
                directory,
                name: name.clone(),
            })
        })
    }

    pub fn is_empty(&self) -> bool {
        self.bin.is_empty() && self.sbin.is_empty()
    }

    pub fn read(paths: &GlobalPaths) -> GlobalResult<Self> {
        let path = paths.state_file();
        check_directory(&paths.publication_dir())?;
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(GlobalError::at("invalid_state", &path, error)),
            Ok(metadata) if !metadata.is_file() => {
                return Err(GlobalError::at(
                    "invalid_state",
                    &path,
                    "Publication state must be a regular file",
                ));
            },
            Ok(_) => {},
        }
        let contents = fs::read_to_string(&path)
            .map_err(|error| GlobalError::at("invalid_state", &path, error))?;
        let state: Self = toml::from_str(&contents)
            .map_err(|error| GlobalError::at("invalid_state", &path, error))?;
        for key in state.keys() {
            CommandKey::new(key.directory, key.name.clone())?;
            let receipt = state.receipt(&key).expect("key comes from state");
            for value in [&receipt.device, &receipt.inode] {
                if !value.parse::<u64>().is_ok_and(|n| n.to_string() == *value) {
                    return Err(GlobalError::at(
                        "invalid_state",
                        &path,
                        "Invalid link identity",
                    ));
                }
            }
            if !receipt.target.is_absolute() || receipt.target.to_str().is_none() {
                return Err(GlobalError::at(
                    "invalid_state",
                    &path,
                    "Invalid link target",
                ));
            }
        }
        Ok(state)
    }

    fn write(&self, paths: &GlobalPaths) -> GlobalResult<()> {
        let path = paths.state_file();
        let contents = toml::to_string(self)
            .map_err(|error| GlobalError::at("publication_io", &path, error))?;
        write_atomically(&path, contents)
            .map_err(|error| GlobalError::at("publication_io", &path, error))
    }
}

pub fn inspect(path: &Path) -> GlobalResult<Option<Receipt>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(GlobalError::at("publication_io", path, error)),
    };
    if !metadata.file_type().is_symlink() {
        return Err(GlobalError::at(
            "ownership_conflict",
            path,
            "Existing entry is not an owned symlink",
        ));
    }
    let target =
        fs::read_link(path).map_err(|error| GlobalError::at("publication_io", path, error))?;
    let after = fs::symlink_metadata(path)
        .map_err(|error| GlobalError::at("publication_io", path, error))?;
    if !after.file_type().is_symlink()
        || (metadata.dev(), metadata.ino()) != (after.dev(), after.ino())
    {
        return Err(GlobalError::at(
            "ownership_conflict",
            path,
            "Entry changed during inspection",
        ));
    }
    Ok(Some(Receipt {
        target,
        device: metadata.dev().to_string(),
        inode: metadata.ino().to_string(),
    }))
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Create,
    Remove,
    Forget,
    Unchanged,
    Conflict,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct EntryPlan {
    pub command: CommandKey,
    pub destination: PathBuf,
    pub target: PathBuf,
    pub observed_target: Option<PathBuf>,
    pub action: Action,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PublicationReport {
    pub status: &'static str,
    pub observed_output: Option<PathBuf>,
    pub actual_output: Option<PathBuf>,
    pub entries: Vec<EntryPlan>,
    pub completed: Vec<PathBuf>,
    pub errors: Vec<GlobalError>,
}

impl PublicationReport {
    pub fn unknown(error: GlobalError) -> Self {
        Self {
            status: "unknown",
            observed_output: None,
            actual_output: None,
            entries: vec![],
            completed: vec![],
            errors: vec![error],
        }
    }
}

/// Reads both receipts and real entries; no target-text-only ownership test.
pub fn plan(
    paths: &GlobalPaths,
    state: &State,
    run_link: &Path,
    desired: impl IntoIterator<Item = CommandKey>,
) -> GlobalResult<Vec<EntryPlan>> {
    for path in [paths.data_dir.as_path(), run_link] {
        if path.to_str().is_none() {
            return Err(GlobalError::new(
                "unsupported_path",
                "Publication paths must be valid UTF-8",
                None,
            ));
        }
    }
    for directory in Directory::ALL {
        check_directory(&paths.data_dir.join(directory.name()))?;
    }
    let mut keys: BTreeMap<CommandKey, bool> = state.keys().map(|key| (key, false)).collect();
    for key in desired {
        keys.insert(key, true);
    }
    keys.into_iter()
        .map(|(key, wanted)| {
            let destination = key.in_directory(&paths.data_dir);
            let target = key.in_directory(run_link);
            let receipt = state.receipt(&key);
            if receipt.is_some_and(|receipt| receipt.target != target) {
                return Err(GlobalError::at(
                    "invalid_state",
                    &paths.state_file(),
                    "Receipt does not target the ordinary global run link",
                ));
            }
            let observed = inspect(&destination);
            let observed_target = observed
                .as_ref()
                .ok()
                .and_then(|entry| entry.as_ref())
                .map(|entry| entry.target.clone());
            let (action, reason) = match observed {
                Ok(None) => (
                    if wanted {
                        Action::Create
                    } else {
                        Action::Forget
                    },
                    None,
                ),
                Ok(Some(observed)) if Some(&observed) == receipt => (
                    if wanted {
                        Action::Unchanged
                    } else {
                        Action::Remove
                    },
                    None,
                ),
                Ok(Some(_)) => (
                    Action::Conflict,
                    Some("Existing link has no matching ownership receipt".into()),
                ),
                Err(error) if error.category == "ownership_conflict" => {
                    (Action::Conflict, Some(error.message))
                },
                Err(error) => return Err(error),
            };
            Ok(EntryPlan {
                command: key,
                destination,
                target,
                observed_target,
                action,
                reason,
            })
        })
        .collect()
}

fn check_directory(path: &Path) -> GlobalResult<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(GlobalError::at(
            "ownership_conflict",
            path,
            "Expected a directory, not a symlink or other entry",
        )),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(GlobalError::at("publication_io", path, error)),
    }
}

fn ensure_directory(path: &Path) -> GlobalResult<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(GlobalError::at(
            "ownership_conflict",
            path,
            "Expected a directory, not a symlink or other entry",
        )),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            fs::create_dir(path).map_err(|error| GlobalError::at("publication_io", path, error))
        },
        Err(error) => Err(GlobalError::at("publication_io", path, error)),
    }
}

pub fn lock(paths: &GlobalPaths) -> GlobalResult<File> {
    ensure_directory(&paths.publication_dir())?;
    let path = paths.publication_dir().join("lock");
    match fs::symlink_metadata(&path) {
        Ok(metadata) if !metadata.is_file() => {
            return Err(GlobalError::at(
                "invalid_state",
                &path,
                "Publication lock must be a regular file",
            ));
        },
        Err(error) if error.kind() != ErrorKind::NotFound => {
            return Err(GlobalError::at("publication_io", &path, error));
        },
        _ => {},
    }
    let lock =
        open_lock_file(&path).map_err(|error| GlobalError::at("publication_io", &path, error))?;
    lock.lock()
        .map_err(|error| GlobalError::at("publication_io", &path, error))?;
    Ok(lock)
}

/// Caller holds the publication lock and has verified the current ordinary output.
/// A checkpoint failure leaves created-but-unrecorded links deliberately unowned.
pub fn apply(
    paths: &GlobalPaths,
    state: &mut State,
    run_link: &Path,
    output: &Path,
    entries: Vec<EntryPlan>,
    dry_run: bool,
) -> PublicationReport {
    apply_with_checkpoint(
        paths,
        state,
        run_link,
        output,
        entries,
        dry_run,
        State::write,
    )
}

fn apply_with_checkpoint(
    paths: &GlobalPaths,
    state: &mut State,
    run_link: &Path,
    output: &Path,
    entries: Vec<EntryPlan>,
    dry_run: bool,
    mut checkpoint: impl FnMut(&State, &GlobalPaths) -> GlobalResult<()>,
) -> PublicationReport {
    let mut report = PublicationReport {
        status: "complete",
        observed_output: Some(output.to_owned()),
        actual_output: fs::canonicalize(run_link).ok(),
        errors: entries
            .iter()
            .filter(|entry| entry.action == Action::Conflict)
            .map(|entry| {
                GlobalError::at(
                    "ownership_conflict",
                    &entry.destination,
                    entry.reason.as_deref().unwrap_or("Ownership is ambiguous"),
                )
            })
            .collect(),
        entries,
        completed: vec![],
    };
    if report.actual_output.as_deref() != Some(output) {
        report.errors.push(GlobalError::at(
            "output_superseded",
            run_link,
            "Ordinary output changed before publication",
        ));
    }
    if !report.errors.is_empty() {
        report.status = "incomplete";
        return report;
    }
    if dry_run {
        return report;
    }
    for entry in &report.entries {
        let result = (|| {
            if fs::canonicalize(run_link).ok().as_deref() != Some(output) {
                return Err(GlobalError::at(
                    "output_superseded",
                    run_link,
                    "Ordinary output changed; retry publication against the current output",
                ));
            }
            let observed = inspect(&entry.destination)?;
            if entry.action == Action::Unchanged {
                if observed.as_ref() != state.receipt(&entry.command) {
                    return Err(GlobalError::at(
                        "ownership_conflict",
                        &entry.destination,
                        "Owned entry changed during reconciliation",
                    ));
                }
                return Ok(());
            }
            match entry.action {
                Action::Create => {
                    if observed.is_some() {
                        return Err(GlobalError::at(
                            "ownership_conflict",
                            &entry.destination,
                            "Entry appeared before creation",
                        ));
                    }
                    ensure_directory(&paths.data_dir.join(entry.command.directory.name()))?;
                    symlink(&entry.target, &entry.destination).map_err(|error| {
                        GlobalError::at("publication_io", &entry.destination, error)
                    })?;
                    let receipt = inspect(&entry.destination)?.ok_or_else(|| {
                        GlobalError::at(
                            "ownership_conflict",
                            &entry.destination,
                            "Created link disappeared",
                        )
                    })?;
                    if receipt.target != entry.target {
                        return Err(GlobalError::at(
                            "ownership_conflict",
                            &entry.destination,
                            "Created link changed before observation",
                        ));
                    }
                    state
                        .entries_mut(entry.command.directory)
                        .insert(entry.command.name.clone(), receipt);
                },
                Action::Remove => {
                    if observed.as_ref() != state.receipt(&entry.command) {
                        return Err(GlobalError::at(
                            "ownership_conflict",
                            &entry.destination,
                            "Entry changed before removal",
                        ));
                    }
                    fs::remove_file(&entry.destination).map_err(|error| {
                        GlobalError::at("publication_io", &entry.destination, error)
                    })?;
                    state
                        .entries_mut(entry.command.directory)
                        .remove(&entry.command.name);
                },
                Action::Forget => {
                    if observed.is_some() {
                        return Err(GlobalError::at(
                            "ownership_conflict",
                            &entry.destination,
                            "Entry appeared before receipt removal",
                        ));
                    }
                    state
                        .entries_mut(entry.command.directory)
                        .remove(&entry.command.name);
                },
                Action::Unchanged | Action::Conflict => unreachable!(),
            }
            // Record the actual entry change even if checkpointing subsequently fails.
            report.completed.push(entry.destination.clone());
            checkpoint(state, paths)
        })();
        if let Err(error) = result {
            report.errors.push(error);
            break;
        }
    }
    report.actual_output = fs::canonicalize(run_link).ok();
    if report.actual_output.as_deref() != Some(output) {
        report.errors.push(GlobalError::at(
            "output_superseded",
            run_link,
            "Ordinary output changed during publication; existing names may already use it",
        ));
    }
    if !report.errors.is_empty() {
        report.status = "incomplete";
    }
    report
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use tempfile::{TempDir, tempdir};

    use super::*;

    struct Fixture {
        _temp: TempDir,
        paths: GlobalPaths,
        run: PathBuf,
        output: PathBuf,
        key: CommandKey,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = tempdir().unwrap();
            let root = fs::canonicalize(temp.path()).unwrap();
            let paths = GlobalPaths {
                data_dir: root.join("data"),
            };
            fs::create_dir(&paths.data_dir).unwrap();
            let output = root.join("output");
            fs::create_dir(&output).unwrap();
            let run = root.join("ordinary-run");
            symlink(&output, &run).unwrap();
            Self {
                _temp: temp,
                paths,
                run,
                output,
                key: CommandKey::new(Directory::Bin, "hello".into()).unwrap(),
            }
        }

        fn publish(&self) -> State {
            let _lock = lock(&self.paths).unwrap();
            let mut state = State::default();
            let entries = plan(&self.paths, &state, &self.run, [self.key.clone()]).unwrap();
            let result = apply(
                &self.paths,
                &mut state,
                &self.run,
                &self.output,
                entries,
                false,
            );
            assert_eq!((result.status, result.errors.len()), ("complete", 0));
            state
        }
    }

    #[test]
    fn invalid_state_never_becomes_empty_ownership() {
        let f = Fixture::new();
        fs::create_dir(f.paths.publication_dir()).unwrap();
        for contents in [
            "not TOML",
            "version = 2",
            "version = 1\nunknown = true",
            "version = 1\n[bin.hello]\ntarget = '/x'\ndevice = '01'\ninode = '1'",
            "version = 1\n[bin.'../sentinel']\ntarget = '/x'\ndevice = '1'\ninode = '1'",
            "version = 1\n[bin.hello]\ntarget = '/x'\ndevice = '1'\ninode = '1'\nunknown = true",
        ] {
            fs::write(f.paths.state_file(), contents).unwrap();
            let result = State::read(&f.paths);
            assert_eq!(
                (
                    result.unwrap_err().category,
                    fs::read_to_string(f.paths.state_file()).unwrap()
                ),
                ("invalid_state", contents.to_owned())
            );
        }
    }

    #[test]
    fn a_receipt_cannot_authorize_a_target_outside_the_ordinary_run_link() {
        let f = Fixture::new();
        fs::create_dir(f.paths.data_dir.join("bin")).unwrap();
        let destination = f.key.in_directory(&f.paths.data_dir);
        symlink("/foreign/hello", &destination).unwrap();
        let observed = inspect(&destination).unwrap().unwrap();
        let mut state = State::default();
        state
            .entries_mut(Directory::Bin)
            .insert(f.key.name.clone(), observed.clone());
        let error = plan(&f.paths, &state, &f.run, []).unwrap_err();
        assert_eq!(
            (error.category, inspect(&destination).unwrap()),
            ("invalid_state", Some(observed))
        );
    }

    #[test]
    fn foreign_file_and_dangling_link_are_preserved() {
        let f = Fixture::new();
        fs::create_dir(f.paths.data_dir.join("bin")).unwrap();
        let destination = f.key.in_directory(&f.paths.data_dir);
        fs::write(&destination, "foreign").unwrap();
        for is_link in [false, true] {
            if is_link {
                fs::remove_file(&destination).unwrap();
                symlink("/missing/foreign", &destination).unwrap();
            }
            let entries = plan(&f.paths, &State::default(), &f.run, [f.key.clone()]).unwrap();
            assert_eq!(
                entries
                    .iter()
                    .map(|entry| entry.action.clone())
                    .collect::<Vec<_>>(),
                vec![Action::Conflict]
            );
            let report = apply(
                &f.paths,
                &mut State::default(),
                &f.run,
                &f.output,
                entries,
                false,
            );
            assert_eq!((report.status, report.completed), ("incomplete", vec![]));
        }
        assert_eq!(
            fs::read_link(destination).unwrap(),
            PathBuf::from("/missing/foreign")
        );
    }

    #[test]
    fn same_target_replacement_is_not_owned() {
        let f = Fixture::new();
        let state = f.publish();
        let replacement = f.paths.data_dir.join("replacement");
        symlink(f.key.in_directory(&f.run), &replacement).unwrap();
        fs::rename(&replacement, f.key.in_directory(&f.paths.data_dir)).unwrap();
        let entries = plan(&f.paths, &state, &f.run, []).unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.action.clone())
                .collect::<Vec<_>>(),
            vec![Action::Conflict]
        );
    }

    #[test]
    fn stable_owned_name_follows_ordinary_run_movement() {
        let f = Fixture::new();
        let mut state = f.publish();
        let old_receipt = inspect(&f.key.in_directory(&f.paths.data_dir)).unwrap();
        let next = f.output.with_file_name("next");
        fs::create_dir(&next).unwrap();
        fs::remove_file(&f.run).unwrap();
        symlink(&next, &f.run).unwrap();
        let entries = plan(&f.paths, &state, &f.run, [f.key.clone()]).unwrap();
        let report = apply(&f.paths, &mut state, &f.run, &next, entries, false);
        assert_eq!(
            (
                report.status,
                report.completed,
                inspect(&f.key.in_directory(&f.paths.data_dir)).unwrap()
            ),
            ("complete", vec![], old_receipt)
        );
    }

    #[test]
    fn checkpoint_failure_does_not_authorize_adopting_created_link() {
        let f = Fixture::new();
        let _lock = lock(&f.paths).unwrap();
        let mut state = State::default();
        let entries = plan(&f.paths, &state, &f.run, [f.key.clone()]).unwrap();
        let report = apply_with_checkpoint(
            &f.paths,
            &mut state,
            &f.run,
            &f.output,
            entries,
            false,
            |_, paths| {
                Err(GlobalError::at(
                    "publication_io",
                    &paths.state_file(),
                    "injected checkpoint failure",
                ))
            },
        );
        let saved = State::read(&f.paths).unwrap();
        let retry = plan(&f.paths, &saved, &f.run, [f.key.clone()]).unwrap();
        assert_eq!(
            (
                report.status,
                saved,
                retry
                    .iter()
                    .map(|entry| entry.action.clone())
                    .collect::<Vec<_>>()
            ),
            ("incomplete", State::default(), vec![Action::Conflict])
        );
    }

    #[test]
    fn missing_obsolete_entry_clears_receipt_after_interrupted_removal() {
        let f = Fixture::new();
        let mut state = f.publish();
        let _lock = lock(&f.paths).unwrap();
        let entries = plan(&f.paths, &state, &f.run, []).unwrap();
        let report = apply_with_checkpoint(
            &f.paths,
            &mut state,
            &f.run,
            &f.output,
            entries,
            false,
            |_, paths| {
                Err(GlobalError::at(
                    "publication_io",
                    &paths.state_file(),
                    "injected checkpoint failure",
                ))
            },
        );
        let mut saved = State::read(&f.paths).unwrap();
        let entries = plan(&f.paths, &saved, &f.run, []).unwrap();
        assert_eq!(
            (
                report.status,
                entries
                    .iter()
                    .map(|entry| entry.action.clone())
                    .collect::<Vec<_>>()
            ),
            ("incomplete", vec![Action::Forget])
        );
        let retry = apply(&f.paths, &mut saved, &f.run, &f.output, entries, false);
        assert_eq!(
            (retry.status, State::read(&f.paths).unwrap()),
            ("complete", State::default())
        );
    }

    #[test]
    fn dry_run_preserves_receipts_and_destinations() {
        let f = Fixture::new();
        let mut state = f.publish();
        let bytes = fs::read(f.paths.state_file()).unwrap();
        let receipt = inspect(&f.key.in_directory(&f.paths.data_dir)).unwrap();
        let entries = plan(&f.paths, &state, &f.run, []).unwrap();
        let report = apply(&f.paths, &mut state, &f.run, &f.output, entries, true);
        assert_eq!(
            (
                report.status,
                report.completed,
                fs::read(f.paths.state_file()).unwrap(),
                inspect(&f.key.in_directory(&f.paths.data_dir)).unwrap()
            ),
            ("complete", vec![], bytes, receipt)
        );
    }

    #[test]
    fn bin_and_sbin_names_are_independent() {
        let f = Fixture::new();
        let _lock = lock(&f.paths).unwrap();
        let mut state = State::default();
        let sbin = CommandKey::new(Directory::Sbin, "hello".into()).unwrap();
        let entries = plan(&f.paths, &state, &f.run, [f.key.clone(), sbin.clone()]).unwrap();
        let report = apply(&f.paths, &mut state, &f.run, &f.output, entries, false);
        assert_eq!(
            (report.status, state.keys().collect::<Vec<_>>()),
            ("complete", vec![f.key, sbin])
        );
    }

    #[test]
    fn superseded_output_stops_before_new_names() {
        let f = Fixture::new();
        let _lock = lock(&f.paths).unwrap();
        let mut state = State::default();
        let entries = plan(&f.paths, &state, &f.run, [f.key.clone()]).unwrap();
        fs::remove_file(&f.run).unwrap();
        symlink("/missing/superseded", &f.run).unwrap();
        let report = apply(&f.paths, &mut state, &f.run, &f.output, entries, false);
        assert_eq!(
            (
                report.status,
                report.completed,
                State::read(&f.paths).unwrap()
            ),
            ("incomplete", vec![], State::default())
        );
    }

    #[test]
    fn cooperating_publication_writers_wait_for_the_same_lock() {
        let f = Fixture::new();
        let first = lock(&f.paths).unwrap();
        let paths = f.paths.clone();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (locked_tx, locked_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let probe = open_lock_file(paths.publication_dir().join("lock")).unwrap();
            assert!(
                probe.try_lock().is_err(),
                "the first publisher must hold this lock"
            );
            ready_tx.send(()).unwrap();
            let _second = lock(&paths).unwrap();
            locked_tx.send(()).unwrap();
        });
        ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(
            locked_rx.recv_timeout(Duration::from_millis(50)),
            Err(mpsc::RecvTimeoutError::Timeout)
        );
        drop(first);
        locked_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        worker.join().unwrap();
    }
}
