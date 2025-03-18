#![allow(dead_code)] // TODO: Remove on first use.
                     // mod visit;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Display, Formatter};
mod shallow;
use enum_dispatch::enum_dispatch;
#[cfg(test)]
use proptest::prelude::*;
pub(crate) use shallow::ShallowMerger;
use thiserror::Error;

use super::typed::{ContainerizeConfig, Manifest};

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
        Self(iter.into_iter().map(|k| k.into()).collect())
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

/// A warning that occurred during the merge of two manifests,
/// along with the names of the two manifests involved.
#[derive(Debug, Clone, PartialEq)]
pub struct WarningWithContext {
    warning: Warning,
    higher_priority_name: String,
}

/// A collection of manifests to be merged with a `ManifestMergeTrait`.
#[derive(Debug, Clone, Default)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub(crate) struct CompositeManifest {
    pub(crate) composer: Manifest,
    #[cfg_attr(
        test,
        proptest(strategy = "proptest::collection::vec(any::<(String, Manifest)>(), 0..=2)")
    )]
    /// (name, manifest)
    /// The order is significant; later manifests have higher priority.
    pub(crate) deps: Vec<(String, Manifest)>,
}

#[derive(Clone, Debug)]
#[enum_dispatch(ManifestMergeTrait)]
pub(crate) enum ManifestMerger {
    Shallow(ShallowMerger),
}

impl CompositeManifest {
    pub(crate) fn merge_all(
        &self,
        merger: ManifestMerger,
    ) -> Result<(Manifest, Vec<WarningWithContext>), MergeError> {
        let current_manifest = &("Current manifest".to_string(), self.composer.clone());

        let mut merges = self.deps.iter().chain([current_manifest]);
        let (_, mut merged_manifest) = merges
            .next()
            .expect("including composer, there should be at least one manifest")
            .clone();

        let mut warnings = Vec::new();

        for (manifest_id, manifest) in merges {
            let (merged, merge_warnings) = merger.merge(&merged_manifest, manifest)?;
            // Update the merged manifest with the new merged manifest
            merged_manifest = merged;

            // Wrap the warnings in a `WarningWithContext` with the name of the higher priority manifest
            let merge_warnings = merge_warnings
                .into_iter()
                .map(|warnings| WarningWithContext {
                    warning: warnings,
                    higher_priority_name: manifest_id.clone(),
                });

            warnings.extend(merge_warnings);
        }

        Ok((merged_manifest, warnings))
    }
}

/// Strategy for merging two manifests which can then be applied iteratively for
/// multiple manifests.
#[enum_dispatch]
trait ManifestMergeTrait {
    fn merge(
        &self,
        low_priority: &Manifest,
        high_priority: &Manifest,
    ) -> Result<(Manifest, Vec<Warning>), MergeError>;
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
    base_key: KeyPath,
    low_priority: Option<&BTreeMap<String, T>>,
    high_priority: Option<&BTreeMap<String, T>>,
) -> (Option<BTreeMap<String, T>>, Vec<Warning>) {
    match (low_priority, high_priority) {
        (None, None) => (None, Default::default()),
        (Some(map1), None) => (Some(map1.clone()), Default::default()),
        (None, Some(map2)) => (Some(map2.clone()), Default::default()),
        (Some(map1), Some(map2)) => {
            let (merged, warnings) = map_union(base_key, map1, map2);
            (Some(merged), warnings)
        },
    }
}

/// Takes the union of the key-value pairs from the two maps, with key-value pairs from the high
/// priority map taking precedence.
fn map_union<K, V>(
    base_key: KeyPath,
    low_priority: &BTreeMap<K, V>,
    high_priority: &BTreeMap<K, V>,
) -> (BTreeMap<K, V>, Vec<Warning>)
where
    K: Clone + Ord,
    for<'a> &'a K: Into<String>,
    V: Clone,
{
    let low_priority_keys: BTreeSet<_> = low_priority.keys().collect();
    let high_priority_keys: BTreeSet<_> = high_priority.keys().collect();
    let warnings = low_priority_keys
        .intersection(&high_priority_keys)
        .map(|key| Warning::Overriding(base_key.push(*key)))
        .collect();

    let mut merged = low_priority.clone();
    merged.extend(high_priority.clone());
    (merged, warnings)
}

