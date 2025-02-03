#![allow(dead_code)] // TODO: Remove on first use.

use std::iter::once;

use flox_core::Version;
use thiserror::Error;

use super::typed::{
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
                merger.merge(merged, next.clone())
            })
    }
}

/// Strategy for merging two manifests which can then be applied iteratively for
/// multiple manifests.
trait ManifestMergeStrategy {
    fn merge_version(
        version1: &Version<1>,
        version2: &Version<1>,
    ) -> Result<Version<1>, MergeError>;
    fn merge_install(
        install1: &ManifestInstall,
        install2: &ManifestInstall,
    ) -> Result<ManifestInstall, MergeError>;
    fn merge_vars(
        vars1: &ManifestVariables,
        vars2: &ManifestVariables,
    ) -> Result<ManifestVariables, MergeError>;
    fn merge_hook(hook1: &ManifestHook, hook2: &ManifestHook) -> Result<ManifestHook, MergeError>;
    fn merge_profile(
        profile1: &ManifestProfile,
        profile2: &ManifestProfile,
    ) -> Result<ManifestProfile, MergeError>;
    fn merge_options(
        options1: &ManifestOptions,
        options2: &ManifestOptions,
    ) -> Result<ManifestOptions, MergeError>;
    fn merge_services(
        services1: &ManifestServices,
        services2: &ManifestServices,
    ) -> Result<ManifestServices, MergeError>;
    fn merge_build(
        build1: &ManifestBuild,
        build2: &ManifestBuild,
    ) -> Result<ManifestBuild, MergeError>;
    fn merge_containerize(
        containerize1: Option<ManifestContainerize>,
        containerize2: Option<ManifestContainerize>,
    ) -> Result<Option<ManifestContainerize>, MergeError>;
    fn merge(&self, manifest1: Manifest, manifest2: Manifest) -> Result<Manifest, MergeError>;
}

/// Merges two manifests by applying `manifest2` on top of `manifest1` and
/// overwriting any conflicts for keys within the top-level of each `Manifest`
/// field, with the exception of `profile` and `hooks`.
struct ShallowMerger;

impl ManifestMergeStrategy for ShallowMerger {
    fn merge_version(
        version1: &Version<1>,
        version2: &Version<1>,
    ) -> Result<Version<1>, MergeError> {
        if version1 != version2 {
            unreachable!("versions are hardcoded into Manifest");
        }

        Ok(version2.clone())
    }

    /// TODO: Not implemented.
    fn merge_install(
        _install1: &ManifestInstall,
        _install2: &ManifestInstall,
    ) -> Result<ManifestInstall, MergeError> {
        Ok(ManifestInstall::default())
    }

    /// Keys in `manifest2` overwrite keys in `manifest1`.
    fn merge_vars(
        vars1: &ManifestVariables,
        vars2: &ManifestVariables,
    ) -> Result<ManifestVariables, MergeError> {
        let mut merged = vars1.clone().into_inner();
        merged.extend(vars2.clone().into_inner());
        Ok(ManifestVariables(merged))
    }

    /// TODO: Not implemented.
    fn merge_hook(
        _hook1: &ManifestHook,
        _hook2: &ManifestHook,
    ) -> Result<ManifestHook, MergeError> {
        Ok(ManifestHook::default())
    }

    /// TODO: Not implemented.
    fn merge_profile(
        _profile1: &ManifestProfile,
        _profile2: &ManifestProfile,
    ) -> Result<ManifestProfile, MergeError> {
        Ok(ManifestProfile::default())
    }

    /// TODO: Not implemented.
    fn merge_options(
        _options1: &ManifestOptions,
        _options2: &ManifestOptions,
    ) -> Result<ManifestOptions, MergeError> {
        Ok(ManifestOptions::default())
    }

    /// TODO: Not implemented.
    fn merge_services(
        _services1: &ManifestServices,
        _services2: &ManifestServices,
    ) -> Result<ManifestServices, MergeError> {
        Ok(ManifestServices::default())
    }

    /// TODO: Not implemented.
    fn merge_build(
        _build1: &ManifestBuild,
        _build2: &ManifestBuild,
    ) -> Result<ManifestBuild, MergeError> {
        Ok(ManifestBuild::default())
    }

    /// TODO: Not implemented.
    fn merge_containerize(
        _containerize1: Option<ManifestContainerize>,
        _containerize2: Option<ManifestContainerize>,
    ) -> Result<Option<ManifestContainerize>, MergeError> {
        Ok(None)
    }

    fn merge(&self, manifest1: Manifest, manifest2: Manifest) -> Result<Manifest, MergeError> {
        let manifest = Manifest {
            version: Self::merge_version(&manifest1.version, &manifest2.version)?,
            install: Self::merge_install(&manifest1.install, &manifest2.install)?,
            vars: Self::merge_vars(&manifest1.vars, &manifest2.vars)?,
            hook: Self::merge_hook(&manifest1.hook, &manifest2.hook)?,
            profile: Self::merge_profile(&manifest1.profile, &manifest2.profile)?,
            options: Self::merge_options(&manifest1.options, &manifest2.options)?,
            services: Self::merge_services(&manifest1.services, &manifest2.services)?,
            build: Self::merge_build(&manifest1.build, &manifest2.build)?,
            containerize: Self::merge_containerize(manifest1.containerize, manifest2.containerize)?,
        };

        Ok(manifest)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn shallow_merger_no_deps() {
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
    fn shallow_merger_with_deps() {
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
}
