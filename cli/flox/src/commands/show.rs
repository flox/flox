use std::collections::{HashMap, HashSet};
use std::io::Write;

use anyhow::{Result, bail};
use bpaf::Bpaf;
use flox_rust_sdk::data::System;
use flox_rust_sdk::flox::Flox;
use floxhub_client::{CatalogClientTrait, DeprecationInfo, PackageBuild, VersionsError};
use itertools::Itertools;
use serde::Serialize;
use tracing::instrument;

use crate::subcommand_metric;
use crate::utils::deprecation::deprecation_notice;
use crate::utils::message;
use crate::utils::search::DEFAULT_DESCRIPTION;
use crate::utils::tracing::sentry_set_tag;

// Show detailed package information
#[derive(Debug, Bpaf, Clone)]
pub struct Show {
    /// Display available versions as a JSON array with all available
    /// metadata about each version.
    #[bpaf(long)]
    pub json: bool,

    /// Show additional metadata about each version, including stabilities
    /// that provide each version. Ignored with --json.
    #[bpaf(short, long)]
    pub all: bool,

    /// The package to show detailed information about. Must be an exact match
    /// for a pkg-path e.g. something copy-pasted from the output of `flox search`.
    // bpaf requires positionals to be the last field.
    #[bpaf(positional("pkg-path"))]
    pub pkg_path: String,
}

impl Show {
    #[instrument(name = "show", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("show");
        sentry_set_tag("pkg_path", &self.pkg_path);
        sentry_set_tag("json", self.json);
        sentry_set_tag("show_all", self.all);

        tracing::debug!("using catalog client for show");
        let results = match flox.floxhub_client.package_versions(&self.pkg_path).await {
            Ok(results) => results,
            Err(VersionsError::NotFound) => {
                bail!("no packages matched this pkg-path: '{}'", self.pkg_path);
            },
            Err(e) => Err(e)?,
        };
        if results.results.is_empty() {
            // The catalog reports the deprecation even when no build passes
            // the stability filter.
            let notice = results
                .deprecation
                .as_ref()
                .map(|deprecation| format!("\n{}", deprecation_notice(&self.pkg_path, deprecation)))
                .unwrap_or_default();
            bail!(
                "No builds are available for '{}' in any stability.{notice}",
                self.pkg_path
            );
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
        if self.json {
            render_show_versions_json(
                &mut std::io::stdout(),
                &results.results,
                results.deprecation.as_ref(),
            )?;
        } else {
            render_show_catalog(
                &mut std::io::stdout(),
                &results.results,
                &expected_systems,
                self.all,
            )?;
            // After the output so it isn't scrolled off by a long version list.
            if let Some(deprecation) = &results.deprecation {
                message::warning(deprecation_notice(&self.pkg_path, deprecation));
            }
        }

        Ok(())
    }
}

/// A build as listed by `flox show --json`.
///
/// The catalog reports a deprecation once per pkg-path, but the output stays an
/// array of builds so existing consumers keep working. The deprecation is
/// therefore repeated on every build, as `flox search --json` does for each
/// result, and left out entirely when there is none.
#[derive(Serialize)]
struct ShowBuildJson<'a> {
    #[serde(flatten)]
    build: &'a PackageBuild,
    #[serde(skip_serializing_if = "Option::is_none")]
    deprecation: Option<&'a DeprecationInfo>,
}

fn render_show_versions_json(
    writer: &mut impl Write,
    versions: &[PackageBuild],
    deprecation: Option<&DeprecationInfo>,
) -> Result<()> {
    let builds = versions
        .iter()
        .map(|build| ShowBuildJson { build, deprecation })
        .collect::<Vec<_>>();
    let json = serde_json::to_string_pretty(&builds)?;
    writeln!(writer, "{json}")?;
    Ok(())
}

fn render_show_catalog(
    writer: &mut impl Write,
    search_results: &[PackageBuild],
    expected_systems: &HashSet<System>,
    all: bool,
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

    let rows = version_rows(pkg_path.as_str(), search_results, expected_systems);
    let version_width = rows.iter().map(|row| row.version.len()).max().unwrap_or(0);
    // Catalogs that don't report stabilities get no stabilities column.
    let stabilities_width = all
        .then(|| {
            rows.iter()
                .filter_map(|row| row.stabilities.as_ref().map(String::len))
                .max()
        })
        .flatten();

    for row in rows {
        let mut line = format!("{:<version_width$}", row.version);
        if let Some(width) = stabilities_width {
            let stabilities = row.stabilities.unwrap_or_default();
            line.push_str(&format!("  {stabilities:<width$}"));
        }
        if let Some(systems) = row.systems {
            line.push_str(&format!(" ({systems} only)"));
        }
        writeln!(writer, "{}", line.trim_end())?;
    }
    Ok(())
}

