use std::collections::{BTreeMap, HashSet};

use anyhow::{bail, Context, Result};
use bpaf::Bpaf;
use flox_rust_sdk::data::System;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::global_manifest_path;
use flox_rust_sdk::models::search::{
    do_search,
    PathOrJson,
    Query,
    SearchParams,
    SearchResult,
    SearchResults,
    SearchStrategy,
    ShowError,
};
use flox_rust_sdk::providers::catalog::{ClientTrait, VersionsError};
use log::debug;
use tracing::instrument;

use crate::subcommand_metric;
use crate::utils::search::{manifest_and_lockfile, DEFAULT_DESCRIPTION, SEARCH_INPUT_SEPARATOR};

// Show detailed package information
#[derive(Debug, Bpaf, Clone)]
pub struct Show {
    /// The package to show detailed information about. Must be an exact match
    /// for a pkg-path e.g. something copy-pasted from the output of `flox search`.
    #[bpaf(positional("pkg-path"))]
    pub pkg_path: String,
}

impl Show {
    #[instrument(name = "show", fields(pkg_path = self.pkg_path), skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("show");

        if let Some(client) = flox.catalog_client {
            tracing::debug!("using catalog client for show");
            let results = match client.package_versions(&self.pkg_path).await {
                Ok(results) => results,
                // Below, results.is_empty() is used to mean the search_term
                // didn't match a package.
                // So translate 404 into an empty vec![].
                // Once we drop the pkgdb code path, we can clean this up.
                Err(VersionsError::Versions(e)) if e.status() == 404 => SearchResults {
                    results: vec![],
                    count: None::<u64>,
                },
                Err(e) => Err(e)?,
            };
            if results.results.is_empty() {
                bail!("no packages matched this pkg-path: '{}'", self.pkg_path);
            }
            let expected_systems = [
                "aarch64-darwin",
                "aarch64-linux",
                "x86_64-darwin",
                "x86_64-linux",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect::<HashSet<_>>();
            render_show_catalog(&results.results, &expected_systems)?;
        } else {
            tracing::debug!("using pkgdb for show");

            let (manifest, lockfile) = manifest_and_lockfile(&flox, "Show using")
                .context("failed while looking for manifest and lockfile")?;
            let search_params = construct_show_params(
                &self.pkg_path,
                manifest.map(|p| p.try_into()).transpose()?,
                global_manifest_path(&flox).try_into()?,
                PathOrJson::Path(lockfile),
                flox.features.search_strategy,
            )?;

            let (search_results, exit_status) = do_search(&search_params)?;

            if search_results.results.is_empty() {
                bail!("no packages matched this pkg-path: '{}'", self.pkg_path);
            }
            // Render what we have no matter what, then indicate whether we encountered an error.
            render_show_pkgdb(search_results.results.as_slice())?;
            if exit_status.success() {
                return Ok(());
            } else {
                bail!(
                    "pkgdb exited with status code: {}",
                    exit_status.code().unwrap_or(-1),
                );
            }
        };

        Ok(())
    }
}

fn construct_show_params(
    search_term: &str,
    manifest: Option<PathOrJson>,
    global_manifest: PathOrJson,
    lockfile: PathOrJson,
    search_strategy: SearchStrategy,
) -> Result<SearchParams> {
    let parts = search_term
        .split(SEARCH_INPUT_SEPARATOR)
        .map(String::from)
        .collect::<Vec<_>>();
    let (_input_name, package_name) = match parts.as_slice() {
        [package_name] => (None, Some(package_name.to_owned())),
        [input_name, package_name] => (Some(input_name.to_owned()), Some(package_name.to_owned())),
        _ => Err(ShowError::InvalidSearchTerm(search_term.to_owned()))?,
    };

    let query = Query::new(
        package_name.as_ref().unwrap(), // We already know it's Some(_)
        search_strategy,
        None,
        false,
    )?;
    let search_params = SearchParams {
        manifest,
        global_manifest,
        lockfile,
        query,
    };
    debug!("show params raw: {:?}", search_params);
    Ok(search_params)
}

