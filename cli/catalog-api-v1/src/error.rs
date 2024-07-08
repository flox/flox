use std::hash::Hash;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(untagged)]
pub enum MessageType {
    #[serde(rename = "general")]
    General,
    #[serde(rename = "resolution_trace")]
    ResolutionTrace,
    #[serde(rename = "attr_path_not_found")]
    AttrPathNotFound,
    #[serde(rename = "constraints_too_tight")]
    ConstraintsTooTight,

    Unknown(String),
}
