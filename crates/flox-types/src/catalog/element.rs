use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use crate::catalog::*;

#[skip_serializing_none]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
/// Metadata needed to install a package and upgrade it over time.
///
/// The name "element" originally comes from the `elements` block of a Nix
/// profile's `manifest.json`.
/// Currently some of this information is duplicated from other portions of the
/// catalog, so it may be dropped from `Element`.
///
/// The important data at this point is `original_url` and `url`.
pub struct Element {
    pub active: Option<bool>,
    pub attr_path: AttrPath,
    /// Describes user intention rather as opposed to locked information.
    /// This allows upgrading over time.
    ///
    /// `original_url` may be indirect, and fetching it may be impure.
    pub original_url: Option<FlakeRef>,
    pub store_paths: Vec<DerivationPath>,
    /// The result of locking `original_url`.
    ///
    /// Fetching `url` is pure, and it allow reproducing a package.
    pub url: FlakeRef,
}
