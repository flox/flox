use serde::{Deserialize, Serialize};
use serde_with::{serde_as, skip_serializing_none, DisplayFromStr};
use url::Url;

#[skip_serializing_none]
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
/// Metadata about the locked version of source.
///
/// Possibly semantically similar to `element.url`?
pub struct Source {
    pub locked: Locked,
    pub original: Option<Unlocked>,
    pub remote: Option<Unlocked>,
}

/// TODO use runix FlakeRef
#[serde_as]
#[skip_serializing_none]
#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct Locked {
    last_modified: u32,
    nar_hash: Option<String>,
    #[serde(rename = "ref")]
    ref_: Option<String>,
    rev: Option<String>,
    rev_count: u32,
    #[serde(rename = "type")]
    type_: Option<String>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    url: Option<Url>,
}

/// TODO use runix FlakeRef
#[serde_as]
#[skip_serializing_none]
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct Unlocked {
    #[serde(rename = "ref")]
    ref_: Option<String>,
    rev: Option<String>,
    #[serde(rename = "type")]
    type_: Option<String>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    url: Option<Url>,
}
