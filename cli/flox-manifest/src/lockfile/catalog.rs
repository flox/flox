use std::collections::BTreeMap;

use catalog_api_v1::types as catalog_types;
use flox_core::data::System;
#[cfg(any(test, feature = "tests"))]
use flox_test_utils::proptest::chrono_strat;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use crate::parsed::common::{DEFAULT_GROUP_NAME, DEFAULT_PRIORITY};
use crate::parsed::latest::PackageDescriptorCatalog;

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
pub struct LockedPackageCatalog {
    // region: original fields from the service
    // These fields are copied from the generated struct.
    pub attr_path: String,
    pub broken: Option<bool>,
    pub derivation: String,
    pub description: Option<String>,
    pub install_id: String,
    pub license: Option<String>,
    pub locked_url: String,
    pub name: String,
    pub pname: String,
    pub rev: String,
    pub rev_count: i64,
    #[cfg_attr(any(test, feature = "tests"), proptest(strategy = "chrono_strat()"))]
    pub rev_date: chrono::DateTime<chrono::offset::Utc>,
    #[cfg_attr(any(test, feature = "tests"), proptest(strategy = "chrono_strat()"))]
    pub scrape_date: chrono::DateTime<chrono::offset::Utc>,
    pub stabilities: Option<Vec<String>>,
    pub unfree: Option<bool>,
    pub version: String,
    pub outputs_to_install: Option<Vec<String>>,
    // endregion

    // region: converted fields
    /// A map of output name to store path
    pub outputs: BTreeMap<String, String>,
    // endregion

    // region: added fields
    pub system: System, // FIXME: this is an enum in the generated code, can't derive Arbitrary there
    pub group: String,
    // This was previously a `usize`, but in Nix `priority` is a `NixInt`, which is explicitly
    // a `uint64_t` instead of a `size_t`. Using a `u64` here matches those semantics, though in
    // reality it's likely not an issue.
    pub priority: u64,
    // endregion
}

impl LockedPackageCatalog {
    /// Construct a [LockedPackageCatalog] from a [ManifestPackageDescriptor],
    /// the resolved [catalog::PackageResolutionInfo], and corresponding [System].
    ///
    /// There may be more validation/parsing we could do here in the future.
    pub fn from_parts(
        package: catalog_types::ResolvedPackageDescriptor,
        descriptor: PackageDescriptorCatalog,
    ) -> Self {
        // unpack package to avoid missing new fields
        let catalog_types::ResolvedPackageDescriptor {
            catalog: _,
            attr_path,
            broken,
            derivation,
            description,
            // TODO: we should add this to LockedPackageCatalog
            insecure: _,
            install_id,
            license,
            locked_url,
            name,
            outputs,
            outputs_to_install,
            pname,
            rev,
            rev_count,
            rev_date,
            scrape_date,
            stabilities,
            unfree,
            version,
            system,
            cache_uri: _,
            pkg_path: _,
            missing_builds: _,
        } = package;

        let outputs = outputs
            .iter()
            .map(|output| (output.name.clone(), output.store_path.clone()))
            .collect::<BTreeMap<_, _>>();

        let priority = descriptor.priority.unwrap_or(DEFAULT_PRIORITY);
        let group = descriptor
            .pkg_group
            .as_deref()
            .unwrap_or(DEFAULT_GROUP_NAME)
            .to_string();

        LockedPackageCatalog {
            attr_path,
            broken,
            derivation,
            description,
            install_id,
            license,
            locked_url,
            name,
            outputs,
            outputs_to_install,
            pname,
            rev,
            rev_count,
            rev_date,
            // This field is deprecated and should be removed in the future,
            // currently it should always be populated, but if it's not, we can
            // default since it's not relied upon for anything downstream.
            scrape_date: scrape_date.unwrap_or(chrono::Utc::now()),
            stabilities,
            unfree,
            version,
            system: system.to_string(),
            priority,
            group,
        }
    }
}
