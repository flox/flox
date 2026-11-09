//! Observations of ordinary package authority and the built public tree.
use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::{env, fs};

use flox_core::data::{CanonicalPath, System};
use flox_manifest::Manifest;
use flox_manifest::interfaces::{AsLatestSchema, AsTypedOnlyManifest, PackageLookup};
use flox_manifest::lockfile::{Lockfile, PackageToList};
use flox_manifest::parsed::common::KnownSchemaVersion;
use flox_rust_sdk::models::environment::{DotFlox, EnvironmentPointer, RenderedEnvironmentLinks};
use flox_rust_sdk::providers::buildenv::get_installed_outputs;
use serde::Serialize;
use serde_json::Value;

use super::publication::{self, CommandKey, Directory, State};
use super::{GlobalError, GlobalPaths, GlobalResult};

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Package {
    pub install_id: String,
    pub request: Value,
    pub resolution: Value,
    pub selected_outputs: Vec<PathBuf>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Command {
    pub directory: Directory,
    pub name: String,
    pub published_path: PathBuf,
    pub stable_target: PathBuf,
    pub output_entry: PathBuf,
    pub resolved_target: PathBuf,
    pub provider_install_ids: Vec<String>,
    pub provider_outputs: Vec<PathBuf>,
    pub publication_status: &'static str,
    pub export_policy: &'static str,
}

impl Command {
    pub fn key(&self) -> CommandKey {
        CommandKey {
            directory: self.directory,
            name: self.name.clone(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Snapshot {
    pub run_link: Option<PathBuf>,
    pub observed_output: Option<PathBuf>,
    pub packages: Vec<Package>,
    pub commands: Vec<Command>,
    pub receipts: Vec<ReceiptObservation>,
    pub errors: Vec<GlobalError>,
    #[serde(skip)]
    pub lockfile: Option<Lockfile>,
}

/// Receipts remain visible even when the current output no longer has a name.
#[derive(Clone, Debug, Serialize)]
pub struct ReceiptObservation {
    pub command: CommandKey,
    pub destination: PathBuf,
    pub expected_target: PathBuf,
    pub observed_target: Option<PathBuf>,
    pub status: &'static str,
}

pub fn read_lock(path: &Path) -> GlobalResult<Option<Lockfile>> {
    match fs::metadata(path) {
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(GlobalError::at("environment_failed", path, error)),
        Ok(_) => {},
    }
    let canonical = CanonicalPath::new(path)
        .map_err(|error| GlobalError::at("environment_failed", path, error))?;
    Lockfile::read_from_file(&canonical)
        .map(Some)
        .map_err(|error| GlobalError::at("environment_failed", path, error))
}

pub fn run_link(paths: &GlobalPaths, system: &System) -> GlobalResult<Option<PathBuf>> {
    let dot_flox = paths.environment_dir().join(".flox");
    match fs::symlink_metadata(&dot_flox) {
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(GlobalError::at("environment_failed", &dot_flox, error)),
        Ok(_) => {},
    }
    let dot = DotFlox::open_in(paths.environment_dir())
        .map_err(|error| GlobalError::at("environment_failed", &dot_flox, error))?;
    let EnvironmentPointer::Path(pointer) = dot.pointer else {
        return Err(GlobalError::at(
            "unsupported_environment",
            &dot_flox,
            "Global remote association is not supported by this command yet",
        ));
    };
    let base = CanonicalPath::new(dot.path.join("run"))
        .map_err(|error| GlobalError::at("output_unavailable", &dot_flox, error))?;
    Ok(Some(
        RenderedEnvironmentLinks::new_in_base_dir_with_name_and_system(
            &base,
            pointer.name.as_ref(),
            system,
        )
        .run
        .as_path()
        .to_owned(),
    ))
}

pub fn packages(lock: &Lockfile, system: &System) -> GlobalResult<Vec<Package>> {
    let manifest = lock
        .migrated_manifest()
        .map_err(|error| GlobalError::new("environment_failed", error, None))?;
    lock.list_packages(system)
        .map_err(|error| GlobalError::new("environment_failed", error, None))?
        .into_iter()
        .map(|package| {
            let mut selected = get_installed_outputs(&package)
                .map_err(|error| GlobalError::new("environment_failed", error, None))?;
            // Legacy environments also expose non-default outputs (except stubs).
            // Keep this observation local; the ordinary builder is unchanged.
            if lock.manifest_schema_version() == KnownSchemaVersion::V1 {
                selected = match &package {
                    PackageToList::Catalog(_, locked) => locked
                        .outputs
                        .keys()
                        .filter(|name| *name != "stubs")
                        .cloned()
                        .collect(),
                    PackageToList::Flake(_, locked) => locked
                        .locked_installable
                        .outputs
                        .keys()
                        .filter(|name| *name != "stubs")
                        .cloned()
                        .collect(),
                    PackageToList::StorePath(_) => selected,
                };
            }
            let (id, resolution, outputs) = match &package {
                PackageToList::Catalog(_, locked) => (
                    locked.install_id.clone(),
                    serde_json::to_value(locked),
                    selected
                        .iter()
                        .filter_map(|name| locked.outputs.get(name))
                        .map(PathBuf::from)
                        .collect(),
                ),
                PackageToList::Flake(_, locked) => (
                    locked.install_id.clone(),
                    serde_json::to_value(locked),
                    selected
                        .iter()
                        .filter_map(|name| locked.locked_installable.outputs.get(name))
                        .map(PathBuf::from)
                        .collect(),
                ),
                PackageToList::StorePath(locked) => (
                    locked.install_id.clone(),
                    serde_json::to_value(locked),
                    vec![PathBuf::from(&locked.store_path)],
                ),
            };
            let request =
                serde_json::to_value(manifest.as_latest_schema().pkg_descriptor_with_id(&id))
                    .map_err(|error| GlobalError::new("environment_failed", error, None))?;
            Ok(Package {
                install_id: id,
                request,
                resolution: resolution
                    .map_err(|error| GlobalError::new("environment_failed", error, None))?,
                selected_outputs: outputs,
            })
        })
        .collect()
}

pub fn read(paths: &GlobalPaths, system: &System) -> GlobalResult<Snapshot> {
    let run = run_link(paths, system);
    let mut snapshot = read_at(
        paths,
        &paths.manifest_dir(),
        run.as_ref().ok().and_then(Option::as_deref),
        system,
    )?;
    if let Err(error) = run {
        snapshot.errors.push(error);
    }
    Ok(snapshot)
}

pub fn read_at(
    paths: &GlobalPaths,
    manifest_dir: &Path,
    run_link: Option<&Path>,
    system: &System,
) -> GlobalResult<Snapshot> {
    let mut snapshot = Snapshot {
        run_link: run_link.map(Path::to_owned),
        ..Snapshot::default()
    };
    snapshot.lockfile = read_lock(&manifest_dir.join("manifest.lock"))?;
    if let Some(lock) = &snapshot.lockfile {
        snapshot.packages = packages(lock, system)?;
        let manifest = Manifest::read_typed(manifest_dir.join("manifest.toml"))
            .map_err(|error| GlobalError::at("environment_failed", manifest_dir, error))?;
        if !lock.is_up_to_date_with_serialized_manifest(&manifest.as_typed_only()) {
            snapshot.errors.push(GlobalError::at("output_inconsistent", manifest_dir, "Ordinary manifest and lockfile disagree; recover the ordinary environment before retrying publication"));
        }
    }
    let Some(run_link) = run_link else {
        return Ok(snapshot);
    };
    let output = match fs::canonicalize(run_link) {
        Ok(output) => output,
        Err(error) => {
            snapshot
                .errors
                .push(GlobalError::at("output_unavailable", run_link, error));
            return Ok(snapshot);
        },
    };
    snapshot.observed_output = Some(output.clone());
    let embedded = read_lock(&output.join("manifest.lock"))?;
    if embedded.is_none() || embedded != snapshot.lockfile {
        snapshot.errors.push(GlobalError::at("output_inconsistent", run_link, "Current built output and ordinary lockfile disagree; publication cannot recover ordinary metadata"));
    }
    // Attribute from what was actually built even when ordinary metadata differs.
    let output_packages = embedded
        .as_ref()
        .map(|lock| packages(lock, system))
        .transpose()?
        .unwrap_or_default();
    for directory in Directory::ALL {
        let public = output.join(directory.name());
        let entries = match fs::read_dir(&public) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => return Err(GlobalError::at("output_unavailable", &public, error)),
        };
        for entry in entries {
            let entry =
                entry.map_err(|error| GlobalError::at("output_unavailable", &public, error))?;
            let name = entry.file_name().into_string().map_err(|_| {
                GlobalError::at(
                    "unsupported_path",
                    &entry.path(),
                    "Command name is not valid UTF-8",
                )
            })?;
            let key = CommandKey::new(directory, name)?;
            let metadata = fs::metadata(entry.path())
                .map_err(|error| GlobalError::at("output_unavailable", &entry.path(), error))?;
            if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
                continue;
            }
            let resolved_target = fs::canonicalize(entry.path())
                .map_err(|error| GlobalError::at("output_unavailable", &entry.path(), error))?;
            let mut providers = BTreeMap::new();
            for package in &output_packages {
                for output in &package.selected_outputs {
                    // Inspect only the selected output's corresponding public name.
                    // This is attribution, not transitive dependency ownership.
                    if fs::canonicalize(key.in_directory(output)).ok().as_ref()
                        == Some(&resolved_target)
                    {
                        providers.insert((package.install_id.clone(), output.clone()), ());
                    }
                }
            }
            snapshot.commands.push(Command {
                directory,
                name: key.name.clone(),
                published_path: key.in_directory(&paths.data_dir),
                stable_target: key.in_directory(run_link),
                output_entry: entry.path(),
                resolved_target,
                provider_install_ids: providers
                    .keys()
                    .map(|(id, _)| id.clone())
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect(),
                provider_outputs: providers.keys().map(|(_, output)| output.clone()).collect(),
                publication_status: "unknown",
                export_policy: "all",
            });
        }
    }
    snapshot.commands.sort_by_key(Command::key);
    if fs::canonicalize(run_link).ok().as_deref() != Some(&output) {
        snapshot.errors.push(GlobalError::at(
            "output_superseded",
            run_link,
            "Ordinary output changed during inspection",
        ));
    }
    Ok(snapshot)
}

pub fn observe_publication(paths: &GlobalPaths, snapshot: &mut Snapshot) {
    let state = match State::read(paths) {
        Ok(state) => state,
        Err(error) => {
            snapshot.errors.push(error);
            return;
        },
    };
    for key in state.keys() {
        let receipt = state.receipt(&key).expect("key comes from state");
        let destination = key.in_directory(&paths.data_dir);
        let (observed_target, status) = match publication::inspect(&destination) {
            Ok(None) => (None, "missing"),
            Ok(Some(observed)) => {
                let status = if &observed == receipt {
                    "matching"
                } else {
                    "foreign"
                };
                (Some(observed.target), status)
            },
            Err(error) if error.category == "ownership_conflict" => (None, "foreign"),
            Err(error) => {
                snapshot.errors.push(error);
                (None, "unknown")
            },
        };
        snapshot.receipts.push(ReceiptObservation {
            command: key,
            destination,
            expected_target: receipt.target.clone(),
            observed_target,
            status,
        });
    }
    for command in &mut snapshot.commands {
        command.publication_status = match publication::inspect(&command.published_path) {
            Ok(None) => "missing",
            Ok(Some(observed))
                if state.receipt(&command.key()) == Some(&observed)
                    && observed.target == command.stable_target =>
            {
                "owned"
            },
            Ok(Some(_)) => "foreign",
            Err(error) if error.category == "ownership_conflict" => "foreign",
            Err(error) => {
                snapshot.errors.push(error);
                "unknown"
            },
        };
    }
}

pub fn path_lookup(command: &str) -> Option<PathBuf> {
    if command.contains('/') {
        return None;
    }
    env::var_os("PATH")
        .into_iter()
        .flat_map(|value| env::split_paths(&value).collect::<Vec<_>>())
        .map(|directory| directory.join(command))
        .find(|path| {
            fs::metadata(path).is_ok_and(|metadata| {
                metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
            })
        })
}
