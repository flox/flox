//! Dependency diff for upgrade detail.
//!
//! Uses the catalog API's raw dependency report to compare the
//! transitive dependency graphs of old and new package versions,
//! showing what changed without requiring local derivation files.

use std::collections::HashMap;

use anyhow::Result;
use flox_rust_sdk::models::environment::SingleSystemUpgradeDiff;
use flox_rust_sdk::providers::catalog::{ClientTrait, RawDependencyReport};

/// A single dependency change entry.
#[derive(Debug, Clone, PartialEq)]
enum DepChange {
    /// Dependency version changed.
    VersionChange {
        name: String,
        old_ver: String,
        new_ver: String,
    },
    /// Same version but different store path (build change).
    BuildChange { name: String, version: String },
    /// Dependency was added.
    Added { name: String, version: String },
    /// Dependency was removed.
    Removed { name: String, version: String },
}

/// Extract (name, version) from a Nix store path.
///
/// For `/nix/store/hash-name-version`, returns `("name", Some("version"))`.
/// Name is everything up to but not including the first component
/// starting with a digit.
fn parse_store_path_name(store_path: &str) -> (String, Option<String>) {
    // Strip .drv extension if present
    let path = store_path.trim_end_matches(".drv");

    // Get the filename part (after last /)
    let filename = path.rsplit('/').next().unwrap_or(path);

    // Split on '-', skip the hash (first component)
    let components: Vec<&str> = filename.split('-').skip(1).collect();

    if components.is_empty() {
        return (filename.to_string(), None);
    }

    let mut name_parts = Vec::new();
    let mut version_parts = Vec::new();
    let mut found_version = false;

    for component in &components {
        if !found_version && component.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            found_version = true;
        }

        if found_version {
            version_parts.push(*component);
        } else {
            name_parts.push(*component);
        }
    }

    let name = if name_parts.is_empty() {
        components[0].to_string()
    } else {
        name_parts.join("-")
    };

    let version = if version_parts.is_empty() {
        None
    } else {
        Some(version_parts.join("-"))
    };

    (name, version)
}

/// Strip the `/nix/store/` prefix from a store path for API calls.
fn strip_store_prefix(path: &str) -> &str {
    path.strip_prefix("/nix/store/").unwrap_or(path)
}

/// Get the primary output store path from a locked package.
///
/// Prefers the "out" output, falls back to the first available output.
fn get_output_path(pkg: &flox_rust_sdk::models::lockfile::LockedPackage) -> Option<String> {
    let catalog = pkg.as_catalog_package_ref()?;
    catalog
        .outputs
        .get("out")
        .or_else(|| catalog.outputs.values().next())
        .cloned()
}

/// Build a name -> (store_path, version) index from a dependency report.
fn index_dependencies(report: &RawDependencyReport) -> HashMap<String, (String, Option<String>)> {
    let mut index = HashMap::new();
    for store_path in report.dependencies.keys() {
        let (name, version) = parse_store_path_name(store_path);
        index.insert(name, (store_path.clone(), version));
    }
    index
}

/// Diff two dependency reports to find changes.
fn diff_dependencies(
    old_report: &RawDependencyReport,
    new_report: &RawDependencyReport,
) -> Vec<DepChange> {
    let old_deps = index_dependencies(old_report);
    let new_deps = index_dependencies(new_report);

    let mut changes = Vec::new();

    // Find changed and removed dependencies
    for (name, (old_path, old_ver)) in &old_deps {
        if let Some((new_path, new_ver)) = new_deps.get(name) {
            // Same store path = unchanged, skip
            if old_path == new_path {
                continue;
            }
            let old_v = old_ver.clone().unwrap_or_default();
            let new_v = new_ver.clone().unwrap_or_default();
            if old_v != new_v {
                changes.push(DepChange::VersionChange {
                    name: name.clone(),
                    old_ver: old_v,
                    new_ver: new_v,
                });
            } else {
                changes.push(DepChange::BuildChange {
                    name: name.clone(),
                    version: old_v,
                });
            }
        } else {
            changes.push(DepChange::Removed {
                name: name.clone(),
                version: old_ver.clone().unwrap_or_default(),
            });
        }
    }

    // Find added dependencies
    for (name, (_, new_ver)) in &new_deps {
        if !old_deps.contains_key(name) {
            changes.push(DepChange::Added {
                name: name.clone(),
                version: new_ver.clone().unwrap_or_default(),
            });
        }
    }

    // Sort by name for deterministic output
    changes.sort_by(|a, b| {
        let name_a = match a {
            DepChange::VersionChange { name, .. }
            | DepChange::BuildChange { name, .. }
            | DepChange::Added { name, .. }
            | DepChange::Removed { name, .. } => name,
        };
        let name_b = match b {
            DepChange::VersionChange { name, .. }
            | DepChange::BuildChange { name, .. }
            | DepChange::Added { name, .. }
            | DepChange::Removed { name, .. } => name,
        };
        name_a.cmp(name_b)
    });

    changes
}

