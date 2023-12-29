use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub type FlakeRef = Value;

#[derive(Deserialize, PartialEq, Serialize)]
pub struct Input {
    pub from: FlakeRef,
    #[serde(flatten)]
    _json: Value,
}

#[derive(Deserialize, Serialize)]
pub struct Registry {
    pub inputs: BTreeMap<String, Input>,
    #[serde(flatten)]
    _json: Value,
}

/// An environment (or global) lockfile.
///
/// The authoritative definition of this structure is in C++. This struct is
/// used as the format to communicate with pkgdb. Many pkgdb commands will need
/// to pass some of the information in the lockfile through to Rust. Although we
/// could selectively pass fields through, I'm hoping it will be easier to parse
/// the entirety of the lockfile in Rust rather than defining a separate set of
/// fields for each different pkgdb command.
#[derive(Deserialize, Serialize)]
pub struct Lockfile {
    pub registry: Registry,
    #[serde(flatten)]
    _json: Value,
}
