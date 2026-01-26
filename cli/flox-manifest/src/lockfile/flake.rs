use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use serde_with::skip_serializing_none;

use crate::parsed::common::DEFAULT_PRIORITY;

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
pub struct LockedPackageFlake {
    pub install_id: String,
    /// Unaltered lock information as returned by `lock-flake-installable`.
    /// In this case we completely own the data format in this repo
    /// and so far have to do no conversion.
    /// If this changes in the future, we can add a conversion layer here
    /// similar to [LockedPackageCatalog::from_parts].
    #[serde(flatten)]
    pub locked_installable: LockedInstallable,
}

impl LockedPackageFlake {
    /// Construct a [LockedPackageFlake] from an [LockedInstallable] and an install_id.
    /// In the future, we may want to pass the original descriptor here as well,
    /// similar to [LockedPackageCatalog::from_parts].
    pub fn from_parts(install_id: String, locked_installable: LockedInstallable) -> Self {
        LockedPackageFlake {
            install_id,
            locked_installable,
        }
    }
}

/// Rust representation of the output of `buitins.lockFlakeInstallable`
/// This is a direct translation of the definition in
/// `<flox>/nix-plugins/include/flox/lock-flake-installable.hh`
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
// [sic] this is inconsistent with the naming of all other structs in the lockfile
// and a relict of different naming conventions in the pkgdb/C++ code.
#[serde(rename_all = "kebab-case")]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
pub struct LockedInstallable {
    /// locked url of the flakeref component of the installable
    pub locked_url: String,
    pub flake_description: Option<String>,
    pub locked_flake_attr_path: String,
    pub derivation: String,
    /// Map of output names to their paths
    /// The values are expected to be nix store paths
    pub outputs: BTreeMap<String, String>,
    /// List of output names in the original order
    pub output_names: Vec<String>,
    /// List of output names to install as defined by the package
    pub outputs_to_install: Option<Vec<String>>,
    /// List of output names to install as requested by the user
    pub requested_outputs_to_install: Option<Vec<String>>,
    /// System as defined by the package
    pub package_system: String,
    /// System as specified by the manifest and used to set default attribute
    /// paths when locking the installable
    pub system: String,
    pub name: String,
    pub pname: Option<String>,
    pub version: Option<String>,
    pub description: Option<String>,
    pub licenses: Option<Vec<String>>,
    pub broken: Option<bool>,
    pub unfree: Option<bool>,
    // In the lockfile, the priority should always be known.
    // Usage of the output type of `buitins.lockFlakeInstallable`,
    // however requires guarding against a missing priority.
    // Since the default priority is not known statically,
    // we assign it as a default value during deserialization.
    #[serde(
        deserialize_with = "locked_installable_default_priority_on_null",
        default = "locked_installable_default_priority_on_undefined"
    )]
    pub priority: u64,
}

/// Deserialize the priority field of a locked installable.
/// `buitins.lockFlakeInstallable` will yield a `null` priority
/// if the priority is not set, which requires a custom deserializer
/// to set the default priority.
fn locked_installable_default_priority_on_null<'de, D>(d: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    Deserialize::deserialize(d).map(|x: Option<_>| x.unwrap_or(DEFAULT_PRIORITY))
}

/// Default priority for a locked installable if the priority is not set,
/// as we may remove null attributes during serialization.
fn locked_installable_default_priority_on_undefined() -> u64 {
    DEFAULT_PRIORITY
}
