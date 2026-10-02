use std::collections::BTreeMap;

use flox_core::data::environment_ref::RemoteEnvironmentRef;
#[cfg(any(test, feature = "tests"))]
use flox_test_utils::proptest::alphanum_string;
#[cfg(any(test, feature = "tests"))]
use proptest::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::compose::WarningWithContext;
use crate::interfaces::{AsLatestSchema, PackageLookup};
use crate::parsed::Inner;
use crate::parsed::latest::IncludeDescriptor;
use crate::{Manifest, ManifestError, TypedOnly};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
pub struct Compose {
    /// The composing environment's manifest that was on disk at lock-time.
    pub composer: Manifest<TypedOnly>,
    /// Metadata and manifests for the included environments in the order
    /// that they were specified in the composing environment's manifest.
    pub include: Vec<LockedInclude>,
    /// Warnings generated during composition + locking.
    pub warnings: Vec<WarningWithContext>,
}

impl Compose {
    /// Detect which included environment, if any, provides a given package.
    pub fn get_include_for_package(
        &self,
        package: &str,
        version: &Option<String>,
    ) -> Result<Option<LockedInclude>, ManifestError> {
        // Reverse of merge order so that we return the highest priority match.
        for include in self.include.iter().rev() {
            let res = match &include.manifest.inner.parsed {
                crate::Parsed::V1(manifest) => manifest.resolve_install_id(package, version),
                crate::Parsed::V1_10_0(manifest) => manifest.resolve_install_id(package, version),
                crate::Parsed::V1_11_0(manifest) => manifest.resolve_install_id(package, version),
                crate::Parsed::V1_12_0(manifest) => manifest.resolve_install_id(package, version),
                crate::Parsed::V1_13_0(manifest) => manifest.resolve_install_id(package, version),
                crate::Parsed::V1_14_0(manifest) => manifest.resolve_install_id(package, version),
                crate::Parsed::V1_15_0(manifest) => manifest.resolve_install_id(package, version),
                crate::Parsed::V1_16_0(manifest) => manifest.resolve_install_id(package, version),
                crate::Parsed::V1_17_0(manifest) => manifest.resolve_install_id(package, version),
                crate::Parsed::V1_18_0(manifest) => manifest.resolve_install_id(package, version),
            };
            match res {
                Ok(_) => return Ok(Some(include.clone())),
                Err(ManifestError::PackageNotFound(_)) => continue,
                Err(ManifestError::MultiplePackagesMatch(_, _)) => continue,
                Err(err) => return Err(err),
            }
        }

        Ok(None)
    }

    /// Maps each install ID an included environment provides to the name of
    /// the include that provides it, using the merge's precedence:
    /// later includes take precedence over earlier ones,
    /// and install IDs the composer declares are omitted.
    pub fn include_names_by_install_id(&self) -> Result<BTreeMap<String, String>, ManifestError> {
        let mut names = BTreeMap::new();
        for include in &self.include {
            let manifest = include.manifest.migrate_typed_only(None)?;
            for install_id in manifest.as_latest_schema().install.inner().keys() {
                names.insert(install_id.clone(), include.name.clone());
            }
        }

        let composer = self.composer.migrate_typed_only(None)?;
        for install_id in composer.as_latest_schema().install.inner().keys() {
            names.remove(install_id);
        }

        Ok(names)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
pub struct LockedInclude {
    pub manifest: Manifest<TypedOnly>,
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "alphanum_string(5)")
    )]
    pub name: String,
    pub descriptor: IncludeDescriptor,
    /// The generation of an environment included from FloxHub that was
    /// fetched, so that users can see which one is in use.
    ///
    /// Older versions of Flox didn't record it, and it's not set for other
    /// included environments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<usize>,
    /// A hash of the packages that an included directory's environment had
    /// locked when the composing environment last took them.
    ///
    /// It tells upgrades that the included environment locked from the
    /// composing environment's own.
    /// Older versions of Flox didn't record it, and it's not set for
    /// environments included from FloxHub, whose generation records it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub packages_hash: Option<String>,
    /// The environments from FloxHub that an included directory's environment
    /// included, directly or in turn, when the composing environment last
    /// took it, which are merged into `manifest`, for checking their trust.
    ///
    /// It's recorded along with `packages_hash`, so if that isn't recorded,
    /// neither is this.
    /// Comparing included environments ignores it: what they include changes
    /// their manifests too.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub included_remotes: Vec<IncludedRemote>,
}

