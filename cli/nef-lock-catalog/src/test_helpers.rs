//! Fixtures shared by the crate's tests.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use floxhub_client::{BuildType, LockedGitSource, LockedInputEntry};

use crate::CatalogRef;

/// A project directory whose `.flox/pkgs` holds `expressions`, given as
/// paths relative to `pkgs` and their contents.
pub(crate) fn project_with_expressions(
    expressions: &[(&str, &str)],
) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let project = tempfile::tempdir().unwrap();
    let dot_flox = project.path().join(".flox");
    let pkgs_dir = dot_flox.join("pkgs");
    for (rel_path, expression) in expressions {
        let file = pkgs_dir.join(rel_path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, expression).unwrap();
    }
    (project, dot_flox, pkgs_dir)
}

/// References built from literals, without checking them.
pub(crate) fn references(refs: &[&str]) -> BTreeSet<CatalogRef> {
    refs.iter().map(|r| CatalogRef::new_unchecked(r)).collect()
}

/// A locked git source. Locked inputs are only ever tracked as git
/// flakerefs, so the typed wire model carries exactly these fields.
pub(crate) fn git_source(url: &str, rev: &str) -> LockedGitSource {
    LockedGitSource {
        dir: ".".to_string(),
        ref_: "refs/heads/main".to_string(),
        rev: rev.to_string(),
        type_: "git".to_string(),
        url: url.to_string(),
        extra: BTreeMap::new(),
    }
}

/// A lookup entry with no inputs.
pub(crate) fn entry(
    catalog: &str,
    attr_path: &[&str],
    build_type: BuildType,
    source: LockedGitSource,
) -> LockedInputEntry {
    LockedInputEntry {
        attr_path: attr_path.iter().map(|s| s.to_string()).collect(),
        build_type,
        catalog: catalog.to_string(),
        inputs: Some(vec![]),
        locked_inputs_hash: "sha256-test".to_string(),
        deep_overrides: None,
        version: None,
        build: None,
        source,
    }
}

/// A NEF lookup entry with no inputs, sourced from a fixed git revision.
pub(crate) fn nef_entry(catalog: &str, attr_path: &[&str]) -> LockedInputEntry {
    entry(
        catalog,
        attr_path,
        BuildType::Nef,
        git_source("https://example.com/repo", "abc"),
    )
}
