use std::collections::{BTreeMap, HashSet};

use anyhow::{bail, Result};
use bpaf::Bpaf;
use flox_rust_sdk::data::System;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::search::{SearchResult, SearchResults};
use flox_rust_sdk::providers::catalog::{ClientTrait, VersionsError};
use tracing::instrument;

use crate::subcommand_metric;
use crate::utils::search::DEFAULT_DESCRIPTION;
use crate::utils::tracing::sentry_set_tag;

// Show detailed package information
#[derive(Debug, Bpaf, Clone)]
pub struct Show {
    /// The package to show detailed information about. Must be an exact match
    /// for a pkg-path e.g. something copy-pasted from the output of `flox search`.
    #[bpaf(positional("pkg-path"))]
    pub pkg_path: String,
}

impl Show {
    #[instrument(name = "show", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("show");
        sentry_set_tag("pkg_path", &self.pkg_path);

        tracing::debug!("using catalog client for show");
        let results = match flox.catalog_client.package_versions(&self.pkg_path).await {
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

        Ok(())
    }
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

#[cfg(test)]
mod test {
    use flox_rust_sdk::flox::test_helpers::flox_instance;
    use flox_rust_sdk::providers::catalog::{ApiErrorResponse, Client};

    use super::*;

    #[tokio::test]
    async fn show_handles_404() {
        let (mut flox, _temp_dir_handle) = flox_instance();
        let Client::Mock(ref mut client) = flox.catalog_client else {
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
            format!("no packages matched this pkg-path: '{search_term}'")
        );
    }
}
