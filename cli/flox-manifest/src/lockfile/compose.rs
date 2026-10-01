use std::collections::BTreeMap;

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
}
