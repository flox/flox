use std::cmp::max;
use std::collections::{BTreeMap, HashSet};
use std::io::Write;

use anyhow::{Result, bail};
use bpaf::Bpaf;
use flox_rust_sdk::data::System;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::search::{PackageBuild, PackageDetails};
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
        render_show_catalog(&mut std::io::stdout(), &results.results, &expected_systems)?;

        Ok(())
    }
}

fn render_show_catalog(
    writer: &mut impl Write,
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
    writeln!(writer, "{pkg_path} - {description}")?;

    let first_pkg = &search_results[0];

    if let Some(catalog) = &first_pkg.catalog {
        writeln!(writer, "Catalog: {catalog}")?;
    }

    // Print Latest version (first version in results is the latest)
    writeln!(writer, "Latest:  {pkg_path}@{}", first_pkg.version)?;

    // Print License
    if let Some(license) = &first_pkg.license {
        writeln!(writer, "License: {license}")?;
    }

    // Print Outputs
    if !first_pkg.outputs.0.is_empty() {
        let output_names: Vec<String> = first_pkg
            .outputs
            .0
            .iter()
            .map(|output| {
                // Mark the outputs_to_install with asterisk
                if let Some(ref to_install) = first_pkg.outputs_to_install
                    && to_install.contains(&output.name)
                {
                    return format!("{}*", output.name);
                }
                output.name.clone()
            })
            .collect();
        writeln!(
            writer,
            "Outputs: {} (* installed by default)",
            output_names.join(", ")
        )?;
    }

    // Print Systems (all systems where the latest version is available)
    let latest_version_systems: Vec<String> = search_results
        .iter()
        .filter(|pkg| pkg.version == first_pkg.version)
        .map(|pkg| pkg.system.to_string())
        .collect();
    writeln!(writer, "Systems: {}", latest_version_systems.join(", "))?;

    writeln!(writer)?; // Empty line before versions list
    writeln!(writer, "Other versions:")?;

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
    // calculating the maximum width needed for the version column
    let version_column_width = {
        let mut seen_versions = HashSet::new();
        let mut max_width = 0;
        for pkg in search_results {
            if !seen_versions.contains(&pkg.version) {
                let version_str = format!("    {pkg_path}@{}", pkg.version);
                max_width = max(max_width, version_str.len());
                seen_versions.insert(&pkg.version);
            }
        }
        max_width
    };

    let mut seen_versions = HashSet::new();
    // We iterate over the search results again instead of just the `version_to_systems` map since
    // although the keys (and therefore the versions) in the map are sorted (BTreeMap is a sorted map),
    // they are sorted lexically. This may be a different order than how the versions *should* be sorted,
    // so we defer to the order in which the server returns results to us.
    for pkg in search_results {
        if seen_versions.contains(&pkg.version) {
            // We print everything in one go for each version, so if we've seen it once
            // we don't need to do anything else.
            continue;
        }
        let Some(systems) = version_to_systems.get(&pkg.version) else {
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

        let version_str = format!("    {pkg_path}@{}", pkg.version);

        if available_systems.len() != expected_systems.len() {
            writeln!(
                writer,
                "{:<version_column_width$} ({} only)",
                version_str,
                available_systems.join(", ")
            )?;
        } else {
            writeln!(writer, "{version_str}")?;
        }
        seen_versions.insert(&pkg.version);
    }
    Ok(())
}

#[cfg(test)]
mod test {
    use catalog_api_v1::types::{PackageOutput, PackageOutputs, PackageSystem};
    use flox_rust_sdk::flox::test_helpers::flox_instance;
    use flox_rust_sdk::providers::catalog::test_helpers::auto_recording_catalog_client;
    use indoc::indoc;
    use pretty_assertions::assert_eq;

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

    #[test]
    fn test_column_alignment_for_system_restrictions() {
        use chrono::TimeZone;

        let rev_date = chrono::Utc
            .with_ymd_and_hms(2025, 5, 31, 12, 5, 15)
            .unwrap();

        let mock_pkg = |version: &str, system: &str| PackageBuild {
            pkg_path: "pkg".to_string(),
            version: version.to_string(),
            description: Some("test".to_string()),
            system: system.parse::<PackageSystem>().unwrap(),
            attr_path: String::new(),
            broken: None,
            cache_uri: None,
            catalog: Some("nixpkgs".to_string()),
            derivation: String::new(),
            insecure: None,
            license: Some("MIT".to_string()),
            locked_url: "https://github.com/flox/nixpkgs?rev=abc123".to_string(),
            missing_builds: None,
            name: String::new(),
            outputs: PackageOutputs(vec![
                PackageOutput {
                    name: "sub-pkg".to_string(),
                    store_path: "somestorepath".to_string(),
                },
                PackageOutput {
                    name: "sub-pkg-other".to_string(),
                    store_path: "somestorepath".to_string(),
                },
            ]),
            outputs_to_install: Some(vec!["sub-pkg".to_string()]),
            pname: String::new(),
            rev: String::new(),
            rev_count: 0,
            rev_date,
            scrape_date: None,
            stabilities: None,
            unfree: None,
        };

        let packages = vec![
            mock_pkg("1.0", "aarch64-darwin"),
            mock_pkg("10.0.0", "aarch64-darwin"),
        ];

        let expected_systems = [
            "aarch64-darwin",
            "aarch64-linux",
            "x86_64-darwin",
            "x86_64-linux",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect::<HashSet<_>>();

        let mut buf = vec![];
        render_show_catalog(&mut buf, &packages, &expected_systems).unwrap();
        let output = String::from_utf8(buf).unwrap();

        // Verify that system restrictions are aligned despite different version lengths
        assert_eq!(output, indoc! {"
                pkg - test
                Catalog: nixpkgs
                Latest:  pkg@1.0
                License: MIT
                Outputs: sub-pkg*, sub-pkg-other (* installed by default)
                Systems: aarch64-darwin

                Other versions:
                    pkg@1.0    (aarch64-darwin only)
                    pkg@10.0.0 (aarch64-darwin only)
            "});
    }
}
