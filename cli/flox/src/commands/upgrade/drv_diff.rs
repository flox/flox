//! Derivation dependency diff tree.
//!
//! Compares two Nix derivation paths and recursively shows which
//! dependencies changed, pruning unchanged branches.

use std::collections::{HashMap, HashSet};
use std::process::Stdio;

use anyhow::{Context, Result};
use flox_rust_sdk::models::environment::SingleSystemUpgradeDiff;
use flox_rust_sdk::providers::nix::nix_base_command;

/// Maximum recursion depth when walking derivation input trees.
const MAX_DEPTH: usize = 10;

/// A node in the derivation diff tree.
#[derive(Debug, Clone)]
enum DiffNode {
    /// Dependency version changed.
    VersionChange {
        name: String,
        old_ver: String,
        new_ver: String,
        children: Vec<DiffNode>,
    },
    /// Same version but different derivation (build change).
    BuildChange {
        name: String,
        version: String,
        children: Vec<DiffNode>,
    },
    /// Dependency was added in the new derivation.
    Added { name: String, version: String },
    /// Dependency was removed in the new derivation.
    Removed { name: String, version: String },
    /// Recursion depth limit reached.
    DepthLimit,
    /// Already shown elsewhere in the tree (cycle/diamond).
    AlreadyShown { name: String },
}

/// Parsed info about a derivation's inputs.
#[derive(Debug, Clone)]
struct DrvInputs {
    /// Map of input derivation path -> output names.
    inputs: HashMap<String, Vec<String>>,
}

