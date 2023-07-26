use std::collections::HashMap;

use runix::narinfo::Narinfo;
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
#[derive(Clone, Default)]
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
    pub fn add(&mut self, cache_meta: CacheMeta) {
        self.0.insert(cache_meta.cache_url.clone(), cache_meta);
    }
}

/// Represents all cache entries of all outputs found in one substituter
#[serde_as]
#[skip_serializing_none]
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheMeta {
    #[serde_as(as = "DisplayFromStr")]
    pub cache_url: SubstituterUrl,
    pub narinfo: Vec<Narinfo>,
    #[serde(flatten)]
    pub _other: BTreeMap<String, Value>,
}