/// Render dependency changes as a flat list with tree-drawing characters.
fn render_changes(changes: &[DepChange], prefix: &str) -> String {
    let mut lines = Vec::new();

    // Show version changes first, then build changes, then added/removed
    let version_changes: Vec<_> = changes
        .iter()
        .filter(|c| matches!(c, DepChange::VersionChange { .. }))
        .collect();
    let build_changes: Vec<_> = changes
        .iter()
        .filter(|c| matches!(c, DepChange::BuildChange { .. }))
        .collect();
    let added: Vec<_> = changes
        .iter()
        .filter(|c| matches!(c, DepChange::Added { .. }))
        .collect();
    let removed: Vec<_> = changes
        .iter()
        .filter(|c| matches!(c, DepChange::Removed { .. }))
        .collect();

    let all_items: Vec<&DepChange> = version_changes
        .into_iter()
        .chain(build_changes)
        .chain(added)
        .chain(removed)
        .collect();

    for (i, change) in all_items.iter().enumerate() {
        let is_last = i == all_items.len() - 1;
        let connector = if is_last { "└── " } else { "├── " };

        let line = match change {
            DepChange::VersionChange {
                name,
                old_ver,
                new_ver,
            } => format!("{prefix}{connector}{name}: {old_ver} -> {new_ver}"),
            DepChange::BuildChange { name, version } => {
                let ver = if version.is_empty() {
                    String::new()
                } else {
                    format!(": {version}")
                };
                format!("{prefix}{connector}{name}{ver} (changed)")
            },
            DepChange::Added { name, version } => {
                let ver = if version.is_empty() {
                    String::new()
                } else {
                    format!(": {version}")
                };
                format!("{prefix}{connector}+ {name}{ver}")
            },
            DepChange::Removed { name, version } => {
                let ver = if version.is_empty() {
                    String::new()
                } else {
                    format!(": {version}")
                };
                format!("{prefix}{connector}- {name}{ver}")
            },
        };
        lines.push(line);
    }

    lines.join("\n")
}

