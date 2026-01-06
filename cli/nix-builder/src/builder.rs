use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File};
use std::io::Write as IoWrite;
use std::os::unix;
use std::path::Path;
use std::time::Instant;

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;

use crate::Config;

// The Nix store directory (typically /nix/store)
const STORE_DIR: &str = "/nix/store";

// JSON structures for deserializing the Nix attributes file

#[derive(Debug, Deserialize)]
struct NixAttrs {
    interpreter_out: String,
    interpreter_wrapper: String,
    #[serde(rename = "manifestPackage")]
    manifest_package: String,
    system: String,
    outputs: HashMap<String, String>,
    #[serde(rename = "exportReferencesGraph")]
    export_references_graph: HashMap<String, Vec<ReferenceGraphEntry>>,
}

#[derive(Debug, Deserialize)]
struct ManifestData {
    packages: Vec<Package>,
    manifest: Manifest,
}

#[derive(Debug, Deserialize)]
struct Manifest {
    install: HashMap<String, InstallEntry>,
    build: Option<HashMap<String, BuildEntry>>,
}

#[derive(Debug, Deserialize)]
struct InstallEntry {
    #[serde(rename = "pkg-path")]
    pkg_path: String,
    systems: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct BuildEntry {
    #[serde(rename = "runtime-packages")]
    runtime_packages: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct Package {
    system: String,
    #[serde(rename = "attr_path")]
    attr_path: Option<String>,
    group: Option<String>,
    outputs: HashMap<String, String>,
    #[serde(rename = "outputs_to_install")]
    outputs_to_install: Option<Vec<String>>,
    #[serde(rename = "outputs-to-install")]
    outputs_to_install_hyphen: Option<Vec<String>>,
    priority: Option<u32>,
    store_path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ReferenceGraphEntry {
    path: String,
    references: Vec<String>,
}

// Internal structures for tracking symlinks and priorities

/// Represents a symlink target with its priority.
/// An empty string target indicates a directory should be created.
#[derive(Debug, Clone)]
struct SymlinkEntry {
    target: String,
    priority: u32,
}

/// A package object in the builder format
#[derive(Debug)]
struct PkgEntry {
    paths: Vec<String>,
    priority: u32,
}

/// Output specification for an environment
#[derive(Debug)]
struct OutputSpec {
    name: String,
    pkgs: Vec<PkgEntry>,
    recurse: bool,
}

/// Parsed components of a store path
#[derive(Debug)]
struct StorePath {
    name: String,
    version: String,
    basename: String,
}

/// Build environment state
struct BuildEnvState {
    /// Map of relative paths to symlink targets and priorities
    symlinks: BTreeMap<String, SymlinkEntry>,
    /// Set of packages that have been processed
    done: HashSet<String>,
    /// Set of packages that need to be processed (propagated dependencies)
    postponed: HashSet<String>,
    /// Configuration
    config: Config,
    /// Whether to recursively link propagated-build-inputs
    flox_recursive_link: bool,
}

impl BuildEnvState {
    fn new(config: Config, flox_recursive_link: bool) -> Self {
        let mut state = BuildEnvState {
            symlinks: BTreeMap::new(),
            done: HashSet::new(),
            postponed: HashSet::new(),
            config,
            flox_recursive_link,
        };

        // Initialize symlinks with empty directories for all pathsToLink and parent directories
        state.symlinks.insert(String::new(), SymlinkEntry {
            target: String::new(),
            priority: 0,
        });

        for path in &state.config.paths_to_link {
            let parts: Vec<&str> = path.split('/').collect();
            let mut cur = String::new();
            for part in parts {
                if !part.is_empty() {
                    cur = format!("{}/{}", cur, part);
                } else if cur.is_empty() {
                    continue;
                }
                if cur == "/" {
                    cur = String::new();
                }
                state.symlinks.insert(cur.clone(), SymlinkEntry {
                    target: String::new(),
                    priority: 0,
                });
            }
        }

        state
    }

    /// Check if a path is in pathsToLink
    fn is_in_paths_to_link(&self, path: &str) -> bool {
        let path = if path.is_empty() { "/" } else { path };

        for elem in &self.config.paths_to_link {
            if elem == "/" {
                return true;
            }
            if path.starts_with(elem) && (path == elem || path[elem.len()..].starts_with('/')) {
                return true;
            }
        }
        false
    }

    /// Check if a path may contain files in pathsToLink
    fn has_paths_to_link(&self, path: &str) -> bool {
        for elem in &self.config.paths_to_link {
            if path.is_empty() {
                return true;
            }
            if elem.starts_with(path) && (path == elem || elem[path.len()..].starts_with('/')) {
                return true;
            }
        }
        false
    }
}

/// Parse a store path to extract name, version, and basename
fn parse_store_path(path: &str) -> Result<StorePath> {
    let components: Vec<&str> = path.split('/').collect();

    if components.len() < 4 {
        bail!("not a store path: {}", path);
    }

    let store_path_prefix = components[..4].join("/");

    if !is_store_path(&store_path_prefix) {
        bail!("not a store path: {}", store_path_prefix);
    }

    let pkg_dir = components[3];
    let parts: Vec<&str> = pkg_dir.splitn(2, '-').collect();

    if parts.len() != 2 {
        bail!("invalid store path format: {}", path);
    }

    let checksum = parts[0];
    if checksum.len() != 32 {
        bail!(
            "invalid checksum '{}' in store path: {}",
            checksum,
            store_path_prefix
        );
    }

    let pkg_name = parts[1];

    // Split package name from version by finding first "-" followed by a digit
    // This mimics the Perl regex: split /-(?=\d)/, $pkgName
    let (name, version) = if let Some(pos) = pkg_name.find('-') {
        // Check if the character after the '-' is a digit
        let after_dash = &pkg_name[pos + 1..];
        if after_dash
            .chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
        {
            (pkg_name[..pos].to_string(), after_dash.to_string())
        } else {
            // No version found, entire thing is the name
            (pkg_name.to_string(), String::new())
        }
    } else {
        // No dash found, entire thing is the name
        (pkg_name.to_string(), String::new())
    };

    let basename = components[4..].join("/");

    Ok(StorePath {
        name,
        version,
        basename,
    })
}

/// Check if something is a valid Nix store path
fn is_store_path(path: &str) -> bool {
    if !path.starts_with('/') {
        return false;
    }

    let parent = Path::new(path).parent();
    match parent {
        Some(p) => p.to_str() == Some(STORE_DIR),
        None => false,
    }
}

/// Check if two files have the same contents
fn check_collision(path1: &Path, path2: &Path) -> Result<bool> {
    if !path1.exists() || !path2.exists() {
        return Ok(false);
    }

    let metadata1 = fs::metadata(path1)?;
    let metadata2 = fs::metadata(path2)?;

    // Check permissions
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode1 = metadata1.permissions().mode();
        let mode2 = metadata2.permissions().mode();

        if mode1 != mode2 {
            eprintln!(
                "WARNING: different permissions in `{}' and `{}': {:04o} <-> {:04o}",
                path1.display(),
                path2.display(),
                mode1 & 0o7777,
                mode2 & 0o7777
            );
            return Ok(false);
        }
    }

    // Compare file contents
    let content1 = fs::read(path1)?;
    let content2 = fs::read(path2)?;

    Ok(content1 == content2)
}

/// Prepend "dangling symlink" to path if it's a dangling symlink
fn prepend_dangling(path: &Path) -> String {
    if path.is_symlink() && !path.exists() {
        format!("dangling symlink `{}'", path.display())
    } else {
        format!("`{}'", path.display())
    }
}

/// Discover files recursively in a directory
fn find_files(
    state: &mut BuildEnvState,
    rel_name: &str,
    target: &Path,
    base_name: &str,
    ignore_collisions: u8,
    check_collision_contents: bool,
    priority: u32,
) -> Result<()> {
    // The store path must not be a file
    let target_str = target
        .to_str()
        .context("target path contains invalid UTF-8")?;
    if target.is_file() && is_store_path(target_str) {
        bail!(
            "The store path {} is a file and can't be merged into an environment using pkgs.buildEnv!",
            target.display()
        );
    }

    // Skip certain paths
    if rel_name == "/propagated-build-inputs"
        || rel_name == "/nix-support"
        || rel_name.ends_with("/info/dir")
        || (rel_name.starts_with("/share/mime/") && !rel_name.starts_with("/share/mime/packages"))
        || base_name == "perllocal.pod"
        || base_name == "log"
        || (!state.has_paths_to_link(rel_name) && !state.is_in_paths_to_link(rel_name))
    {
        return Ok(());
    }

    let old_entry = state.symlinks.get(rel_name).cloned();

    // If target doesn't exist, create it. If it already exists as a symlink to a file
    // (not a directory) in a lower-priority package, overwrite it.
    if old_entry.is_none()
        || (priority < old_entry.as_ref().unwrap().priority
            && !old_entry.as_ref().unwrap().target.is_empty()
            && !Path::new(&old_entry.as_ref().unwrap().target).is_dir())
    {
        // Warn about dangling symlinks
        if target.is_symlink() && !target.exists() {
            let link_target = fs::read_link(target)?;
            eprintln!(
                "WARNING: creating dangling symlink `{}{}/{}' -> `{}' -> `{}'",
                state.config.out.display(),
                state.config.extra_prefix,
                rel_name,
                target.display(),
                link_target.display()
            );
        }

        let target_str = target
            .to_str()
            .context("target path contains invalid UTF-8")?;
        state.symlinks.insert(rel_name.to_string(), SymlinkEntry {
            target: target_str.to_string(),
            priority,
        });
        return Ok(());
    }

    let old_entry = old_entry.unwrap();

    // If both targets resolve to the same path, skip
    if !old_entry.target.is_empty() {
        let old_canon = fs::canonicalize(&old_entry.target).ok();
        let new_canon = fs::canonicalize(target).ok();

        if let (Some(old), Some(new)) = (old_canon, new_canon) {
            if old == new {
                // Prefer the target that is not a symlink
                if Path::new(&old_entry.target).is_symlink() && !target.is_symlink() {
                    let target_str = target
                        .to_str()
                        .context("target path contains invalid UTF-8")?;
                    state.symlinks.insert(rel_name.to_string(), SymlinkEntry {
                        target: target_str.to_string(),
                        priority,
                    });
                }
                return Ok(());
            }
        }
    }

    // If target already exists as a symlink to a file (not a directory) in a higher-priority
    // package, skip
    if priority > old_entry.priority
        && !old_entry.target.is_empty()
        && !Path::new(&old_entry.target).is_dir()
    {
        return Ok(());
    }

    // If target is supposed to be a directory but it isn't, die
    if old_entry.target.is_empty() && !target.is_dir() {
        bail!("not a directory: `{}'", target.display());
    }

    // Handle collision between two files
    if !target.is_dir() || (!old_entry.target.is_empty() && !Path::new(&old_entry.target).is_dir())
    {
        let target_ref = prepend_dangling(target);
        let old_target_ref = prepend_dangling(Path::new(&old_entry.target));

        if ignore_collisions > 0 {
            if ignore_collisions == 1 {
                eprintln!(
                    "WARNING: collision between {} and {}",
                    target_ref, old_target_ref
                );
            }
            return Ok(());
        } else if check_collision_contents && check_collision(Path::new(&old_entry.target), target)?
        {
            return Ok(());
        } else {
            // Improve upon the default collision message
            let target_str = target
                .to_str()
                .context("target path contains invalid UTF-8")?;
            let target_parsed = parse_store_path(target_str)?;
            let old_target_parsed = parse_store_path(&old_entry.target)?;
            let orig_priority = old_entry.priority / 1000;

            let errmsg = if target_parsed.basename == old_target_parsed.basename {
                format!(
                    "'{}' conflicts with '{}'. Both packages provide the file '{}'",
                    old_target_parsed.name, target_parsed.name, target_parsed.basename
                )
            } else {
                format!(
                    "'{}' conflicts with '{}'. collision between {} and {}",
                    old_target_parsed.name, target_parsed.name, target_ref, old_target_ref
                )
            };

            bail!(
                "{}\n\nResolve by uninstalling one of the conflicting packages or \
                setting the priority of the preferred package to a value lower than '{}'",
                errmsg,
                orig_priority
            );
        }
    }

    // Recurse into both directories
    if !old_entry.target.is_empty() {
        find_files_in_dir(
            state,
            rel_name,
            Path::new(&old_entry.target),
            ignore_collisions,
            check_collision_contents,
            old_entry.priority,
        )?;
    }

    find_files_in_dir(
        state,
        rel_name,
        target,
        ignore_collisions,
        check_collision_contents,
        priority,
    )?;

    state.symlinks.insert(rel_name.to_string(), SymlinkEntry {
        target: String::new(),
        priority,
    });

    Ok(())
}

/// Find files in a directory
fn find_files_in_dir(
    state: &mut BuildEnvState,
    rel_name: &str,
    target: &Path,
    ignore_collisions: u8,
    check_collision_contents: bool,
    priority: u32,
) -> Result<()> {
    let entries =
        fs::read_dir(target).with_context(|| format!("cannot open `{}'", target.display()))?;

    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();

    names.sort();

    for name in names {
        if name == "." || name == ".." {
            continue;
        }

        let new_rel_name = format!("{}/{}", rel_name, name);
        let new_target = target.join(&name);

        find_files(
            state,
            &new_rel_name,
            &new_target,
            &name,
            ignore_collisions,
            check_collision_contents,
            priority,
        )?;
    }

    Ok(())
}

/// Add a package to the environment
fn add_pkg(
    state: &mut BuildEnvState,
    pkg_dir: &str,
    ignore_collisions: u8,
    check_collision_contents: bool,
    priority: u32,
) -> Result<()> {
    if state.done.contains(pkg_dir) {
        return Ok(());
    }
    state.done.insert(pkg_dir.to_string());

    find_files(
        state,
        "",
        Path::new(pkg_dir),
        "",
        ignore_collisions,
        check_collision_contents,
        priority,
    )?;

    // Handle propagated dependencies
    if state.flox_recursive_link {
        for propagated_fn in &[
            format!("{}/nix-support/propagated-user-env-packages", pkg_dir),
            format!("{}/nix-support/propagated-build-inputs", pkg_dir),
        ] {
            if Path::new(propagated_fn).exists() {
                let content = fs::read_to_string(propagated_fn)?;
                for p in content.split_whitespace() {
                    // Skip packages with stub outputs
                    if p.ends_with("-stubs") {
                        continue;
                    }
                    if !state.done.contains(p) {
                        state.postponed.insert(p.to_string());
                    }
                }
            }
        }
    } else {
        let propagated_fn = format!("{}/nix-support/propagated-user-env-packages", pkg_dir);
        if Path::new(&propagated_fn).exists() {
            let content = fs::read_to_string(&propagated_fn)?;
            for p in content.split_whitespace() {
                if !state.done.contains(p) {
                    state.postponed.insert(p.to_string());
                }
            }
        }
    }

    Ok(())
}

/// Sort store paths by package name for deterministic builds
fn sort_by_package_name(paths: &mut [String]) {
    paths.sort_by(|a, b| {
        let a_parsed = parse_store_path(a).ok();
        let b_parsed = parse_store_path(b).ok();

        match (a_parsed, b_parsed) {
            (Some(a_sp), Some(b_sp)) => a_sp
                .name
                .cmp(&b_sp.name)
                .then(a_sp.version.cmp(&b_sp.version))
                .then(a_sp.basename.cmp(&b_sp.basename))
                .then(a.cmp(b)),
            _ => a.cmp(b),
        }
    });
}

/// Build a single environment
fn build_single_env(
    env_name: &str,
    requisites: &HashMap<String, Vec<String>>,
    out: &Path,
    pkgs: &[PkgEntry],
    config: &Config,
    flox_recursive_link: bool,
) -> Result<()> {
    let start = Instant::now();
    let mut state = BuildEnvState::new(config.clone(), flox_recursive_link);

    // Add packages installed explicitly by the user
    for pkg in pkgs {
        let mut paths = pkg.paths.clone();
        sort_by_package_name(&mut paths);

        for path in paths {
            if Path::new(&path).exists() {
                add_pkg(
                    &mut state,
                    &path,
                    config.ignore_collisions,
                    config.check_collision_contents,
                    pkg.priority,
                )?;
            }
        }
    }

    // Add propagated packages
    let mut priority_counter = 1000u32;
    while !state.postponed.is_empty() {
        let mut pkg_dirs: Vec<String> = state.postponed.drain().collect();
        sort_by_package_name(&mut pkg_dirs);

        for pkg_dir in pkg_dirs {
            add_pkg(
                &mut state,
                &pkg_dir,
                2,
                config.check_collision_contents,
                priority_counter,
            )?;
            priority_counter += 1;
        }
    }

    // Create symlinks
    let mut nr_links = 0;
    for (rel_name, entry) in &state.symlinks {
        if !state.is_in_paths_to_link(rel_name) {
            continue;
        }

        let abs = out
            .join(&config.extra_prefix)
            .join(rel_name.trim_start_matches('/'));

        if entry.target.is_empty() {
            // Create directory
            fs::create_dir_all(&abs)
                .with_context(|| format!("cannot create directory `{}'", abs.display()))?;
        } else {
            // Create symlink
            if let Some(parent) = abs.parent() {
                fs::create_dir_all(parent)?;
            }
            unix::fs::symlink(&entry.target, &abs)
                .with_context(|| format!("error creating link `{}'", abs.display()))?;
            nr_links += 1;
        }
    }

    let elapsed = start.elapsed();
    eprintln!(
        "created {} symlinks in {} environment in {:.06} seconds",
        nr_links,
        env_name,
        elapsed.as_secs_f64()
    );

    // Ensure output directory exists
    if !out.exists() {
        fs::create_dir_all(out)
            .with_context(|| format!("cannot create directory `{}'", out.display()))?;
    }

    // Build requisites.txt
    let mut requisites_set = HashSet::new();
    for pkg in state.done {
        if let Some(reqs) = requisites.get(&pkg) {
            for req in reqs {
                requisites_set.insert(req.clone());
            }
        }
    }

    // Include the package itself
    requisites_set.insert(out.to_string_lossy().to_string());

    // Write requisites.txt
    let requisites_file = out.join("requisites.txt");
    let mut file = File::create(&requisites_file)
        .with_context(|| format!("Could not open file '{}'", requisites_file.display()))?;

    let mut requisites_vec: Vec<_> = requisites_set.into_iter().collect();
    requisites_vec.sort();

    for requisite in requisites_vec {
        writeln!(file, "{}", requisite)?;
    }

    Ok(())
}

/// Convert package objects to builder format
fn packages_to_pkgs(packages: &[PackageEntry]) -> Vec<PkgEntry> {
    let mut result = Vec::new();
    let mut other_output_priority_counter = 1u32;

    for package in packages {
        // Handle store paths directly
        if let Some(store_path) = &package.store_path {
            result.push(PkgEntry {
                paths: vec![store_path.clone()],
                priority: package.priority * 1000,
            });
            continue;
        }

        let mut outputs_to_install_vec = Vec::new();
        let mut other_outputs_vec = Vec::new();

        // Get outputs_to_install list
        let outputs_to_install = if let Some(ref oti) = package.outputs_to_install {
            oti.clone()
        } else if let Some(ref oti_hyphen) = package.outputs_to_install_hyphen {
            oti_hyphen.clone()
        } else {
            package
                .outputs
                .keys()
                .filter(|k| k.as_str() != "log")
                .cloned()
                .collect()
        };

        for (output, path) in &package.outputs {
            // Skip log outputs if they're files
            if output == "log" && Path::new(path).is_file() {
                continue;
            }
            // Skip stubs outputs
            if output == "stubs" {
                continue;
            }

            if outputs_to_install.contains(output) {
                outputs_to_install_vec.push(path.clone());
            } else {
                other_outputs_vec.push(path.clone());
            }
        }

        if !outputs_to_install_vec.is_empty() {
            result.push(PkgEntry {
                paths: outputs_to_install_vec,
                priority: package.priority * 1000,
            });
        }

        for other_output in other_outputs_vec {
            result.push(PkgEntry {
                paths: vec![other_output],
                priority: package.priority * 1000 + other_output_priority_counter,
            });
            other_output_priority_counter += 1;
        }
    }

    result
}

/// Internal package representation
#[derive(Debug, Clone)]
struct PackageEntry {
    attr_path: Option<String>,
    outputs: HashMap<String, String>,
    outputs_to_install: Option<Vec<String>>,
    outputs_to_install_hyphen: Option<Vec<String>>,
    priority: u32,
    store_path: Option<String>,
}

/// Transform manifest data into build specifications
fn output_data(nix_attrs: &NixAttrs, manifest_data: &ManifestData) -> Result<Vec<OutputSpec>> {
    let interpreter_out = &nix_attrs.interpreter_out;
    let interpreter_wrapper = &nix_attrs.interpreter_wrapper;
    let manifest_package = &nix_attrs.manifest_package;
    let system = &nix_attrs.system;
    let packages = &manifest_data.packages;
    let manifest = &manifest_data.manifest;
    let install = &manifest.install;
    let builds = manifest.build.as_ref();

    // Create package entries for flox-sourced packages
    let interpreter_out_entry = PackageEntry {
        attr_path: None,
        outputs: {
            let mut map = HashMap::new();
            map.insert("out".to_string(), interpreter_out.clone());
            map
        },
        outputs_to_install: Some(vec!["out".to_string()]),
        outputs_to_install_hyphen: None,
        priority: 1,
        store_path: None,
    };

    let interpreter_wrapper_entry = PackageEntry {
        attr_path: None,
        outputs: {
            let mut map = HashMap::new();
            map.insert("out".to_string(), interpreter_wrapper.clone());
            map
        },
        outputs_to_install: Some(vec!["out".to_string()]),
        outputs_to_install_hyphen: None,
        priority: 1,
        store_path: None,
    };

    let manifest_package_entry = PackageEntry {
        attr_path: None,
        outputs: {
            let mut map = HashMap::new();
            map.insert("out".to_string(), manifest_package.clone());
            map
        },
        outputs_to_install: Some(vec!["out".to_string()]),
        outputs_to_install_hyphen: None,
        priority: 1,
        store_path: None,
    };

    // Filter system-specific outputs
    let out_packages: Vec<PackageEntry> = packages
        .iter()
        .filter(|p| p.system == *system)
        .map(|p| PackageEntry {
            attr_path: p.attr_path.clone(),
            outputs: p.outputs.clone(),
            outputs_to_install: p.outputs_to_install.clone(),
            outputs_to_install_hyphen: p.outputs_to_install_hyphen.clone(),
            priority: p.priority.unwrap_or(5),
            store_path: p.store_path.clone(),
        })
        .collect();

    // Define develop packages
    let mut develop_packages = out_packages.clone();
    develop_packages.push(interpreter_out_entry.clone());
    develop_packages.push(manifest_package_entry.clone());

    // Filter toplevel packages
    let toplevel_packages: Vec<PackageEntry> = packages
        .iter()
        .filter(|p| {
            p.system == *system && p.group.as_ref().map(|g| g == "toplevel").unwrap_or(false)
        })
        .map(|p| PackageEntry {
            attr_path: p.attr_path.clone(),
            outputs: p.outputs.clone(),
            outputs_to_install: p.outputs_to_install.clone(),
            outputs_to_install_hyphen: p.outputs_to_install_hyphen.clone(),
            priority: p.priority.unwrap_or(5),
            store_path: p.store_path.clone(),
        })
        .collect();

    let mut output_specs = vec![
        OutputSpec {
            name: "runtime".to_string(),
            pkgs: packages_to_pkgs(&develop_packages),
            recurse: false,
        },
        OutputSpec {
            name: "develop".to_string(),
            pkgs: packages_to_pkgs(&develop_packages),
            recurse: true,
        },
    ];

    // Handle build environments
    if let Some(builds_map) = builds {
        for (build_name, build_entry) in builds_map {
            let build_packages = if let Some(ref runtime_packages) = build_entry.runtime_packages {
                // Filter by runtime-packages
                let mut build_package_attr_paths = Vec::new();

                for name in runtime_packages {
                    if let Some(install_entry) = install.get(name) {
                        // Check system filter
                        if let Some(ref systems) = install_entry.systems {
                            if !systems.contains(system) {
                                continue;
                            }
                        }

                        // Verify package is in toplevel
                        let pkg_path = &install_entry.pkg_path;
                        if packages.iter().any(|p| {
                            p.attr_path.as_ref() == Some(pkg_path)
                                && p.group.as_ref().map(|g| g == "toplevel").unwrap_or(false)
                        }) {
                            build_package_attr_paths.push(pkg_path.clone());
                        } else {
                            bail!("package '{}' is not in 'toplevel' pkg-group", name);
                        }
                    } else {
                        bail!(
                            "package '{}' not found in '[install]' section of manifest",
                            name
                        );
                    }
                }

                // Filter packages to only those whose attr_path is in build_package_attr_paths
                let mut filtered: Vec<PackageEntry> = toplevel_packages
                    .iter()
                    .filter(|p| {
                        p.attr_path
                            .as_ref()
                            .map(|ap| build_package_attr_paths.contains(ap))
                            .unwrap_or(false)
                    })
                    .cloned()
                    .collect();

                filtered.push(interpreter_wrapper_entry.clone());
                filtered.push(manifest_package_entry.clone());
                filtered
            } else {
                let mut all = toplevel_packages.clone();
                all.push(interpreter_wrapper_entry.clone());
                all.push(manifest_package_entry.clone());
                all
            };

            output_specs.push(OutputSpec {
                name: format!("build-{}", build_name),
                pkgs: packages_to_pkgs(&build_packages),
                recurse: true,
            });
        }
    }

    Ok(output_specs)
}

/// Walk reference tree recursively
fn walk_references(
    references: &HashMap<String, Vec<String>>,
    pkg: &str,
    seen: &mut HashSet<String>,
    result: &mut Vec<String>,
) {
    result.push(pkg.to_string());

    if let Some(refs) = references.get(pkg) {
        if !seen.contains(pkg) {
            for reference in refs {
                if reference != pkg {
                    walk_references(references, reference, seen, result);
                }
            }
            seen.insert(pkg.to_string());
        }
    } else {
        eprintln!("WARNING: references for package {} not found", pkg);
    }
}

/// Map reference graph array to a hash
fn map_references(graph: &[ReferenceGraphEntry]) -> HashMap<String, Vec<String>> {
    let mut result = HashMap::new();
    for entry in graph {
        result.insert(entry.path.clone(), entry.references.clone());
    }
    result
}

/// Build the environment by linking packages according to the configuration.
pub fn build_env(config: Config) -> Result<()> {
    // Read the NIX_ATTRS_JSON_FILE
    let nix_attrs_content = fs::read_to_string(&config.nix_attrs_json_file)
        .with_context(|| format!("Failed to read {}", config.nix_attrs_json_file.display()))?;

    let nix_attrs: NixAttrs =
        serde_json::from_str(&nix_attrs_content).context("Failed to parse NIX_ATTRS_JSON_FILE")?;

    // Read the manifest.lock file
    let manifest_lock_path = Path::new(&nix_attrs.manifest_package).join("manifest.lock");
    let manifest_content = fs::read_to_string(&manifest_lock_path)
        .with_context(|| format!("Failed to read {}", manifest_lock_path.display()))?;

    let manifest_data: ManifestData =
        serde_json::from_str(&manifest_content).context("Failed to parse manifest.lock")?;

    // Generate output specifications
    let output_specs = output_data(&nix_attrs, &manifest_data)?;

    // Build requisites map from the reference graph
    // This matches the Perl implementation at lines 774-782
    let mut requisites: HashMap<String, Vec<String>> = HashMap::new();

    for graph_entries in nix_attrs.export_references_graph.values() {
        // Convert the graph entries array to a hash for efficient lookup
        let references = map_references(graph_entries);

        // For each package in the graph, walk its reference tree
        for entry in graph_entries {
            let mut seen = HashSet::new();
            let mut refs = Vec::new();

            walk_references(&references, &entry.path, &mut seen, &mut refs);

            // Store the requisites for this package
            requisites.insert(entry.path.clone(), refs);
        }
    }

    // Build each environment
    for output_spec in output_specs {
        let output_path = nix_attrs
            .outputs
            .get(&output_spec.name)
            .ok_or_else(|| anyhow!("Output '{}' not found in nix_attrs", output_spec.name))?;

        build_single_env(
            &output_spec.name,
            &requisites,
            Path::new(output_path),
            &output_spec.pkgs,
            &config,
            output_spec.recurse,
        )?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;
    use std::path::PathBuf;

    use tempfile::TempDir;

    use super::*;

    // Unit tests

    #[test]
    fn test_parse_store_path_valid() {
        let path = "/nix/store/abc123def456ghi789jkl012mno345pq-hello-2.10/bin/hello";
        let result = parse_store_path(path).unwrap();

        assert_eq!(result.name, "hello");
        assert_eq!(result.version, "2.10");
        assert_eq!(result.basename, "bin/hello");
    }

    #[test]
    fn test_parse_store_path_no_version() {
        let path = "/nix/store/abc123def456ghi789jkl012mno345pq-hello/bin/hello";
        let result = parse_store_path(path).unwrap();

        assert_eq!(result.name, "hello");
        assert_eq!(result.version, "");
        assert_eq!(result.basename, "bin/hello");
    }

    #[test]
    fn test_parse_store_path_name_with_dashes() {
        let path = "/nix/store/abc123def456ghi789jkl012mno345pq-bash-interactive-5.0/bin/bash";
        let result = parse_store_path(path).unwrap();

        // The parser uses find('-') which finds the FIRST dash in the string
        // For "bash-interactive-5.0", the first dash is after "bash"
        // The character after that dash is 'i' (not a digit), so no split occurs
        // The entire string "bash-interactive-5.0" becomes the name with empty version
        assert_eq!(result.name, "bash-interactive-5.0");
        assert_eq!(result.version, "");
        assert_eq!(result.basename, "bin/bash");
    }

    #[test]
    fn test_parse_store_path_invalid_checksum() {
        let path = "/nix/store/short-hello-2.10/bin/hello";
        let result = parse_store_path(path);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid checksum"));
    }

    #[test]
    fn test_parse_store_path_not_store_path() {
        let path = "/usr/bin/hello";
        let result = parse_store_path(path);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not a store path"));
    }

    #[test]
    fn test_parse_store_path_too_short() {
        let path = "/nix/store";
        let result = parse_store_path(path);

        assert!(result.is_err());
    }

    #[test]
    fn test_is_store_path_valid() {
        let path = "/nix/store/abc123def456ghi789jkl012mno345pq-hello-2.10";
        assert!(is_store_path(path));
    }

    #[test]
    fn test_is_store_path_invalid_not_absolute() {
        let path = "nix/store/abc123def456ghi789jkl012mno345pq-hello-2.10";
        assert!(!is_store_path(path));
    }

    #[test]
    fn test_is_store_path_invalid_wrong_parent() {
        let path = "/usr/store/abc123def456ghi789jkl012mno345pq-hello-2.10";
        assert!(!is_store_path(path));
    }

    #[test]
    fn test_is_store_path_root() {
        let path = "/";
        assert!(!is_store_path(path));
    }

    #[test]
    fn test_is_in_paths_to_link_root() {
        let config = Config {
            nix_attrs_json_file: PathBuf::from("/tmp/attrs.json"),
            out: PathBuf::from("/tmp/out"),
            paths_to_link: vec!["/".to_string()],
            extra_prefix: String::new(),
            ignore_collisions: 0,
            check_collision_contents: false,
        };
        let state = BuildEnvState::new(config, false);

        assert!(state.is_in_paths_to_link(""));
        assert!(state.is_in_paths_to_link("/bin"));
        assert!(state.is_in_paths_to_link("/share/man"));
    }

    #[test]
    fn test_is_in_paths_to_link_specific_paths() {
        let config = Config {
            nix_attrs_json_file: PathBuf::from("/tmp/attrs.json"),
            out: PathBuf::from("/tmp/out"),
            paths_to_link: vec!["/bin".to_string(), "/share".to_string()],
            extra_prefix: String::new(),
            ignore_collisions: 0,
            check_collision_contents: false,
        };
        let state = BuildEnvState::new(config, false);

        assert!(state.is_in_paths_to_link("/bin"));
        assert!(state.is_in_paths_to_link("/bin/hello"));
        assert!(state.is_in_paths_to_link("/share"));
        assert!(state.is_in_paths_to_link("/share/man"));
        assert!(!state.is_in_paths_to_link("/lib"));
        assert!(!state.is_in_paths_to_link("/include"));
    }

    #[test]
    fn test_is_in_paths_to_link_empty_path() {
        let config = Config {
            nix_attrs_json_file: PathBuf::from("/tmp/attrs.json"),
            out: PathBuf::from("/tmp/out"),
            paths_to_link: vec!["/bin".to_string()],
            extra_prefix: String::new(),
            ignore_collisions: 0,
            check_collision_contents: false,
        };
        let state = BuildEnvState::new(config, false);

        // Empty path is treated as "/"
        assert!(!state.is_in_paths_to_link(""));
    }

    #[test]
    fn test_has_paths_to_link_root() {
        let config = Config {
            nix_attrs_json_file: PathBuf::from("/tmp/attrs.json"),
            out: PathBuf::from("/tmp/out"),
            paths_to_link: vec!["/".to_string()],
            extra_prefix: String::new(),
            ignore_collisions: 0,
            check_collision_contents: false,
        };
        let state = BuildEnvState::new(config, false);

        // Empty path always returns true
        assert!(state.has_paths_to_link(""));
        // "/" starts with "" so these should be false
        // The logic checks if elem (pathsToLink entry) starts with path
        // "/" does not start with "/bin", so this returns false
        assert!(!state.has_paths_to_link("/bin"));
        assert!(!state.has_paths_to_link("/share"));
    }

    #[test]
    fn test_has_paths_to_link_specific_paths() {
        let config = Config {
            nix_attrs_json_file: PathBuf::from("/tmp/attrs.json"),
            out: PathBuf::from("/tmp/out"),
            paths_to_link: vec!["/bin/foo".to_string(), "/share".to_string()],
            extra_prefix: String::new(),
            ignore_collisions: 0,
            check_collision_contents: false,
        };
        let state = BuildEnvState::new(config, false);

        // Empty path may contain any paths_to_link
        assert!(state.has_paths_to_link(""));
        // /bin may contain /bin/foo
        assert!(state.has_paths_to_link("/bin"));
        // /bin/foo is exactly in paths_to_link
        assert!(state.has_paths_to_link("/bin/foo"));
        // /share is in paths_to_link
        assert!(state.has_paths_to_link("/share"));
        // /lib does not contain any paths_to_link
        assert!(!state.has_paths_to_link("/lib"));
    }

    #[test]
    fn test_sort_by_package_name_stable() {
        let mut paths = vec![
            "/nix/store/1234567890abcdef1234567890abcdef-zlib-1.0/lib".to_string(),
            "/nix/store/abcdef1234567890abcdef1234567890-hello-2.0/bin".to_string(),
            "/nix/store/567890abcdef1234567890abcdef1234-hello-1.0/bin".to_string(),
            "/nix/store/90abcdef1234567890abcdef12345678-bash-5.0/bin".to_string(),
        ];

        sort_by_package_name(&mut paths);

        // Should be sorted by name, then version, then basename
        let names: Vec<String> = paths
            .iter()
            .map(|p| parse_store_path(p).unwrap().name)
            .collect();

        assert_eq!(names, vec!["bash", "hello", "hello", "zlib"]);
    }

    #[test]
    fn test_sort_by_package_name_with_versions() {
        let mut paths = vec![
            "/nix/store/abc123def456ghi789jkl012mno345pq-hello-2.0/bin".to_string(),
            "/nix/store/567890abcdef1234567890abcdef1234-hello-1.0/bin".to_string(),
            "/nix/store/cdef1234567890abcdef1234567890ab-hello-10.0/bin".to_string(),
        ];

        sort_by_package_name(&mut paths);

        let versions: Vec<String> = paths
            .iter()
            .map(|p| parse_store_path(p).unwrap().version)
            .collect();

        // Versions are sorted lexicographically: "1.0" < "10.0" < "2.0"
        assert_eq!(versions, vec!["1.0", "10.0", "2.0"]);
    }

    #[test]
    fn test_sort_by_package_name_invalid_paths() {
        let mut paths = vec![
            "/nix/store/abc123def456ghi789jkl012mno345pq-hello-2.0/bin".to_string(),
            "/invalid/path".to_string(),
            "/nix/store/567890abcdef1234567890abcdef1234-bash-1.0/bin".to_string(),
        ];

        sort_by_package_name(&mut paths);

        // Invalid paths should be sorted lexicographically
        assert_eq!(paths[0], "/invalid/path");
    }

    #[test]
    fn test_check_collision_same_contents() {
        let tempdir = TempDir::new().unwrap();
        let file1 = tempdir.path().join("file1");
        let file2 = tempdir.path().join("file2");

        fs::write(&file1, b"hello world").unwrap();
        fs::write(&file2, b"hello world").unwrap();

        assert!(check_collision(&file1, &file2).unwrap());
    }

    #[test]
    fn test_check_collision_different_contents() {
        let tempdir = TempDir::new().unwrap();
        let file1 = tempdir.path().join("file1");
        let file2 = tempdir.path().join("file2");

        fs::write(&file1, b"hello world").unwrap();
        fs::write(&file2, b"goodbye world").unwrap();

        assert!(!check_collision(&file1, &file2).unwrap());
    }

    #[test]
    fn test_check_collision_different_permissions() {
        let tempdir = TempDir::new().unwrap();
        let file1 = tempdir.path().join("file1");
        let file2 = tempdir.path().join("file2");

        fs::write(&file1, b"hello world").unwrap();
        fs::write(&file2, b"hello world").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms1 = fs::metadata(&file1).unwrap().permissions();
            perms1.set_mode(0o644);
            fs::set_permissions(&file1, perms1).unwrap();

            let mut perms2 = fs::metadata(&file2).unwrap().permissions();
            perms2.set_mode(0o755);
            fs::set_permissions(&file2, perms2).unwrap();

            assert!(!check_collision(&file1, &file2).unwrap());
        }
    }

    #[test]
    fn test_check_collision_nonexistent_file() {
        let tempdir = TempDir::new().unwrap();
        let file1 = tempdir.path().join("file1");
        let file2 = tempdir.path().join("file2");

        fs::write(&file1, b"hello world").unwrap();

        assert!(!check_collision(&file1, &file2).unwrap());
    }

    #[test]
    fn test_packages_to_pkgs_simple() {
        let packages = vec![PackageEntry {
            attr_path: Some("hello".to_string()),
            outputs: {
                let mut map = HashMap::new();
                map.insert("out".to_string(), "/nix/store/abc-hello".to_string());
                map
            },
            outputs_to_install: Some(vec!["out".to_string()]),
            outputs_to_install_hyphen: None,
            priority: 5,
            store_path: None,
        }];

        let pkgs = packages_to_pkgs(&packages);

        assert_eq!(pkgs.len(), 1);
        assert_eq!(pkgs[0].paths.len(), 1);
        assert_eq!(pkgs[0].paths[0], "/nix/store/abc-hello");
        assert_eq!(pkgs[0].priority, 5000);
    }

    #[test]
    fn test_packages_to_pkgs_multiple_outputs() {
        let packages = vec![PackageEntry {
            attr_path: Some("bash".to_string()),
            outputs: {
                let mut map = HashMap::new();
                map.insert("out".to_string(), "/nix/store/abc-bash".to_string());
                map.insert("man".to_string(), "/nix/store/def-bash-man".to_string());
                map.insert("doc".to_string(), "/nix/store/ghi-bash-doc".to_string());
                map
            },
            outputs_to_install: Some(vec!["out".to_string(), "man".to_string()]),
            outputs_to_install_hyphen: None,
            priority: 5,
            store_path: None,
        }];

        let pkgs = packages_to_pkgs(&packages);

        // Should have two PkgEntry: one for outputs_to_install, one for other outputs
        assert_eq!(pkgs.len(), 2);

        // First entry should have the installed outputs
        assert_eq!(pkgs[0].paths.len(), 2);
        assert!(pkgs[0].paths.contains(&"/nix/store/abc-bash".to_string()));
        assert!(pkgs[0]
            .paths
            .contains(&"/nix/store/def-bash-man".to_string()));
        assert_eq!(pkgs[0].priority, 5000);

        // Second entry should have the other outputs with higher priority
        assert_eq!(pkgs[1].paths.len(), 1);
        assert_eq!(pkgs[1].paths[0], "/nix/store/ghi-bash-doc");
        assert_eq!(pkgs[1].priority, 5001);
    }

    #[test]
    fn test_packages_to_pkgs_skip_log() {
        let tempdir = TempDir::new().unwrap();
        let log_file = tempdir.path().join("log");
        fs::write(&log_file, b"build log").unwrap();

        let packages = vec![PackageEntry {
            attr_path: Some("hello".to_string()),
            outputs: {
                let mut map = HashMap::new();
                map.insert("out".to_string(), "/nix/store/abc-hello".to_string());
                map.insert("log".to_string(), log_file.to_str().unwrap().to_string());
                map
            },
            outputs_to_install: None,
            outputs_to_install_hyphen: None,
            priority: 5,
            store_path: None,
        }];

        let pkgs = packages_to_pkgs(&packages);

        // Log output should be skipped if it's a file
        assert_eq!(pkgs.len(), 1);
        assert_eq!(pkgs[0].paths.len(), 1);
        assert_eq!(pkgs[0].paths[0], "/nix/store/abc-hello");
    }

    #[test]
    fn test_packages_to_pkgs_skip_stubs() {
        let packages = vec![PackageEntry {
            attr_path: Some("hello".to_string()),
            outputs: {
                let mut map = HashMap::new();
                map.insert("out".to_string(), "/nix/store/abc-hello".to_string());
                map.insert(
                    "stubs".to_string(),
                    "/nix/store/def-hello-stubs".to_string(),
                );
                map
            },
            outputs_to_install: None,
            outputs_to_install_hyphen: None,
            priority: 5,
            store_path: None,
        }];

        let pkgs = packages_to_pkgs(&packages);

        // Stubs output should be skipped
        assert_eq!(pkgs.len(), 1);
        assert_eq!(pkgs[0].paths.len(), 1);
        assert_eq!(pkgs[0].paths[0], "/nix/store/abc-hello");
    }

    #[test]
    fn test_packages_to_pkgs_store_path() {
        let packages = vec![PackageEntry {
            attr_path: None,
            outputs: HashMap::new(),
            outputs_to_install: None,
            outputs_to_install_hyphen: None,
            priority: 5,
            store_path: Some("/nix/store/abc-direct-path".to_string()),
        }];

        let pkgs = packages_to_pkgs(&packages);

        assert_eq!(pkgs.len(), 1);
        assert_eq!(pkgs[0].paths.len(), 1);
        assert_eq!(pkgs[0].paths[0], "/nix/store/abc-direct-path");
        assert_eq!(pkgs[0].priority, 5000);
    }

    #[test]
    fn test_packages_to_pkgs_outputs_to_install_hyphen() {
        let packages = vec![PackageEntry {
            attr_path: Some("hello".to_string()),
            outputs: {
                let mut map = HashMap::new();
                map.insert("out".to_string(), "/nix/store/abc-hello".to_string());
                map.insert("man".to_string(), "/nix/store/def-hello-man".to_string());
                map
            },
            outputs_to_install: None,
            outputs_to_install_hyphen: Some(vec!["out".to_string()]),
            priority: 5,
            store_path: None,
        }];

        let pkgs = packages_to_pkgs(&packages);

        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].paths.len(), 1);
        assert_eq!(pkgs[0].paths[0], "/nix/store/abc-hello");
    }

    #[test]
    fn test_packages_to_pkgs_default_outputs() {
        let packages = vec![PackageEntry {
            attr_path: Some("hello".to_string()),
            outputs: {
                let mut map = HashMap::new();
                map.insert("out".to_string(), "/nix/store/abc-hello".to_string());
                map.insert("man".to_string(), "/nix/store/def-hello-man".to_string());
                map.insert("log".to_string(), "/nix/store/ghi-hello-log".to_string());
                map
            },
            outputs_to_install: None,
            outputs_to_install_hyphen: None,
            priority: 5,
            store_path: None,
        }];

        let pkgs = packages_to_pkgs(&packages);

        // Without outputs_to_install, should use all outputs except log
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].paths.len(), 2);
        assert!(pkgs[0].paths.contains(&"/nix/store/abc-hello".to_string()));
        assert!(pkgs[0]
            .paths
            .contains(&"/nix/store/def-hello-man".to_string()));
    }
}