/// One line of the "Other versions" list.
#[derive(Clone, Debug)]
struct VersionRow {
    version: String,
    /// The stabilities that have the version on every system it's listed for,
    /// strictest first, or `None` if the catalog doesn't report them
    stabilities: Option<String>,
    /// The systems the version is limited to, if not all expected ones
    systems: Option<String>,
}

/// Collapse the per-system builds into one row per version, in the order the
/// catalog returns them, which is newest first rather than lexical.
fn version_rows(
    pkg_path: &str,
    search_results: &[PackageBuild],
    expected_systems: &HashSet<System>,
) -> Vec<VersionRow> {
    let mut versions: Vec<&str> = Vec::new();
    let mut builds_by_version: HashMap<&str, Vec<&PackageBuild>> = HashMap::new();
    for pkg in search_results {
        let builds = builds_by_version.entry(pkg.version.as_str()).or_default();
        if builds.is_empty() {
            versions.push(pkg.version.as_str());
        }
        builds.push(pkg);
    }

    versions
        .into_iter()
        .map(|version| {
            let builds = &builds_by_version[version];

            let systems = builds
                .iter()
                .map(|pkg| pkg.system.to_string())
                .collect::<HashSet<_>>();
            let mut available_systems = expected_systems
                .intersection(&systems)
                .cloned()
                .collect::<Vec<_>>();
            available_systems.sort();

            // Intersecting keeps the catalog's strictest-first order.
            let stabilities = builds
                .iter()
                .map(|pkg| pkg.available_stabilities.as_ref())
                .collect::<Option<Vec<_>>>()
                .and_then(|per_system| {
                    let (first, rest) = per_system.split_first()?;
                    Some(
                        first
                            .iter()
                            .filter(|s| rest.iter().all(|other| other.contains(s)))
                            .join(", "),
                    )
                });

            VersionRow {
                version: format!("    {pkg_path}@{version}"),
                stabilities,
                systems: (available_systems.len() != expected_systems.len())
                    .then(|| available_systems.join(", ")),
            }
        })
        .collect()
}

#[cfg(test)]
mod test {
    use bpaf::Parser;
    use flox_rust_sdk::flox::test_helpers::flox_instance;
    use flox_rust_sdk::providers::catalog::test_helpers::auto_recording_catalog_client;
    use floxhub_client::client::test_helpers::client_config;
    use floxhub_client::{
        DeprecationKind,
        FloxhubClient,
        PackageOutput,
        PackageOutputs,
        PackageSystem,
    };
    use httpmock::MockServer;
    use indoc::indoc;
    use pretty_assertions::assert_eq;

    use super::*;

    /// bpaf only reports a misplaced positional while building usage, so
    /// this fails if `pkg_path` stops being the last field of `Show`.
    #[test]
    fn show_parser_satisfies_bpaf_invariants() {
        show().to_options().check_invariants(true);
    }

    #[tokio::test]
    async fn show_handles_no_available_builds() {
        let (mut flox, _temp_dir_handle) = flox_instance();
        // Use a hand-written response because no production package returning
        // an empty successful result was found for an auto-recorded fixture.
        let server = MockServer::start_async().await;
        let mock = server.mock(|when, then| {
            when.method("GET")
                .path("/api/v1/catalog/packages/search_term");
            then.status(200)
                .json_body(serde_json::json!({"items": [], "total_count": 0}));
        });
        flox.floxhub_client = FloxhubClient::new(client_config(&server.base_url())).unwrap();

        let err = Show {
            pkg_path: "search_term".to_string(),
            json: false,
            all: false,
        }
        .handle(flox)
        .await
        .unwrap_err();

        assert_eq!(
            err.to_string(),
            "No builds are available for 'search_term' in any stability."
        );
        mock.assert();
    }

    /// The catalog reports a deprecation even when every build is filtered
    /// out, and the notice has to reach the user then too.
    #[tokio::test]
    async fn show_no_available_builds_includes_the_deprecation() {
        let (mut flox, _temp_dir_handle) = flox_instance();
        let server = MockServer::start_async().await;
        server.mock(|when, then| {
            when.method("GET").path("/api/v1/catalog/packages/php81");
            then.status(200).json_body(serde_json::json!({
                "items": [],
                "total_count": 0,
                "deprecation": {"kind": "removed", "message": "php81 is EOL"},
            }));
        });
        flox.floxhub_client = FloxhubClient::new(client_config(&server.base_url())).unwrap();

        let err = Show {
            pkg_path: "php81".to_string(),
            json: false,
            all: false,
        }
        .handle(flox)
        .await
        .unwrap_err();

        assert_eq!(
            err.to_string(),
            "No builds are available for 'php81' in any stability.\nphp81 is EOL"
        );
    }

