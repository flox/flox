#![allow(dead_code)] // TODO: Remove on first use.

use std::iter::once;

use flox_core::Version;
use thiserror::Error;

use super::typed::{
    Inner,
    Manifest,
    ManifestBuild,
    ManifestContainerize,
    ManifestHook,
    ManifestInstall,
    ManifestOptions,
    ManifestProfile,
    ManifestServices,
    ManifestVariables,
};

#[derive(Error, Debug)]
pub enum MergeError {}

/// A collection of manifests to be merged with a `ManifestMergeStrategy`.
struct CompositeManifest {
    composer: Manifest,
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
        low_priority: &ManifestInstall,
        high_priority: &ManifestInstall,
    ) -> Result<ManifestInstall, MergeError>;
    fn merge_vars(
        low_priority: &ManifestVariables,
        high_priority: &ManifestVariables,
    ) -> Result<ManifestVariables, MergeError>;
    fn merge_hook(
        low_priority: &ManifestHook,
        high_priority: &ManifestHook,
    ) -> Result<ManifestHook, MergeError>;
    fn merge_profile(
        low_priority: &ManifestProfile,
        high_priority: &ManifestProfile,
    ) -> Result<ManifestProfile, MergeError>;
    fn merge_options(
        low_priority: &ManifestOptions,
        high_priority: &ManifestOptions,
    ) -> Result<ManifestOptions, MergeError>;
    fn merge_services(
        low_priority: &ManifestServices,
        high_priority: &ManifestServices,
    ) -> Result<ManifestServices, MergeError>;
    fn merge_build(
        low_priority: &ManifestBuild,
        high_priority: &ManifestBuild,
    ) -> Result<ManifestBuild, MergeError>;
    fn merge_containerize(
        low_priority: Option<&ManifestContainerize>,
        high_priority: Option<&ManifestContainerize>,
    ) -> Result<Option<ManifestContainerize>, MergeError>;
    fn merge(
        &self,
        low_priority: &Manifest,
        high_priority: &Manifest,
    ) -> Result<Manifest, MergeError>;
}

/// Merges two manifests by applying `manifest2` on top of `manifest1` and
/// overwriting any conflicts for keys within the top-level of each `Manifest`
/// field, with the exception of `profile` and `hooks`.
struct ShallowMerger;

impl ManifestMergeStrategy for ShallowMerger {
    fn merge_version(
        low_priority: &Version<1>,
        high_priority: &Version<1>,
    ) -> Result<Version<1>, MergeError> {
        if low_priority != high_priority {
            unreachable!("versions are hardcoded into Manifest");
        }

        Ok(high_priority.clone())
    }

    fn merge_install(
        low_priority: &ManifestInstall,
        high_priority: &ManifestInstall,
    ) -> Result<ManifestInstall, MergeError> {
        let mut merged = low_priority.inner().clone();
        merged.extend(high_priority.inner().clone());
        Ok(ManifestInstall(merged))
    }

    /// Keys in `manifest2` overwrite keys in `manifest1`.
    fn merge_vars(
        low_priority: &ManifestVariables,
        high_priority: &ManifestVariables,
    ) -> Result<ManifestVariables, MergeError> {
        let mut merged = low_priority.clone().into_inner();
        merged.extend(high_priority.clone().into_inner());
        Ok(ManifestVariables(merged))
    }

    fn merge_hook(
        low_priority: &ManifestHook,
        high_priority: &ManifestHook,
    ) -> Result<ManifestHook, MergeError> {
        Ok(ManifestHook {
            on_activate: append_optional_strings(
                low_priority.on_activate.as_ref(),
                high_priority.on_activate.as_ref(),
            ),
        })
    }

    fn merge_profile(
        low_priority: &ManifestProfile,
        high_priority: &ManifestProfile,
    ) -> Result<ManifestProfile, MergeError> {
        let common =
            append_optional_strings(low_priority.common.as_ref(), high_priority.common.as_ref());
        let bash = append_optional_strings(low_priority.bash.as_ref(), high_priority.bash.as_ref());
        let zsh = append_optional_strings(low_priority.zsh.as_ref(), high_priority.zsh.as_ref());
        let tcsh = append_optional_strings(low_priority.tcsh.as_ref(), high_priority.tcsh.as_ref());
        let fish = append_optional_strings(low_priority.fish.as_ref(), high_priority.fish.as_ref());
        let merged = ManifestProfile {
            common,
            bash,
            zsh,
            fish,
            tcsh,
        };
        Ok(merged)
    }

    /// TODO: Not implemented.
    fn merge_options(
        low_priority: &ManifestOptions,
        high_priority: &ManifestOptions,
    ) -> Result<ManifestOptions, MergeError> {
        let mut merged = low_priority.clone();
        merged.allow.unfree = high_priority.allow.unfree;
        merged.allow.broken = high_priority.allow.broken;
        merged.allow.licenses = high_priority.allow.licenses.clone();
        merged.semver.allow_pre_releases = high_priority.semver.allow_pre_releases;
        merged.cuda_detection = high_priority.cuda_detection;
        merged.systems = high_priority.systems.clone();
        Ok(merged)
    }

    /// TODO: Not implemented.
    fn merge_services(
        low_priority: &ManifestServices,
        high_priority: &ManifestServices,
    ) -> Result<ManifestServices, MergeError> {
        let mut merged = low_priority.inner().clone();
        merged.extend(high_priority.inner().clone());
        Ok(ManifestServices(merged))
    }

    /// TODO: Not implemented.
    fn merge_build(
        low_priority: &ManifestBuild,
        high_priority: &ManifestBuild,
    ) -> Result<ManifestBuild, MergeError> {
        let mut merged = low_priority.inner().clone();
        merged.extend(high_priority.inner().clone());
        Ok(ManifestBuild(merged))
    }

