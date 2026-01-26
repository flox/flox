use flox_core::data::System;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
#[skip_serializing_none]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
pub struct LockedPackageStorePath {
    /// The install_id of the descriptor in the manifest
    pub install_id: String,
    /// Store path to add to the environment
    pub store_path: String,
    pub system: System,
    pub priority: u64,
}