    #[tokio::test]
    async fn show_handles_404() {
        let (mut flox, _temp_dir_handle) = flox_instance();
        flox.floxhub_client = auto_recording_catalog_client("show_handles_404");
        let search_term = "search_term";
        let err = Show {
            pkg_path: search_term.to_string(),
            json: false,
            all: false,
        }
        .handle(flox)
        .await
        .unwrap_err();

        assert_eq!(
            err.to_string(),
            format!("no packages matched this pkg-path: '{search_term}'")
        );
    }

    fn mock_pkg(version: &str, system: &str) -> PackageBuild {
        use chrono::TimeZone;

        let rev_date = chrono::Utc
            .with_ymd_and_hms(2025, 5, 31, 12, 5, 15)
            .unwrap();

        PackageBuild {
            available_stabilities: None,
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
        }
    }

    fn mock_pkg_at(version: &str, system: &str, stabilities: &[&str]) -> PackageBuild {
        PackageBuild {
            available_stabilities: Some(stabilities.iter().map(|s| s.to_string()).collect()),
            ..mock_pkg(version, system)
        }
    }

    fn all_systems() -> HashSet<System> {
        [
            "aarch64-darwin",
            "aarch64-linux",
            "x86_64-darwin",
            "x86_64-linux",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    #[test]
    fn column_alignment_for_system_restrictions() {
        let packages = vec![
            mock_pkg("1.0", "aarch64-darwin"),
            mock_pkg("10.0.0", "aarch64-darwin"),
        ];

        let mut buf = vec![];
        render_show_catalog(&mut buf, &packages, &all_systems(), false).unwrap();
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

    /// With `--all`, each version lists the stabilities that have it on every
    /// system it's listed for, in an aligned column before any system
    /// restriction.
    #[test]
    fn versions_list_their_stabilities() {
        let stable = ["stable", "staging", "unstable"];
        let lts = ["lts", "stable", "staging", "unstable"];
        let mut packages = vec![mock_pkg_at("10.0.0", "aarch64-darwin", &["unstable"])];
        for system in all_systems() {
            packages.push(mock_pkg_at("2.0", &system, &stable));
        }
        for system in all_systems() {
            // One system has the version at lts too.
            let stabilities = if system == "x86_64-linux" {
                &lts[..]
            } else {
                &stable[..]
            };
            packages.push(mock_pkg_at("1.0", &system, stabilities));
        }

        let mut buf = vec![];
        render_show_catalog(&mut buf, &packages, &all_systems(), true).unwrap();
        let output = String::from_utf8(buf).unwrap();

        assert_eq!(output, indoc! {"
                pkg - test
                Catalog: nixpkgs
                Latest:  pkg@10.0.0
                License: MIT
                Outputs: sub-pkg*, sub-pkg-other (* installed by default)
                Systems: aarch64-darwin

                Other versions:
                    pkg@10.0.0  unstable                  (aarch64-darwin only)
                    pkg@2.0     stable, staging, unstable
                    pkg@1.0     stable, staging, unstable
            "});
    }

    /// The default output has no stabilities column, even when the catalog
    /// reports stabilities.
    #[test]
    fn default_output_omits_stabilities() {
        let packages = vec![
            mock_pkg_at("2.0", "aarch64-darwin", &["stable", "unstable"]),
            mock_pkg_at("1.0", "aarch64-darwin", &["unstable"]),
        ];

        let mut buf = vec![];
        render_show_catalog(&mut buf, &packages, &all_systems(), false).unwrap();
        let output = String::from_utf8(buf).unwrap();

        assert_eq!(output, indoc! {"
                pkg - test
                Catalog: nixpkgs
                Latest:  pkg@2.0
                License: MIT
                Outputs: sub-pkg*, sub-pkg-other (* installed by default)
                Systems: aarch64-darwin

                Other versions:
                    pkg@2.0 (aarch64-darwin only)
                    pkg@1.0 (aarch64-darwin only)
            "});
    }

    /// A catalog that doesn't report stabilities gets no column under `--all`.
    #[test]
    fn all_without_reported_stabilities_has_no_column() {
        let packages = vec![
            mock_pkg("1.0", "aarch64-darwin"),
            mock_pkg("10.0.0", "aarch64-darwin"),
        ];

        let mut buf = vec![];
        render_show_catalog(&mut buf, &packages, &all_systems(), true).unwrap();
        let output = String::from_utf8(buf).unwrap();

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

    /// `--json` serializes the version results, including each build's
    /// stabilities, as a single-line array.
    #[test]
    fn json_serializes_version_results() {
        let results = vec![mock_pkg_at("1.0", "aarch64-darwin", &[
            "stable", "unstable",
        ])];

        let mut buf = vec![];
        render_show_versions_json(&mut buf, &results, None).unwrap();
        let json = String::from_utf8(buf).unwrap();

        assert!(json.trim().contains('\n'), "expected pretty-printed JSON");
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let entries = parsed.as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["version"], "1.0");
        assert_eq!(entries[0]["system"], "aarch64-darwin");
        assert_eq!(
            entries[0]["available_stabilities"],
            serde_json::json!(["stable", "unstable"])
        );
    }

    /// `--json` stays an array of builds; each carries the pkg-path's
    /// deprecation, the way `flox search --json` carries it per result. The
    /// deprecation is the catalog's answer for `aria`.
    #[test]
    fn json_carries_deprecation_on_every_build() {
        let deprecation = DeprecationInfo {
            kind: DeprecationKind::Removed,
            message: Some("'aria' has been renamed to/replaced by 'aria2'".to_string()),
            replacement: Some("aria2".to_string()),
        };
        let results = vec![
            mock_pkg("2.0", "aarch64-darwin"),
            mock_pkg("1.0", "aarch64-darwin"),
        ];

        let mut buf = vec![];
        render_show_versions_json(&mut buf, &results, Some(&deprecation)).unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&buf).unwrap();

        assert_eq!(
            parsed,
            serde_json::json!([
                {
                    "attr_path": "",
                    "broken": null,
                    "catalog": "nixpkgs",
                    "deprecation": {
                        "kind": "removed",
                        "message": "'aria' has been renamed to/replaced by 'aria2'",
                        "replacement": "aria2"
                    },
                    "derivation": "",
                    "description": "test",
                    "insecure": null,
                    "license": "MIT",
                    "locked_url": "https://github.com/flox/nixpkgs?rev=abc123",
                    "missing_builds": null,
                    "name": "",
                    "outputs": [
                        {"name": "sub-pkg", "store_path": "somestorepath"},
                        {"name": "sub-pkg-other", "store_path": "somestorepath"}
                    ],
                    "outputs_to_install": ["sub-pkg"],
                    "pkg_path": "pkg",
                    "pname": "",
                    "rev": "",
                    "rev_count": 0,
                    "rev_date": "2025-05-31T12:05:15Z",
                    "scrape_date": null,
                    "stabilities": null,
                    "system": "aarch64-darwin",
                    "unfree": null,
                    "version": "2.0"
                },
                {
                    "attr_path": "",
                    "broken": null,
                    "catalog": "nixpkgs",
                    "deprecation": {
                        "kind": "removed",
                        "message": "'aria' has been renamed to/replaced by 'aria2'",
                        "replacement": "aria2"
                    },
                    "derivation": "",
                    "description": "test",
                    "insecure": null,
                    "license": "MIT",
                    "locked_url": "https://github.com/flox/nixpkgs?rev=abc123",
                    "missing_builds": null,
                    "name": "",
                    "outputs": [
                        {"name": "sub-pkg", "store_path": "somestorepath"},
                        {"name": "sub-pkg-other", "store_path": "somestorepath"}
                    ],
                    "outputs_to_install": ["sub-pkg"],
                    "pkg_path": "pkg",
                    "pname": "",
                    "rev": "",
                    "rev_count": 0,
                    "rev_date": "2025-05-31T12:05:15Z",
                    "scrape_date": null,
                    "stabilities": null,
                    "system": "aarch64-darwin",
                    "unfree": null,
                    "version": "1.0"
                }
            ])
        );
    }

    /// Output for a package without a deprecation is exactly what
    /// serializing the builds alone gives: same key order, no `deprecation`.
    #[test]
    fn json_without_deprecation_is_the_plain_builds() {
        let results = vec![mock_pkg("1.0", "aarch64-darwin")];

        let mut buf = vec![];
        render_show_versions_json(&mut buf, &results, None).unwrap();

        assert_eq!(
            String::from_utf8(buf).unwrap(),
            format!("{}\n", serde_json::to_string_pretty(&results).unwrap())
        );
    }
}