fn render_show_catalog(
    search_results: &[SearchResult],
    expected_systems: &HashSet<System>,
) -> Result<()> {
    if search_results.is_empty() {
        // This should never happen since we've already checked that the
        // set of results is non-empty.
        bail!("no packages found");
    }
    let pkg_name = search_results[0].rel_path.join(".");
    let description = search_results[0]
        .description
        .as_ref()
        .map(|d| d.replace('\n', " "))
        .unwrap_or(DEFAULT_DESCRIPTION.into());
    println!("{pkg_name} - {description}");

    // Organize the versions to be queried and printed
    let version_to_systems = {
        let mut map = BTreeMap::new();
        for pkg in search_results.iter() {
            // The `version` field on `SearchResult` is optional for compatibility with `pkgdb`.
            // Every package from the catalog will have a version, but right now `search` and `show`
            // both convert to `SearchResult` with this optional `version` field for compatibility
            // with `pkgdb` even though with the catalog we get much more data.
            if let Some(ref version) = pkg.version {
                map.entry(version.clone())
                    .or_insert(HashSet::new())
                    .insert(pkg.system.clone());
            }
        }
        map
    };
    let mut seen_versions = HashSet::new();
    // We iterate over the search results again instead of just the `version_to_systems` map since
    // although the keys (and therefore the versions) in the map are sorted (BTreeMap is a sorted map),
    // they are sorted lexically. This may be a different order than how the versions *should* be sorted,
    // so we defer to the order in which the server returns results to us.
    for pkg in search_results {
        if let Some(ref version) = pkg.version {
            if seen_versions.contains(&version) {
                // We print everything in one go for each version, so if we've seen it once
                // we don't need to do anything else.
                continue;
            }
            let Some(systems) = version_to_systems.get(version) else {
                // This should be unreachable since we've already iterated over the search results.
                continue;
            };
            let available_systems = {
                let mut intersection = expected_systems
                    .intersection(systems)
                    .cloned()
                    .collect::<Vec<_>>();
                intersection.sort();
                intersection
            };
            if available_systems.len() != expected_systems.len() {
                println!(
                    "    {pkg_name}@{version} ({} only)",
                    available_systems.join(", ")
                );
            } else {
                println!("    {pkg_name}@{version}");
            }
            seen_versions.insert(version);
        }
    }
    Ok(())
}

fn render_show_pkgdb(search_results: &[SearchResult]) -> Result<()> {
    let mut pkg_name = None;
    let mut results = Vec::new();
    // Collect all versions of the top search result
    for package in search_results.iter() {
        let this_pkg_name = package.rel_path.join(".");
        if pkg_name.is_none() {
            pkg_name = Some(this_pkg_name.clone());
        }
        if pkg_name == Some(this_pkg_name) {
            results.push(package);
        }
    }
    if results.is_empty() {
        // This should never happen since we've already checked that the
        // set of results is non-empty.
        bail!("no packages found");
    }
    let pkg_name = pkg_name.unwrap();
    let description = results[0]
        .description
        .as_ref()
        .map(|d| d.replace('\n', " "))
        .unwrap_or(DEFAULT_DESCRIPTION.into());

    println!("{pkg_name} - {description}");
    for result in results.iter() {
        let name = result.rel_path.join(".");
        // We don't print packages that don't have a version since
        // the resolver will always rank versioned packages higher.
        let Some(version) = result.version.clone() else {
            continue;
        };
        println!("    {name}@{version}");
    }
    Ok(())
}

#[cfg(test)]
mod test {
    use flox_rust_sdk::flox::test_helpers::flox_instance_with_optional_floxhub_and_client;
    use flox_rust_sdk::providers::catalog::{ApiErrorResponse, Client};

    use super::*;

    #[tokio::test]
    async fn show_handles_404() {
        let (mut flox, _temp_dir_handle) =
            flox_instance_with_optional_floxhub_and_client(None, true);
        let Client::Mock(ref mut client) = flox.catalog_client.as_mut().unwrap() else {
            panic!()
        };
        client.push_error_response(
            ApiErrorResponse {
                detail: "detail".to_string(),
            },
            404,
        );
        let search_term = "search_term";
        let err = Show {
            pkg_path: search_term.to_string(),
        }
        .handle(flox)
        .await
        .unwrap_err();

        assert_eq!(
            err.to_string(),
            format!("no packages matched this pkg-path: '{}'", search_term)
        );
    }
}