/// An environment from FloxHub that an included environment includes
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
pub struct IncludedRemote {
    pub remote: RemoteEnvironmentRef,
    /// The generation it was fetched at, if recorded
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "proptest::option::of(0..10usize)")
    )]
    pub generation: Option<usize>,
}

impl LockedInclude {
    /// This version of the included environment, keeping only the fields
    /// that `recorded` records.
    ///
    /// Lockfiles written by older versions of Flox don't record every field,
    /// so comparing with them ignores the fields they lack.
    pub fn as_recorded_by(&self, recorded: &LockedInclude) -> LockedInclude {
        LockedInclude {
            generation: recorded.generation.and(self.generation),
            packages_hash: recorded
                .packages_hash
                .as_ref()
                .and(self.packages_hash.clone()),
            included_remotes: recorded.included_remotes.clone(),
            ..self.clone()
        }
    }

    /// Whether this is the version of the included environment that
    /// `recorded` records, see [Self::as_recorded_by]
    pub fn is_recorded_by(&self, recorded: &LockedInclude) -> bool {
        self.as_recorded_by(recorded) == *recorded
    }

    /// Whether the included environment still has the lock that `recorded`
    /// records, i.e. it hasn't changed its packages since,
    /// or [None] if `recorded` doesn't record its lock
    pub fn has_lock_recorded_by(&self, recorded: &LockedInclude) -> Option<bool> {
        if !recorded
            .descriptor
            .includes_same_environment(&self.descriptor)
        {
            return None;
        }
        match &recorded.descriptor {
            IncludeDescriptor::Remote { .. } => recorded
                .generation
                .map(|generation| self.generation == Some(generation)),
            IncludeDescriptor::Local { .. } => recorded
                .packages_hash
                .as_ref()
                .map(|hash| self.packages_hash.as_ref() == Some(hash)),
        }
    }

    /// This included environment without what's recorded about how it was
    /// locked: its generation, `packages_hash` and `included_remotes`
    pub fn without_records(&self) -> LockedInclude {
        LockedInclude {
            generation: None,
            packages_hash: None,
            included_remotes: Vec::new(),
            ..self.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use indoc::indoc;
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::interfaces::AsTypedOnlyManifest;
    use crate::test_helpers::with_latest_schema;

    fn typed_manifest(body: &str) -> Manifest<TypedOnly> {
        Manifest::parse_toml_typed(with_latest_schema(body))
            .unwrap()
            .as_typed_only()
    }

    fn locked_include(name: &str, body: &str) -> LockedInclude {
        LockedInclude {
            manifest: typed_manifest(body),
            name: name.to_string(),
            descriptor: IncludeDescriptor::Local {
                dir: name.into(),
                name: None,
                auto_upgrade: None,
            },
            generation: None,
            packages_hash: None,
            included_remotes: Vec::new(),
        }
    }

    #[test]
    fn include_names_by_install_id_prefers_composer_and_later_includes() {
        let compose = Compose {
            composer: typed_manifest(indoc! {r#"
                [install]
                a.pkg-path = "a"
            "#}),
            include: vec![
                locked_include("include1", indoc! {r#"
                    [install]
                    a.pkg-path = "a"
                    b.pkg-path = "b"
                    c.pkg-path = "c"
                "#}),
                locked_include("include2", indoc! {r#"
                    [install]
                    c.pkg-path = "c"
                "#}),
            ],
            warnings: vec![],
        };

        assert_eq!(
            compose.include_names_by_install_id().unwrap(),
            BTreeMap::from([
                ("b".to_string(), "include1".to_string()),
                ("c".to_string(), "include2".to_string()),
            ])
        );
    }

    /// Another environment from FloxHub with the same name and generation
    /// number doesn't have the recorded lock
    #[test]
    fn has_lock_recorded_by_only_for_the_same_environment() {
        let remote = |reference: &str| LockedInclude {
            descriptor: IncludeDescriptor::Remote {
                remote: reference.parse().unwrap(),
                name: None,
                generation: None,
                auto_upgrade: None,
            },
            generation: Some(2),
            ..locked_include("python", "")
        };
        let recorded = remote("alice/python");

        assert_eq!(
            remote("alice/python").has_lock_recorded_by(&recorded),
            Some(true)
        );
        assert_eq!(remote("bob/python").has_lock_recorded_by(&recorded), None);
    }
}
