use std::collections::HashMap;

use serde::ser::SerializeSeq;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_with::{serde_as, skip_serializing_none, DisplayFromStr};
use url::Url;

use crate::catalog::*;

pub type SubstituterUrl = Url;

/// Metadata about whether package outputs have been cached.
///
/// Unlike other sections of the catalog, a `Cache` object does not represent
/// invariants; it may in fact store the information that a package has _not_
/// been cached.
#[derive(Default)]
pub struct Cache(pub HashMap<SubstituterUrl, CacheMeta>);

impl<'de> Deserialize<'de> for Cache {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let metas: Vec<CacheMeta> = Vec::deserialize(deserializer)?;
        let pairs = metas.into_iter().map(|meta| (meta.cache_url.clone(), meta));
        Ok(Cache(HashMap::from_iter(pairs)))
    }
}

impl Serialize for Cache {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let count = self.0.len();

        let mut ser = serializer.serialize_seq(Some(count))?;
        for meta in self.0.values() {
            ser.serialize_element(meta)?
        }
        ser.end()
    }
}

impl Cache {
    pub fn add(&mut self, cache_meta: CacheMeta) -> () {
        self.0.insert(cache_meta.cache_url.clone(), cache_meta);
    }
}

/// Represents all cache entries of all outputs found in one substituter
#[serde_as]
#[skip_serializing_none]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct CacheMeta {
    #[serde_as(as = "DisplayFromStr")]
    pub cache_url: SubstituterUrl,
    pub narinfo: Vec<Narinfo>,
}

fn default_true() -> bool {
    true
}

/// Narinfo stores information output by `nix path-info --json`
#[derive(Serialize, Deserialize, Clone)]
pub struct Narinfo {
    pub path: DerivationPath,
    // TODO remove this default once https://github.com/NixOS/nix/pull/7924 has
    // made it's way into our verison of Nix
    #[serde(default = "default_true")]
    pub valid: bool,
    // TODO add other fields
    #[serde(flatten)]
    _other: HashMap<String, Value>,
}