/// Takes the high priority `T` if it's present, otherwise the low priority `T`.
#[must_use]
fn shallow_merge_options<M, T: Into<M>>(
    key: KeyPath,
    low_priority: Option<T>,
    high_priority: Option<T>,
) -> (Option<M>, Option<Warning>) {
    match (low_priority, high_priority) {
        (None, None) => (None, None),
        (Some(lp), None) => (Some(lp.into()), None),
        (None, Some(hp)) => (Some(hp.into()), None),
        (Some(_), Some(hp)) => (Some(hp.into()), Some(Warning::Overriding(key))),
    }
}

fn deep_merge_optional_containerize_config(
    low_priority: Option<&ContainerizeConfig>,
    high_priority: Option<&ContainerizeConfig>,
) -> (Option<ContainerizeConfig>, Vec<Warning>) {
    let mut warnings = Vec::new();

    match (low_priority, high_priority) {
        (None, None) => (None, warnings),
        (Some(cfg), None) => (Some(cfg.clone()), warnings),
        (None, Some(cfg)) => (Some(cfg.clone()), warnings),
        (Some(cfg_lp), Some(cfg_hp)) => {
            let root_key = KeyPath::from_iter(["containerize", "config"]);
            let (user, user_warning) = shallow_merge_options(
                root_key.push("user"),
                cfg_lp.user.as_ref(),
                cfg_hp.user.as_ref(),
            );
            warnings.extend(user_warning);

            let (cmd, cmd_warning) = shallow_merge_options(
                root_key.push("cmd"),
                cfg_lp.cmd.as_deref(),
                cfg_hp.cmd.as_deref(),
            );
            warnings.extend(cmd_warning);

            let (working_dir, working_dir_warning) = shallow_merge_options(
                root_key.push("working-dir"),
                cfg_lp.working_dir.as_ref(),
                cfg_hp.working_dir.as_ref(),
            );
            warnings.extend(working_dir_warning);

            let (labels, labels_warnings) = optional_map_union(
                root_key.push("labels"),
                cfg_lp.labels.as_ref(),
                cfg_hp.labels.as_ref(),
            );
            warnings.extend(labels_warnings);

            let (stop_signal, stop_signal_warning) = shallow_merge_options(
                root_key.push("stop-signal"),
                cfg_lp.stop_signal.as_ref(),
                cfg_hp.stop_signal.as_ref(),
            );
            warnings.extend(stop_signal_warning);

            let cfg = ContainerizeConfig {
                user,
                exposed_ports: optional_set_union(
                    cfg_lp.exposed_ports.as_ref(),
                    cfg_hp.exposed_ports.as_ref(),
                ),
                cmd,
                volumes: optional_set_union(cfg_lp.volumes.as_ref(), cfg_hp.volumes.as_ref()),
                working_dir,
                labels,
                stop_signal,
            };

            (Some(cfg), warnings)
        },
    }
}

#[cfg(test)]
mod tests {
    use super::shallow::ShallowMerger;
    use super::*;
    use crate::models::manifest::typed::{Inner, Profile, Vars};

    #[test]
    fn composite_manifest_runs_merger() {
        let composer = Manifest {
            profile: Some(Profile {
                common: Some("composer".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let manifest1 = {
            let mut manifest = Manifest::default();
            manifest
                .vars
                .inner_mut()
                .insert("var1".to_string(), "manifest1".to_string());
            manifest
        };
        let manifest2 = Manifest {
            vars: Vars(BTreeMap::from([(
                "var2".to_string(),
                "manifest2".to_string(),
            )])),
            profile: Some(Profile {
                common: Some("manifest2".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let composite = CompositeManifest {
            composer,
            deps: vec![
                ("dep1".to_string(), manifest1),
                ("dep2".to_string(), manifest2),
            ],
        };
        let (merged, _warnings) = composite
            .merge_all(ManifestMerger::Shallow(ShallowMerger))
            .unwrap();
        assert_eq!(merged.vars.inner()["var1"], "manifest1");
        assert_eq!(merged.vars.inner()["var2"], "manifest2");
        assert_eq!(
            merged.profile,
            Some(Profile {
                common: Some("manifest2\ncomposer".to_string()),
                ..Default::default()
            })
        );
    }
}