/// Extract (name, version) from a Nix store path or derivation path.
///
/// For `/nix/store/hash-name-version.drv`, returns `("name", Some("version"))`.
/// Uses the same algorithm as `manifest/raw.rs` — name is everything up to
/// but not including the first dash not followed by a letter.
fn parse_drv_name(store_path: &str) -> (String, Option<String>) {
    // Strip .drv extension if present
    let path = store_path.trim_end_matches(".drv");

    // Get the filename part (after last /)
    let filename = path.rsplit('/').next().unwrap_or(path);

    // Split on '-', skip the hash (first component)
    let components: Vec<&str> = filename.split('-').skip(1).collect();

    if components.is_empty() {
        return (filename.to_string(), None);
    }

    // Name = components until we hit one starting with a digit
    // Version = remaining components joined by '-'
    let mut name_parts = Vec::new();
    let mut version_parts = Vec::new();
    let mut found_version = false;

    for component in &components {
        if !found_version
            && component
                .chars()
                .next()
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false)
        {
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

/// Fetch derivation inputs by running `nix derivation show` on the given paths.
///
/// Batches all paths into a single nix call for efficiency.
fn fetch_derivation_inputs(drv_paths: &[&str]) -> Result<HashMap<String, DrvInputs>> {
    if drv_paths.is_empty() {
        return Ok(HashMap::new());
    }

    let mut cmd = nix_base_command();
    cmd.arg("derivation").arg("show");
    for path in drv_paths {
        cmd.arg(path);
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let output = cmd
        .output()
        .context("failed to run 'nix derivation show'")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("nix derivation show failed: {stderr}");
    }

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).context("failed to parse derivation JSON")?;

    let obj = json
        .as_object()
        .context("expected JSON object from nix derivation show")?;

    let mut result = HashMap::new();

    for (drv_path, drv_info) in obj {
        let input_drvs = drv_info
            .get("inputDrvs")
            .and_then(|v| v.as_object())
            .unwrap_or(&serde_json::Map::new())
            .clone();

        let mut inputs = HashMap::new();
        for (input_path, outputs_val) in input_drvs {
            let outputs = outputs_val
                .get("outputs")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            inputs.insert(input_path, outputs);
        }

        result.insert(drv_path.clone(), DrvInputs { inputs });
    }

    Ok(result)
}

/// Match inputs from old and new derivations by package name.
///
/// Returns: (matched_pairs, added, removed)
/// where matched_pairs contains entries where the drv path changed.
fn match_inputs(
    old_inputs: &HashMap<String, Vec<String>>,
    new_inputs: &HashMap<String, Vec<String>>,
) -> (
    Vec<(String, String, String)>, // (name, old_drv, new_drv)
    Vec<(String, String)>,         // (name, new_drv) — added
    Vec<(String, String)>,         // (name, old_drv) — removed
) {
    // Build name -> drv_path maps
    let old_by_name: HashMap<String, &String> = old_inputs
        .keys()
        .map(|path| {
            let (name, _) = parse_drv_name(path);
            (name, path)
        })
        .collect();

    let new_by_name: HashMap<String, &String> = new_inputs
        .keys()
        .map(|path| {
            let (name, _) = parse_drv_name(path);
            (name, path)
        })
        .collect();

    let mut matched = Vec::new();
    let mut added = Vec::new();
    let mut removed = Vec::new();

    // Find changed and removed
    for (name, old_path) in &old_by_name {
        if let Some(new_path) = new_by_name.get(name) {
            if old_path != new_path {
                matched.push((name.clone(), (*old_path).clone(), (*new_path).clone()));
            }
            // If paths are equal, dependency is unchanged — prune
        } else {
            removed.push((name.clone(), (*old_path).clone()));
        }
    }

    // Find added
    for (name, new_path) in &new_by_name {
        if !old_by_name.contains_key(name) {
            added.push((name.clone(), (*new_path).clone()));
        }
    }

    // Sort for deterministic output
    matched.sort_by(|a, b| a.0.cmp(&b.0));
    added.sort_by(|a, b| a.0.cmp(&b.0));
    removed.sort_by(|a, b| a.0.cmp(&b.0));

    (matched, added, removed)
}

/// Recursively diff two derivation trees.
fn diff_drv_tree(
    old_drv: &str,
    new_drv: &str,
    depth: usize,
    visited: &mut HashSet<String>,
) -> Result<Vec<DiffNode>> {
    if depth >= MAX_DEPTH {
        return Ok(vec![DiffNode::DepthLimit]);
    }

    let key = format!("{old_drv}:{new_drv}");
    if visited.contains(&key) {
        let (name, _) = parse_drv_name(old_drv);
        return Ok(vec![DiffNode::AlreadyShown { name }]);
    }
    visited.insert(key);

    // Batch fetch both derivations
    let infos = fetch_derivation_inputs(&[old_drv, new_drv])?;

    let old_info = infos.get(old_drv);
    let new_info = infos.get(new_drv);

    let empty = HashMap::new();
    let old_inputs = old_info.map(|i| &i.inputs).unwrap_or(&empty);
    let new_inputs = new_info.map(|i| &i.inputs).unwrap_or(&empty);

    let (matched, added, removed) = match_inputs(old_inputs, new_inputs);

    let mut nodes = Vec::new();

    for (name, old_path, new_path) in matched {
        let (_, old_ver) = parse_drv_name(&old_path);
        let (_, new_ver) = parse_drv_name(&new_path);

        let old_ver_str = old_ver.unwrap_or_default();
        let new_ver_str = new_ver.unwrap_or_default();

        let children = diff_drv_tree(&old_path, &new_path, depth + 1, visited)?;

        if old_ver_str != new_ver_str {
            nodes.push(DiffNode::VersionChange {
                name,
                old_ver: old_ver_str,
                new_ver: new_ver_str,
                children,
            });
        } else {
            nodes.push(DiffNode::BuildChange {
                name,
                version: old_ver_str,
                children,
            });
        }
    }

    for (name, drv_path) in added {
        let (_, ver) = parse_drv_name(&drv_path);
        nodes.push(DiffNode::Added {
            name,
            version: ver.unwrap_or_default(),
        });
    }

    for (name, drv_path) in removed {
        let (_, ver) = parse_drv_name(&drv_path);
        nodes.push(DiffNode::Removed {
            name,
            version: ver.unwrap_or_default(),
        });
    }

    Ok(nodes)
}

/// Render a list of DiffNodes as a tree with unicode box-drawing characters.
fn render_tree(nodes: &[DiffNode], prefix: &str, is_root: bool) -> String {
    let mut lines = Vec::new();

    for (i, node) in nodes.iter().enumerate() {
        let is_last = i == nodes.len() - 1;
        let connector = if is_root {
            ""
        } else if is_last {
            "└── "
        } else {
            "├── "
        };
        let child_prefix = if is_root {
            prefix.to_string()
        } else if is_last {
            format!("{prefix}    ")
        } else {
            format!("{prefix}│   ")
        };

        match node {
            DiffNode::VersionChange {
                name,
                old_ver,
                new_ver,
                children,
            } => {
                lines.push(format!("{prefix}{connector}{name}: {old_ver} -> {new_ver}"));
                if !children.is_empty() {
                    lines.push(render_tree(children, &child_prefix, false));
                }
            },
            DiffNode::BuildChange {
                name,
                version,
                children,
            } => {
                let ver_display = if version.is_empty() {
                    String::new()
                } else {
                    format!(": {version}")
                };
                lines.push(format!(
                    "{prefix}{connector}{name}{ver_display} (changed)"
                ));
                if !children.is_empty() {
                    lines.push(render_tree(children, &child_prefix, false));
                }
            },
            DiffNode::Added { name, version } => {
                let ver_display = if version.is_empty() {
                    String::new()
                } else {
                    format!(": {version}")
                };
                lines.push(format!("{prefix}{connector}+ {name}{ver_display}"));
            },
            DiffNode::Removed { name, version } => {
                let ver_display = if version.is_empty() {
                    String::new()
                } else {
                    format!(": {version}")
                };
                lines.push(format!("{prefix}{connector}- {name}{ver_display}"));
            },
            DiffNode::DepthLimit => {
                lines.push(format!("{prefix}{connector}[depth limit reached]"));
            },
            DiffNode::AlreadyShown { name } => {
                lines.push(format!("{prefix}{connector}{name} [already shown]"));
            },
        }
    }

    lines.join("\n")
}

/// Entry point: render a detail tree for build-only changes in the upgrade diff.
///
/// Only processes packages where the version didn't change (build-only updates).
/// For each, it runs `nix derivation show` recursively to find what dependencies
/// actually changed.
pub fn render_detail_tree(diff: &SingleSystemUpgradeDiff) -> Result<String> {
    let mut output_parts = Vec::new();

    for (_, (before, after)) in diff.iter() {
        let old_version = before.version().unwrap_or("unknown");
        let new_version = after.version().unwrap_or("unknown");

        // Only show detail for build-only changes
        if new_version != old_version {
            continue;
        }

        let Some(old_drv) = before.derivation() else {
            continue;
        };
        let Some(new_drv) = after.derivation() else {
            continue;
        };

        let install_id = before.install_id();
        let mut visited = HashSet::new();
        let nodes = match diff_drv_tree(old_drv, new_drv, 0, &mut visited) {
            Ok(nodes) => nodes,
            Err(e) => {
                tracing::debug!("Could not analyze dependencies for {install_id}: {e}");
                output_parts.push(format!(
                    "  {install_id}: could not analyze dependencies \
                     (derivations may not be available locally)"
                ));
                continue;
            },
        };

        if !nodes.is_empty() {
            let header = format!("  {install_id} dependency changes:");
            let tree = render_tree(&nodes, "    ", false);
            output_parts.push(format!("{header}\n{tree}"));
        }
    }

    Ok(output_parts.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_drv_name() {
        let (name, ver) =
            parse_drv_name("/nix/store/abc123-terraform-docs-0.21.0.drv");
        assert_eq!(name, "terraform-docs");
        assert_eq!(ver, Some("0.21.0".to_string()));
    }

    #[test]
    fn parse_drv_name_no_version() {
        let (name, ver) = parse_drv_name("/nix/store/abc123-source.drv");
        assert_eq!(name, "source");
        assert_eq!(ver, None);
    }

    #[test]
    fn parse_drv_name_complex() {
        let (name, ver) =
            parse_drv_name("/nix/store/sa46bbbzrfbapj9lxdmvcvkr6qkc9690-bash-5.3p3.drv");
        assert_eq!(name, "bash");
        assert_eq!(ver, Some("5.3p3".to_string()));
    }

    #[test]
    fn parse_drv_name_hyphenated_name() {
        let (name, ver) =
            parse_drv_name("/nix/store/abc123-apache-httpd-2.0.48.drv");
        assert_eq!(name, "apache-httpd");
        assert_eq!(ver, Some("2.0.48".to_string()));
    }

    #[test]
    fn parse_drv_name_go_package() {
        let (name, ver) =
            parse_drv_name("/nix/store/hash-go-1.22.5.drv");
        assert_eq!(name, "go");
        assert_eq!(ver, Some("1.22.5".to_string()));
    }

    #[test]
    fn render_tree_version_change() {
        let nodes = vec![DiffNode::VersionChange {
            name: "go".to_string(),
            old_ver: "1.22.5".to_string(),
            new_ver: "1.22.8".to_string(),
            children: vec![],
        }];
        let rendered = render_tree(&nodes, "  ", false);
        assert_eq!(rendered, "  └── go: 1.22.5 -> 1.22.8");
    }

    #[test]
    fn render_tree_multiple_nodes() {
        let nodes = vec![
            DiffNode::VersionChange {
                name: "go".to_string(),
                old_ver: "1.22.5".to_string(),
                new_ver: "1.22.8".to_string(),
                children: vec![],
            },
            DiffNode::Added {
                name: "cacert".to_string(),
                version: "3.98".to_string(),
            },
        ];
        let rendered = render_tree(&nodes, "  ", false);
        assert_eq!(
            rendered,
            "  ├── go: 1.22.5 -> 1.22.8\n  └── + cacert: 3.98"
        );
    }

    #[test]
    fn render_tree_nested() {
        let nodes = vec![DiffNode::VersionChange {
            name: "go".to_string(),
            old_ver: "1.22.5".to_string(),
            new_ver: "1.22.8".to_string(),
            children: vec![DiffNode::VersionChange {
                name: "openssl".to_string(),
                old_ver: "3.3.0".to_string(),
                new_ver: "3.3.1".to_string(),
                children: vec![],
            }],
        }];
        let rendered = render_tree(&nodes, "  ", false);
        assert_eq!(
            rendered,
            "  └── go: 1.22.5 -> 1.22.8\n      └── openssl: 3.3.0 -> 3.3.1"
        );
    }

    #[test]
    fn render_tree_depth_limit() {
        let nodes = vec![DiffNode::DepthLimit];
        let rendered = render_tree(&nodes, "  ", false);
        assert_eq!(rendered, "  └── [depth limit reached]");
    }

    #[test]
    fn render_tree_already_shown() {
        let nodes = vec![DiffNode::AlreadyShown {
            name: "glibc".to_string(),
        }];
        let rendered = render_tree(&nodes, "  ", false);
        assert_eq!(rendered, "  └── glibc [already shown]");
    }

    #[test]
    fn render_tree_build_change() {
        let nodes = vec![DiffNode::BuildChange {
            name: "zlib".to_string(),
            version: "1.3.1".to_string(),
            children: vec![],
        }];
        let rendered = render_tree(&nodes, "  ", false);
        assert_eq!(rendered, "  └── zlib: 1.3.1 (changed)");
    }

    #[test]
    fn match_inputs_finds_changes() {
        let mut old = HashMap::new();
        old.insert(
            "/nix/store/aaa-go-1.22.5.drv".to_string(),
            vec!["out".to_string()],
        );
        old.insert(
            "/nix/store/bbb-openssl-3.3.0.drv".to_string(),
            vec!["out".to_string()],
        );

        let mut new = HashMap::new();
        new.insert(
            "/nix/store/ccc-go-1.22.8.drv".to_string(),
            vec!["out".to_string()],
        );
        new.insert(
            "/nix/store/bbb-openssl-3.3.0.drv".to_string(),
            vec!["out".to_string()],
        );

        let (matched, added, removed) = match_inputs(&old, &new);

        // go changed (different drv path)
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].0, "go");

        // openssl unchanged (same path) — should NOT appear
        // No added or removed
        assert!(added.is_empty());
        assert!(removed.is_empty());
    }

    #[test]
    fn match_inputs_added_and_removed() {
        let mut old = HashMap::new();
        old.insert(
            "/nix/store/aaa-curl-8.9.0.drv".to_string(),
            vec!["out".to_string()],
        );

        let mut new = HashMap::new();
        new.insert(
            "/nix/store/bbb-wget-1.21.drv".to_string(),
            vec!["out".to_string()],
        );

        let (matched, added, removed) = match_inputs(&old, &new);

        assert!(matched.is_empty());
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].0, "wget");
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].0, "curl");
    }
}
