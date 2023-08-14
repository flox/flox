use std::{collections::HashMap, path::PathBuf, time::SystemTime};

use serde::{Deserialize, Serialize};
use serde_with::serde_as;

use flox_types::catalog::cache::Narinfo;

#[serde_as]
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Cache(#[serde_as(as = "Vec<(_,_)>")] HashMap<(String, PathBuf), CacheItem>);

impl Cache {
    pub fn get(&self, key: &(String, PathBuf)) -> Option<&CacheItem> {
        self.0.get(key)
    }

    pub fn insert(&mut self, key: (String, PathBuf), value: CacheItem) -> Option<CacheItem> {
        self.0.insert(key, value)
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct CacheItem {
    pub ts: SystemTime,
    pub narinfo: Narinfo,
}