    /// TODO: Not implemented.
    fn merge_containerize(
        low_priority: Option<&ManifestContainerize>,
        high_priority: Option<&ManifestContainerize>,
    ) -> Result<Option<ManifestContainerize>, MergeError> {
        let merged_containerize = if let Some(lp) = low_priority {
            if let Some(hp) = high_priority {
                let mut merged = lp.config.clone();
                if let Some(ref config) = hp.config {
                    merged = merged.map(|mut c| {
                        c.user = config.user.clone();
                        c.exposed_ports = config.exposed_ports.clone();
                        c.cmd = config.cmd.clone();
                        c.volumes = config.volumes.clone();
                        c.working_dir = config.working_dir.clone();
                        c.labels = config.labels.clone();
                        c.stop_signal = config.stop_signal.clone();
                        c
                    });
                }
                Some(ManifestContainerize { config: merged })
            } else {
                low_priority.cloned()
            }
        } else {
            high_priority.cloned()
        };
        Ok(merged_containerize)
    }

    fn merge(
        &self,
        low_priority: &Manifest,
        high_priority: &Manifest,
    ) -> Result<Manifest, MergeError> {
        let manifest = Manifest {
            version: Self::merge_version(&low_priority.version, &high_priority.version)?,
            install: Self::merge_install(&low_priority.install, &high_priority.install)?,
            vars: Self::merge_vars(&low_priority.vars, &high_priority.vars)?,
            hook: Self::merge_hook(&low_priority.hook, &high_priority.hook)?,
            profile: Self::merge_profile(&low_priority.profile, &high_priority.profile)?,
            options: Self::merge_options(&low_priority.options, &high_priority.options)?,
            services: Self::merge_services(&low_priority.services, &high_priority.services)?,
            build: Self::merge_build(&low_priority.build, &high_priority.build)?,
            containerize: Self::merge_containerize(
                low_priority.containerize.as_ref(),
                high_priority.containerize.as_ref(),
            )?,
        };

        Ok(manifest)
    }
}

/// Given two optional strings, append them if they're present, return the present one or `None` if not.
fn append_optional_strings(first: Option<&String>, second: Option<&String>) -> Option<String> {
    if let Some(first) = first {
        if let Some(second) = second {
            Some(format!("{first}\n{second}"))
        } else {
            Some(first.clone())
        }
    } else {
        second.cloned()
    }
}

#[cfg(test)]
mod tests {
    mod shallow {

        use std::collections::BTreeMap;

        use pretty_assertions::assert_eq;
        use proptest::prelude::*;

        use super::super::*;

        #[test]
        fn vars_no_deps() {
            let composer = Manifest {
                version: Version::<1>,
                vars: ManifestVariables(BTreeMap::from([
                    ("composer_a".to_string(), "set by composer".to_string()),
                    ("composer_b".to_string(), "set by composer".to_string()),
                ])),
                ..Manifest::default()
            };

            let composite_manifest = CompositeManifest {
                composer: composer.clone(),
                deps: vec![],
            };

            let merged = composite_manifest.merge_all(ShallowMerger).unwrap();
            assert_eq!(merged, composer);
        }

        #[test]
        fn vars_with_deps() {
            let dep1 = Manifest {
                version: Version::<1>,
                vars: ManifestVariables(BTreeMap::from([
                    ("dep1_a".to_string(), "set by dep1".to_string()),
                    ("dep1_b".to_string(), "set by dep1".to_string()),
                    ("dep1_c".to_string(), "set by dep1".to_string()),
                ])),
                ..Manifest::default()
            };

            let dep2 = Manifest {
                version: Version::<1>,
                vars: ManifestVariables(BTreeMap::from([
                    ("dep1_a".to_string(), "updated by dep2".to_string()),
                    ("dep1_b".to_string(), "updated by dep2".to_string()),
                    ("dep2_a".to_string(), "set by dep2".to_string()),
                    ("dep2_b".to_string(), "set by dep2".to_string()),
                ])),
                ..Manifest::default()
            };

            let composer = Manifest {
                version: Version::<1>,
                vars: ManifestVariables(BTreeMap::from([
                    ("dep1_a".to_string(), "updated by composer".to_string()),
                    ("dep2_a".to_string(), "updated by composer".to_string()),
                    ("composer_a".to_string(), "set by composer".to_string()),
                ])),
                ..Manifest::default()
            };

            let composite_manifest = CompositeManifest {
                composer,
                deps: vec![dep1, dep2],
            };

            let merged = composite_manifest.merge_all(ShallowMerger).unwrap();

            assert_eq!(merged, Manifest {
                version: Version::<1>,
                vars: ManifestVariables(BTreeMap::from([
                    ("dep1_a".to_string(), "updated by composer".to_string()),
                    ("dep1_b".to_string(), "updated by dep2".to_string()),
                    ("dep1_c".to_string(), "set by dep1".to_string()),
                    ("dep2_a".to_string(), "updated by composer".to_string()),
                    ("dep2_b".to_string(), "set by dep2".to_string()),
                    ("composer_a".to_string(), "set by composer".to_string()),
                ])),
                // TODO: Not implemented.
                install: ManifestInstall::default(),
                hook: ManifestHook::default(),
                profile: ManifestProfile::default(),
                options: ManifestOptions::default(),
                services: ManifestServices::default(),
                build: ManifestBuild::default(),
                containerize: None,
            })
        }

        proptest! {
            #[test]
            fn install_section(lp: ManifestInstall, hp: ManifestInstall) {
                // todo
            }
        }
    }
}
