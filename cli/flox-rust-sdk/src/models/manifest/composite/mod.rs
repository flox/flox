#![allow(dead_code)] // TODO: Remove on first use.
                     // mod visit;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Display, Formatter};
use std::iter::once;
mod shallow;
use flox_core::Version;
#[cfg(test)]
use proptest::prelude::*;
use thiserror::Error;

use super::typed::{
    Build,
    Containerize,
    ContainerizeConfig,
    Hook,
    Install,
    Manifest,
    Options,
    Profile,
    Services,
    Vars,
};

#[derive(Error, Debug)]
pub enum MergeError {}

/// A key path to a value in a manifest.
/// This is used to provide the location for warnings.
///
/// The `KeyPath` behaves like an immutable stack of keys,
/// where [`KeyPath::push`] and [`KeyPath::extend`] return a new `KeyPath`
/// with the new key(s) added to the top of the stack,
/// leaving the original `KeyPath` unchanged.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct KeyPath(Vec<String>);
impl KeyPath {
    /// Create a new empty `KeyPath`.
    pub const fn new() -> Self {
        Self(Vec::new())
    }

    /// Create a new `KeyPath` from `self`
    /// with the given key pushed onto the top of the stack.
    pub fn push(&self, key: impl Into<String>) -> Self {
        self.extend([key.into()])
    }

    /// Create a new `KeyPath` from `self` with the given keys pushed onto the top of the stack.
    fn extend(&self, iter: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let mut new_path = self.0.clone();
        new_path.extend(iter.into_iter().map(|k| k.into()));
        Self(new_path)
    }
}

impl Display for KeyPath {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "{}", self.0.join("."))
    }
}

impl<Key: Into<String>> FromIterator<Key> for KeyPath {
    fn from_iter<T: IntoIterator<Item = Key>>(iter: T) -> Self {
        iter.into_iter().map(|k| k.into()).collect()
    }
}

/// A warning that occurred during the merge of two manifests.
/// This is used to provide feedback to the user about potential issues.
///
/// Warnings are not errors, but they may indicate
/// that the user should review the merged manifest or its dependencies.
///
/// Currently, the only warning is that a value is being overridden,
/// but more warnings may be added in the future.
#[derive(Debug, Clone, PartialEq)]
#[must_use]
pub enum Warning {
    Overriding(KeyPath),
}

/// A collection of manifests to be merged with a `ManifestMergeStrategy`.
#[derive(Debug, Clone, Default)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
struct CompositeManifest {
    composer: Manifest,
    #[cfg_attr(
        test,
        proptest(strategy = "proptest::collection::vec(any::<Manifest>(), 0..=2)")
    )]
    deps: Vec<Manifest>,
}

impl CompositeManifest {
    fn merge_all(&self, merger: impl ManifestMergeStrategy) -> Result<Manifest, MergeError> {
        let Some(first_dep) = self.deps.first() else {
            // No deps, just composer.
            return Ok(self.composer.clone());
        };

        self.deps
            .iter()
            .skip(1) // First dep is used as initializer.
            .chain(once(&self.composer)) // Composer goes last.
            .try_fold(first_dep.clone(), |merged, next| {
                merger.merge(&merged, next)
            })
    }
}

/// Strategy for merging two manifests which can then be applied iteratively for
/// multiple manifests.
trait ManifestMergeStrategy {
    fn merge_version(
        low_priority: &Version<1>,
        high_priority: &Version<1>,
    ) -> Result<Version<1>, MergeError>;
    fn merge_install(
        low_priority: &Install,
        high_priority: &Install,
    ) -> Result<Install, MergeError>;
    fn merge_vars(low_priority: &Vars, high_priority: &Vars) -> Result<Vars, MergeError>;
    fn merge_hook(low_priority: &Hook, high_priority: &Hook) -> Result<Hook, MergeError>;
    fn merge_profile(
        low_priority: &Profile,
        high_priority: &Profile,
    ) -> Result<Profile, MergeError>;
    fn merge_options(
        low_priority: &Options,
        high_priority: &Options,
    ) -> Result<Options, MergeError>;
    fn merge_services(
        low_priority: &Services,
        high_priority: &Services,
    ) -> Result<Services, MergeError>;
    fn merge_build(low_priority: &Build, high_priority: &Build) -> Result<Build, MergeError>;
    fn merge_containerize(
        low_priority: Option<&Containerize>,
        high_priority: Option<&Containerize>,
    ) -> Result<Option<Containerize>, MergeError>;
    fn merge(
        &self,
        low_priority: &Manifest,
        high_priority: &Manifest,
    ) -> Result<Manifest, MergeError>;
}

/// Takes the higher priority string if it's present, or the lower priority string.
fn shallow_merge_optional_strings(
    low_priority: Option<&String>,
    high_priority: Option<&String>,
) -> Option<String> {
    high_priority.cloned().or(low_priority.cloned())
}

/// Given two optional strings, append them if they're present, return the present one or `None` if not.
fn append_optional_strings(first: Option<&String>, second: Option<&String>) -> Option<String> {
    match (first, second) {
        (Some(s1), Some(s2)) => Some(format!("{s1}\n{s2}")),
        (Some(s1), None) => Some(s1.clone()),
        (None, Some(s2)) => Some(s2.clone()),
        (None, None) => None,
    }
}

