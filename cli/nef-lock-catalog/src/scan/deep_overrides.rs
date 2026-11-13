use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// The marker file that promotes a package directory to a deep override,
/// mirroring `deepOverrideMarkerFile` in
/// `package-builder/nef/lib/deepOverrides.nix`.
const DEEP_OVERRIDE_MARKER_FILE: &str = "deep-override";

/// One name's classified `pkgs/` entry, for the collision rule
/// [deep_override_attr_paths_in] follows: a `.nix` file replaces a
/// same-named directory in the scan, never the reverse.
#[derive(Debug, Clone)]
enum PkgsEntry {
    NixFile,
    Directory(PathBuf),
}

/// Attribute paths of the deep overrides declared under a `pkgs/`
/// directory, as a lock entry's `deep_overrides` records them.
///
/// Mirrors how NEF classifies `pkgs/` entries
/// (`package-builder/nef/lib/dirToAttrs.nix`) and marks one as a deep
/// override (`isMarkedAsDeepOverride` in
/// `package-builder/nef/lib/deepOverrides.nix`): a directory holding a
/// `default.nix` is a package, and a package is a deep override iff it
/// also holds a `deep-override` file beside that `default.nix`. A
/// package directory is never recursed into. A directory without
/// `default.nix` is a package set and is recursed into, so a nested
/// override is reported with its full path, e.g.
/// `["setMakeScope", "makeScopeDependency"]`. A `.nix` file has no
/// directory of its own to hold the marker, so it is never an override.
/// Where a `.nix` file and a directory share a name, the file wins the name
/// (`dirToAttrs.nix`), so the directory is not scanned at all, marked or
/// not.
///
/// An absent `pkgs/` directory is a repository with no overrides, so
/// this returns an empty list rather than an error. The result is
/// sorted.
pub fn deep_override_attr_paths_in(pkgs_dir: impl AsRef<Path>) -> Vec<Vec<String>> {
    let mut attr_paths = Vec::new();
    scan_pkgs_dir_for_deep_overrides(pkgs_dir.as_ref(), &mut Vec::new(), &mut attr_paths);
    attr_paths.sort();
    attr_paths
}

