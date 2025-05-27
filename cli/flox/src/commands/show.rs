use std::cmp::max;
use std::collections::{BTreeMap, HashSet};

use anyhow::{Result, bail};
use bpaf::Bpaf;
use flox_rust_sdk::data::System;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::search::{PackageBuild, PackageDetails};
use flox_rust_sdk::providers::catalog::{ClientTrait, VersionsError};
use tracing::{debug, instrument};

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

        debug!("using catalog client for show");

        let results = match flox.catalog_client.package_versions(&self.pkg_path).await {
            Ok(results) => results,
            // Below, results.is_empty() is used to mean the search_term
            // didn't match a package.
            // So translate 404 into an empty vec![].
            // Once we drop the pkgdb code path, we can clean this up.
            Err(VersionsError::NotFound) => PackageDetails {
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
    search_results: &[PackageBuild],
    expected_systems: &HashSet<System>,
) -> Result<()> {
    if search_results.is_empty() {
        // This should never happen since we've already checked that the
        // set of results is non-empty.
        bail!("no packages found");
    }

    let pkg_path = search_results[0].pkg_path.clone();
    let description = search_results[0]
        .description
        .as_ref()
        .map(|d| d.replace('\n', " "))
        .filter(|d| !d.trim().is_empty())
        .unwrap_or(DEFAULT_DESCRIPTION.into());
    println!("{pkg_path} - {description}");

    // Organize the versions to be queried and printed
    let version_to_systems = {
        let mut map = BTreeMap::new();
        for pkg in search_results.iter() {
            map.entry(pkg.version.clone())
                .or_insert(HashSet::new())
                .insert(pkg.system.to_string());
        }
        map
    };

    let version_width = max(
        10,
        version_to_systems
            .keys()
            .map(|version| version.len())
            .max()
            .unwrap_or(0),
    );

    let systems_width = version_to_systems
        .values()
        .map(|systems| {
            let mut intersection = expected_systems
                .intersection(systems)
                .cloned()
                .collect::<Vec<_>>();
            intersection.sort();
            intersection.join(", ").len()
        })
        .max()
        .unwrap_or(10);

    let mut seen_versions = HashSet::new();
    for pkg in search_results {
        if seen_versions.contains(&pkg.version) {
            continue;
        }
        let Some(systems) = version_to_systems.get(&pkg.version) else {
            continue;
        };

        let available_systems = {
            let mut intersection = expected_systems
                .intersection(systems)
                .cloned()
                .collect::<Vec<_>>();
            intersection.sort();
            intersection.join(", ")
        };

        println!(
            "    {pkg_path}@{:<version_width$} {:<systems_width$}",
            pkg.version, available_systems
        );
        seen_versions.insert(&pkg.version);
    }

    Ok(())
}

#[cfg(test)]
mod test {
    use flox_rust_sdk::flox::test_helpers::flox_instance;
    use flox_rust_sdk::providers::catalog::test_helpers::auto_recording_catalog_client;

    use super::*;

    #[tokio::test]
    async fn show_handles_404() {
        let (mut flox, _temp_dir_handle) = flox_instance();
        flox.catalog_client = auto_recording_catalog_client("show_handles_404");
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