/// Entry point: render dependency changes for build-only updates.
///
/// Fetches dependency reports from the catalog server for the old
/// and new output paths, diffs them, and renders the changes.
pub async fn render_detail_tree(
    diff: &SingleSystemUpgradeDiff,
    client: &impl ClientTrait,
) -> Result<String> {
    let mut output_parts = Vec::new();

    for (_, (before, after)) in diff.iter() {
        let old_version = before.version().unwrap_or("unknown");
        let new_version = after.version().unwrap_or("unknown");

        // Only show detail for build-only changes
        if new_version != old_version {
            continue;
        }

        let install_id = before.install_id();

        let Some(old_output) = get_output_path(before) else {
            continue;
        };
        let Some(new_output) = get_output_path(after) else {
            continue;
        };

        let old_stripped = strip_store_prefix(&old_output);
        let new_stripped = strip_store_prefix(&new_output);

        let (old_report, new_report) = match (
            client.get_raw_dependency_report(old_stripped).await,
            client.get_raw_dependency_report(new_stripped).await,
        ) {
            (Ok(old), Ok(new)) => (old, new),
            (Err(e), _) | (_, Err(e)) => {
                tracing::debug!("Could not fetch dependency report for {install_id}: {e}");
                output_parts.push(format!(
                    "  {install_id}: could not analyze dependencies \
                     (dependency report unavailable)"
                ));
                continue;
            },
        };

        let changes = diff_dependencies(&old_report, &new_report);

        if changes.is_empty() {
            continue;
        }

        let version_changes = changes
            .iter()
            .filter(|c| matches!(c, DepChange::VersionChange { .. }))
            .count();
        let build_changes = changes
            .iter()
            .filter(|c| matches!(c, DepChange::BuildChange { .. }))
            .count();
        let added_count = changes
            .iter()
            .filter(|c| matches!(c, DepChange::Added { .. }))
            .count();
        let removed_count = changes
            .iter()
            .filter(|c| matches!(c, DepChange::Removed { .. }))
            .count();

        let mut summary_parts = Vec::new();
        if version_changes > 0 {
            summary_parts.push(format!(
                "{version_changes} version change{}",
                if version_changes == 1 { "" } else { "s" }
            ));
        }
        if build_changes > 0 {
            summary_parts.push(format!(
                "{build_changes} rebuild{}",
                if build_changes == 1 { "" } else { "s" }
            ));
        }
        if added_count > 0 {
            summary_parts.push(format!("{added_count} added"));
        }
        if removed_count > 0 {
            summary_parts.push(format!("{removed_count} removed"));
        }

        let summary = summary_parts.join(", ");
        let header = format!("  {install_id} dependencies ({summary}):");
        let tree = render_changes(&changes, "    ");
        output_parts.push(format!("{header}\n{tree}"));
    }

    Ok(output_parts.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_store_path() {
        let (name, ver) = parse_store_path_name("/nix/store/abc123-terraform-docs-0.21.0");
        assert_eq!(name, "terraform-docs");
        assert_eq!(ver, Some("0.21.0".to_string()));
    }

    #[test]
    fn parse_store_path_no_version() {
        let (name, ver) = parse_store_path_name("/nix/store/abc123-source");
        assert_eq!(name, "source");
        assert_eq!(ver, None);
    }

    #[test]
    fn parse_store_path_drv_extension() {
        let (name, ver) =
            parse_store_path_name("/nix/store/sa46bbbzrfbapj9lxdmvcvkr6qkc9690-bash-5.3p3.drv");
        assert_eq!(name, "bash");
        assert_eq!(ver, Some("5.3p3".to_string()));
    }

    #[test]
    fn parse_store_path_hyphenated_name() {
        let (name, ver) = parse_store_path_name("/nix/store/abc123-apache-httpd-2.0.48");
        assert_eq!(name, "apache-httpd");
        assert_eq!(ver, Some("2.0.48".to_string()));
    }

    #[test]
    fn parse_store_path_go_package() {
        let (name, ver) = parse_store_path_name("/nix/store/hash-go-1.22.5");
        assert_eq!(name, "go");
        assert_eq!(ver, Some("1.22.5".to_string()));
    }

    #[test]
    fn strip_nix_store_prefix() {
        assert_eq!(
            strip_store_prefix("/nix/store/abc123-hello-2.12.2"),
            "abc123-hello-2.12.2"
        );
        assert_eq!(
            strip_store_prefix("abc123-hello-2.12.2"),
            "abc123-hello-2.12.2"
        );
    }

    #[test]
    fn diff_deps_version_change() {
        let old = RawDependencyReport {
            storepath: "/nix/store/old-hello-2.12.2".to_string(),
            dependencies: HashMap::from([
                ("/nix/store/aaa-glibc-2.38".to_string(), Some(vec![])),
                ("/nix/store/bbb-openssl-3.3.0".to_string(), Some(vec![])),
            ]),
        };
        let new = RawDependencyReport {
            storepath: "/nix/store/new-hello-2.12.2".to_string(),
            dependencies: HashMap::from([
                ("/nix/store/aaa-glibc-2.38".to_string(), Some(vec![])),
                ("/nix/store/ccc-openssl-3.4.0".to_string(), Some(vec![])),
            ]),
        };

        let changes = diff_dependencies(&old, &new);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0], DepChange::VersionChange {
            name: "openssl".to_string(),
            old_ver: "3.3.0".to_string(),
            new_ver: "3.4.0".to_string(),
        });
    }

    #[test]
    fn diff_deps_build_change() {
        let old = RawDependencyReport {
            storepath: "/nix/store/old-pkg-1.0".to_string(),
            dependencies: HashMap::from([("/nix/store/aaa-zlib-1.3.1".to_string(), Some(vec![]))]),
        };
        let new = RawDependencyReport {
            storepath: "/nix/store/new-pkg-1.0".to_string(),
            dependencies: HashMap::from([("/nix/store/bbb-zlib-1.3.1".to_string(), Some(vec![]))]),
        };

        let changes = diff_dependencies(&old, &new);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0], DepChange::BuildChange {
            name: "zlib".to_string(),
            version: "1.3.1".to_string(),
        });
    }

    #[test]
    fn diff_deps_added_and_removed() {
        let old = RawDependencyReport {
            storepath: "/nix/store/old-pkg-1.0".to_string(),
            dependencies: HashMap::from([("/nix/store/aaa-curl-8.9.0".to_string(), Some(vec![]))]),
        };
        let new = RawDependencyReport {
            storepath: "/nix/store/new-pkg-1.0".to_string(),
            dependencies: HashMap::from([("/nix/store/bbb-wget-1.21".to_string(), Some(vec![]))]),
        };

        let changes = diff_dependencies(&old, &new);
        assert_eq!(changes.len(), 2);
        // Should have one added and one removed (sorted by name)
        assert!(
            changes
                .iter()
                .any(|c| matches!(c, DepChange::Added { name, .. } if name == "wget"))
        );
        assert!(
            changes
                .iter()
                .any(|c| matches!(c, DepChange::Removed { name, .. } if name == "curl"))
        );
    }

    #[test]
    fn diff_deps_unchanged_pruned() {
        let old = RawDependencyReport {
            storepath: "/nix/store/old-pkg-1.0".to_string(),
            dependencies: HashMap::from([
                ("/nix/store/aaa-glibc-2.38".to_string(), Some(vec![])),
                ("/nix/store/bbb-openssl-3.3.0".to_string(), Some(vec![])),
            ]),
        };
        let new = RawDependencyReport {
            storepath: "/nix/store/new-pkg-1.0".to_string(),
            dependencies: HashMap::from([
                ("/nix/store/aaa-glibc-2.38".to_string(), Some(vec![])),
                ("/nix/store/ccc-openssl-3.4.0".to_string(), Some(vec![])),
            ]),
        };

        let changes = diff_dependencies(&old, &new);
        // glibc unchanged (same path), should not appear
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0], DepChange::VersionChange {
            name: "openssl".to_string(),
            old_ver: "3.3.0".to_string(),
            new_ver: "3.4.0".to_string(),
        });
    }

    #[test]
    fn render_changes_mixed() {
        let changes = vec![
            DepChange::VersionChange {
                name: "go".to_string(),
                old_ver: "1.22.5".to_string(),
                new_ver: "1.22.8".to_string(),
            },
            DepChange::BuildChange {
                name: "zlib".to_string(),
                version: "1.3.1".to_string(),
            },
            DepChange::Added {
                name: "cacert".to_string(),
                version: "3.98".to_string(),
            },
        ];
        let rendered = render_changes(&changes, "    ");
        assert_eq!(
            rendered,
            "    ├── go: 1.22.5 -> 1.22.8\n\
             \x20   ├── zlib: 1.3.1 (changed)\n\
             \x20   └── + cacert: 3.98"
        );
    }

    #[test]
    fn render_changes_single() {
        let changes = vec![DepChange::VersionChange {
            name: "openssl".to_string(),
            old_ver: "3.3.0".to_string(),
            new_ver: "3.3.1".to_string(),
        }];
        let rendered = render_changes(&changes, "  ");
        assert_eq!(rendered, "  └── openssl: 3.3.0 -> 3.3.1");
    }
}
