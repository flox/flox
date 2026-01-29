use std::collections::HashMap;

#[cfg(any(test, feature = "tests"))]
use flox_test_utils::proptest::alphanum_string;
#[cfg(any(test, feature = "tests"))]
use proptest::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::compose::WarningWithContext;
use crate::parsed::common::IncludeDescriptor;
use crate::{Manifest, ManifestError};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub struct Compose {
    /// The composing environment's manifest that was on disk at lock-time.
    pub composer: Manifest,
    /// Metadata and manifests for the included environments in the order
    /// that they were specified in the composing environment's manifest.
    pub include: Vec<LockedInclude>,
    /// Warnings generated during composition + locking.
    pub warnings: Vec<WarningWithContext>,
}

impl Compose {
    /// Get the highest priority included environment which provides each package.
    /// Packages that are not provided by any included environments will be absent from the map.
    pub fn get_includes_for_packages(
        &self,
        packages: &[String],
    ) -> Result<HashMap<String, LockedInclude>, ManifestError> {
        let mut result = HashMap::new();
        for package in packages {
            if let Some(include) = Self::get_include_for_package(package, &self.include)? {
                result.insert(package.clone(), include);
            }
        }

        Ok(result)
    }

    /// Detect which included environment, if any, provides a given package.
    fn get_include_for_package(
        package: &String,
        includes: &[LockedInclude],
    ) -> Result<Option<LockedInclude>, ManifestError> {
        // Reverse of merge order so that we return the highest priority match.
        for include in includes.iter().rev() {
            match include.manifest.get_install_ids(vec![package.to_string()]) {
                Ok(_) => return Ok(Some(include.clone())),
                Err(ManifestError::PackageNotFound(_)) => continue,
                Err(ManifestError::MultiplePackagesMatch(_, _)) => continue,
                Err(err) => return Err(err),
            }
        }

        Ok(None)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub struct LockedInclude {
    pub manifest: Manifest,
    #[cfg_attr(test, proptest(strategy = "alphanum_string(5)"))]
    pub name: String,
    pub descriptor: IncludeDescriptor,
    // TODO: Record generation if/when:
    // 1. We have a need for it in presentation, e.g.
    //   - https://github.com/flox/flox/issues/2948
    // 2. Generations work has settled:
    //   - https://github.com/flox/product/pull/881
    //   - https://github.com/flox/product/pull/891
    // 3. We've exposed it from `RemoteEnvironment`/`ManagedEnvironment`
    // pub remote: Option<Generation>,
}
