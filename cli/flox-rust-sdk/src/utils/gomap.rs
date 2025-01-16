use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Representation of Go's `struct{}`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct GoEmptyStruct(BTreeMap<(), ()>);

/// Representation of Go's `map[string]struct{}`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct GoMap(BTreeMap<String, GoEmptyStruct>);

impl From<Vec<String>> for GoMap {
    fn from(v: Vec<String>) -> Self {
        GoMap(
            v.into_iter()
                .map(|s| (s, GoEmptyStruct::default()))
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use serde_json;

    use super::*;

    #[test]
    fn new() {
        let vals = vec!["aaa".to_string(), "bbb".to_string(), "ccc".to_string()];
        let gomap = GoMap::from(vals);
        let json = serde_json::to_string(&gomap).unwrap();

        assert_eq!(json, r#"{"aaa":{},"bbb":{},"ccc":{}}"#);
    }
}
