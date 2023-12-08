use std::collections::BTreeMap;

use runix::DerivationPath;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_with::skip_serializing_none;

use crate::catalog::*;

#[skip_serializing_none]
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
/// Proof that a package has been successfully evaluated.
///
/// `Eval` contains the same information as produced by `nix eval`.
/// A successful eval means output paths are known, although the content of
/// those paths is not known.
/// This means `Eval` can function as an eval cache.
pub struct Eval {
    pub attr_path: Option<AttrPath>,
    pub drv_path: Option<DerivationPath>,
    pub meta: Meta,
    pub name: String,
    pub namespace: Option<Vec<String>>,
    pub outputs: BTreeMap<String, StorePath>,
    // not all packages have pname; a random example from Nixpkgs is _3llo
    pub pname: Option<String>,
    pub stability: Option<String>,
    pub system: System,
    pub version: PackageVersion,
}

#[skip_serializing_none]
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Meta {
    pub description: Option<String>,
    pub outputs_to_install: Option<Vec<String>>,
    pub unfree: bool,
    #[serde(flatten)]
    pub _other: BTreeMap<String, Value>,
}
