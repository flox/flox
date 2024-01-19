use std::path::PathBuf;

use anyhow::Result;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::lockfile::LockedManifest;
use flox_rust_sdk::models::search::{PathOrJson, Query, SearchParams};
use log::debug;

use crate::commands::detect_environment;
use crate::config::features::Features;

/// Return an optional manifest and a lockfile to use for search and show.
///
/// This searches for an environment to use,
/// and if one is found, it returns the path to its manifest and optionally the
/// path to its lockfile.
///
/// If no environment is found, or if environment does not have a lockfile, the
/// global lockfile is used.
/// The global lockfile is created if it does not exist.
///
/// Note that this may perform network operations to pull a
/// [ManagedEnvironment],
/// since a freshly cloned user repo with a [ManagedEnvironment] may not have a
/// manifest or lockfile in floxmeta unless the environment is initialized.
pub fn manifest_and_lockfile(flox: &Flox, message: &str) -> Result<(Option<PathBuf>, PathBuf)> {
    let (manifest_path, lockfile_path) = match detect_environment(message)? {
        None => {
            debug!("no environment found");
            (None, None)
        },
        Some(uninitialized) => {
            debug!("using environment {uninitialized}");

            let environment = uninitialized
                .into_concrete_environment(flox)?
                .into_dyn_environment();

            let lockfile_path = environment.lockfile_path(flox)?;
            debug!("checking lockfile: path={}", lockfile_path.display());
            let lockfile = if lockfile_path.exists() {
                debug!("lockfile exists");
                Some(lockfile_path)
            } else {
                debug!("lockfile doesn't exist");
                None
            };
            (Some(environment.manifest_path(flox)?), lockfile)
        },
    };

    // Use the global lock if we don't have a lock yet
    let lockfile_path = match lockfile_path {
        Some(lockfile_path) => lockfile_path,
        None => LockedManifest::ensure_global_lockfile(flox)?,
    };
    Ok((manifest_path, lockfile_path))
}

/// Create [SearchParams] from the given search term
/// using available manifests and lockfiles for resolution.
pub(crate) fn construct_search_params(
    search_term: &str,
    results_limit: Option<u8>,
    manifest: Option<PathOrJson>,
    global_manifest: PathOrJson,
    lockfile: PathOrJson,
) -> Result<SearchParams> {
    let query = Query::from_term_and_limit(
        search_term,
        Features::parse()?.search_strategy,
        results_limit,
    )?;
    let params = SearchParams {
        manifest,
        global_manifest,
        lockfile,
        query,
    };
    debug!("search params raw: {:?}", params);
    Ok(params)
}