/// Takes the union of the two sets, with keys from the high priority set taking precedence.
fn optional_set_union<T: Clone + Ord>(
    low_priority: Option<&BTreeSet<T>>,
    high_priority: Option<&BTreeSet<T>>,
) -> Option<BTreeSet<T>> {
    match (low_priority, high_priority) {
        (Some(set1), Some(set2)) => {
            let mut set1 = (*set1).clone();
            for key in set2.iter() {
                set1.insert(key.clone());
            }
            Some(set1)
        },
        (Some(_set1), None) => low_priority.cloned(),
        (None, Some(_set2)) => high_priority.cloned(),
        (None, None) => None,
    }
}

/// Takes the union of the key-value pairs from the two maps, with key-value pairs from the high
/// priority map taking precedence.
fn optional_map_union<T: Clone + Ord>(
    low_priority: Option<&BTreeMap<String, T>>,
    high_priority: Option<&BTreeMap<String, T>>,
) -> Option<BTreeMap<String, T>> {
    match (low_priority, high_priority) {
        (None, None) => None,
        (Some(map1), None) => Some(map1.clone()),
        (None, Some(map2)) => Some(map2.clone()),
        (Some(map1), Some(map2)) => {
            let merged = map_union(map1, map2);
            Some(merged)
        },
    }
}

/// Takes the union of the key-value pairs from the two maps, with key-value pairs from the high
/// priority map taking precedence.
fn map_union<K: Clone + Ord, V: Clone>(
    low_priority: &BTreeMap<K, V>,
    high_priority: &BTreeMap<K, V>,
) -> BTreeMap<K, V> {
    let mut merged = low_priority.clone();
    merged.extend(high_priority.clone());
    merged
}

/// Takes the entire contents of the high priority vector if it's present, otherwise the entire
/// contents of the low priority vector.
fn shallow_merge_optional_vecs<T: Clone>(
    low_priority: Option<&Vec<T>>,
    high_priority: Option<&Vec<T>>,
) -> Option<Vec<T>> {
    high_priority.cloned().or(low_priority.cloned())
}

/// Takes the high priority `T` if it's present, otherwise the low priority `T`.
fn shallow_merge_options<T: Clone>(
    low_priority: Option<&T>,
    high_priority: Option<&T>,
) -> Option<T> {
    high_priority.cloned().or(low_priority.cloned())
}

fn deep_merge_optional_containerize_config(
    low_priority: Option<&ContainerizeConfig>,
    high_priority: Option<&ContainerizeConfig>,
) -> Option<ContainerizeConfig> {
    match (low_priority, high_priority) {
        (None, None) => None,
        (Some(cfg), None) => Some(cfg.clone()),
        (None, Some(cfg)) => Some(cfg.clone()),
        (Some(cfg_lp), Some(cfg_hp)) => {
            let cfg = ContainerizeConfig {
                user: shallow_merge_options(cfg_lp.user.as_ref(), cfg_hp.user.as_ref()),
                exposed_ports: optional_set_union(
                    cfg_lp.exposed_ports.as_ref(),
                    cfg_hp.exposed_ports.as_ref(),
                ),
                cmd: shallow_merge_options(cfg_lp.cmd.as_ref(), cfg_hp.cmd.as_ref()),
                volumes: optional_set_union(cfg_lp.volumes.as_ref(), cfg_hp.volumes.as_ref()),
                working_dir: shallow_merge_options(
                    cfg_lp.working_dir.as_ref(),
                    cfg_hp.working_dir.as_ref(),
                ),
                labels: optional_map_union(cfg_lp.labels.as_ref(), cfg_hp.labels.as_ref()),
                stop_signal: shallow_merge_options(
                    cfg_lp.stop_signal.as_ref(),
                    cfg_hp.stop_signal.as_ref(),
                ),
            };
            Some(cfg)
        },
    }
}

#[cfg(test)]
mod tests {
    use super::shallow::ShallowMerger;
    use super::*;
    use crate::models::manifest::typed::Inner;

    #[test]
    fn composite_manifest_runs_merger() {
        let composer = {
            let mut manifest = Manifest::default();
            manifest.profile.common = Some("composer".to_string());
            manifest
        };
        let manifest1 = {
            let mut manifest = Manifest::default();
            manifest
                .vars
                .inner_mut()
                .insert("var1".to_string(), "manifest1".to_string());
            manifest
        };
        let manifest2 = {
            let mut manifest = Manifest::default();
            manifest
                .vars
                .inner_mut()
                .insert("var2".to_string(), "manifest2".to_string());
            manifest.profile.common = Some("manifest2".to_string());
            manifest
        };
        let composite = CompositeManifest {
            composer,
            deps: vec![manifest1, manifest2],
        };
        let merged = composite.merge_all(ShallowMerger).unwrap();
        assert_eq!(merged.vars.inner()["var1"], "manifest1");
        assert_eq!(merged.vars.inner()["var2"], "manifest2");
        assert_eq!(
            merged.profile.common,
            Some("manifest2\ncomposer".to_string())
        );
    }
}