/// Recursive worker for [deep_override_attr_paths_in]. `attr_path` is the
/// path to `dir` from the `pkgs/` root, mutated in place as a scratch
/// stack rather than rebuilt per call, and restored before returning so
/// a sibling directory sees the prefix as it found it.
fn scan_pkgs_dir_for_deep_overrides(
    dir: &Path,
    attr_path: &mut Vec<String>,
    out: &mut Vec<Vec<String>>,
) {
    let Ok(read_dir) = fs::read_dir(dir) else {
        return;
    };
    let entries: Vec<PathBuf> = read_dir.flatten().map(|entry| entry.path()).collect();

    // Two passes so a `.nix` file always wins its name over a same-named
    // directory, regardless of the order `read_dir` happens to yield them.
    let mut by_name: BTreeMap<String, PkgsEntry> = BTreeMap::new();
    for path in &entries {
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        by_name.insert(name.to_string(), PkgsEntry::Directory(path.clone()));
    }
    for path in &entries {
        if path.extension().and_then(|ext| ext.to_str()) != Some("nix") {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        // dirToAttrs.nix excludes any file ending in "flake.nix" from the
        // scan, not only a file named exactly that.
        if file_name.ends_with("flake.nix") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        by_name.insert(stem.to_string(), PkgsEntry::NixFile);
    }

    for (name, entry) in by_name {
        let PkgsEntry::Directory(path) = entry else {
            continue;
        };
        if !path.join("default.nix").is_file() {
            attr_path.push(name);
            scan_pkgs_dir_for_deep_overrides(&path, attr_path, out);
            attr_path.pop();
            continue;
        }
        if path.join(DEEP_OVERRIDE_MARKER_FILE).is_file() {
            let mut marked_path = attr_path.clone();
            marked_path.push(name);
            out.push(marked_path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A package directory marked with `deep-override` beside its
    /// `default.nix` is reported, as a single-component attribute path.
    #[test]
    fn marked_package_directory_is_listed() {
        let pkgs = tempfile::tempdir().unwrap();
        fs::create_dir_all(pkgs.path().join("openssl")).unwrap();
        fs::write(pkgs.path().join("openssl").join("default.nix"), "").unwrap();
        fs::write(pkgs.path().join("openssl").join("deep-override"), "").unwrap();

        let paths = deep_override_attr_paths_in(pkgs.path());

        assert_eq!(paths, vec![vec!["openssl".to_string()]]);
    }

    /// A package directory without the marker file is a shallow
    /// package, not reported.
    #[test]
    fn unmarked_package_directory_is_not_listed() {
        let pkgs = tempfile::tempdir().unwrap();
        fs::create_dir_all(pkgs.path().join("zlib")).unwrap();
        fs::write(pkgs.path().join("zlib").join("default.nix"), "").unwrap();

        let paths = deep_override_attr_paths_in(pkgs.path());

        assert_eq!(paths, Vec::<Vec<String>>::new());
    }

    /// A lone `.nix` file has no directory of its own to hold the
    /// marker, so a `deep-override` file sitting beside it at the same
    /// `pkgs/` level attaches to nothing.
    #[test]
    fn nix_file_beside_a_deep_override_file_is_not_listed() {
        let pkgs = tempfile::tempdir().unwrap();
        fs::write(pkgs.path().join("foo.nix"), "").unwrap();
        fs::write(pkgs.path().join("deep-override"), "").unwrap();

        let paths = deep_override_attr_paths_in(pkgs.path());

        assert_eq!(paths, Vec::<Vec<String>>::new());
    }

    /// A marked package nested inside a package set (a directory with
    /// no `default.nix` of its own) is reported with its full
    /// attribute path, not just its own name.
    #[test]
    fn nested_override_in_a_package_set_is_listed_with_full_path() {
        let pkgs = tempfile::tempdir().unwrap();
        let nested = pkgs.path().join("setMakeScope").join("makeScopeDependency");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("default.nix"), "").unwrap();
        fs::write(nested.join("deep-override"), "").unwrap();

        let paths = deep_override_attr_paths_in(pkgs.path());

        assert_eq!(paths, vec![vec![
            "setMakeScope".to_string(),
            "makeScopeDependency".to_string(),
        ]]);
    }

    /// A package directory is never recursed into, even when it holds
    /// a nested directory that would otherwise look like a marked
    /// package-set member.
    #[test]
    fn subdirectory_of_a_marked_package_is_not_scanned() {
        let pkgs = tempfile::tempdir().unwrap();
        let openssl = pkgs.path().join("openssl");
        fs::create_dir_all(&openssl).unwrap();
        fs::write(openssl.join("default.nix"), "").unwrap();
        fs::write(openssl.join("deep-override"), "").unwrap();
        let nested = openssl.join("nested");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("default.nix"), "").unwrap();
        fs::write(nested.join("deep-override"), "").unwrap();

        let paths = deep_override_attr_paths_in(pkgs.path());

        assert_eq!(paths, vec![vec!["openssl".to_string()]]);
    }

    /// A `.nix` file wins its name over a same-named directory
    /// (`dirToAttrs.nix`), so the directory is not scanned at all, even
    /// when it is validly marked as a deep override.
    #[test]
    fn nix_file_wins_the_name_over_a_marked_directory() {
        let pkgs = tempfile::tempdir().unwrap();
        fs::write(pkgs.path().join("foo.nix"), "").unwrap();
        fs::create_dir_all(pkgs.path().join("foo")).unwrap();
        fs::write(pkgs.path().join("foo").join("default.nix"), "").unwrap();
        fs::write(pkgs.path().join("foo").join("deep-override"), "").unwrap();

        let paths = deep_override_attr_paths_in(pkgs.path());

        assert_eq!(paths, Vec::<Vec<String>>::new());
    }

    /// A repository with no `pkgs/` directory at all declares no
    /// overrides; that is an empty list, not an error.
    #[test]
    fn deep_override_attr_paths_in_is_empty_without_pkgs_dir() {
        let root = tempfile::tempdir().unwrap();

        assert!(deep_override_attr_paths_in(root.path().join("pkgs")).is_empty());
    }
}
