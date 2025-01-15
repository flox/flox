use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Representation of Go's `struct{}`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct GoEmptyStruct(BTreeMap<(), ()>);

/// Representation of Go's `map[string]struct{}`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct GoMap(BTreeMap<String, GoEmptyStruct>);

impl GoMap {
    // Construct a new GoMap from a vec of keys.
    pub fn new(keys: Vec<String>) -> Self {
        GoMap(
            keys.into_iter()
                .map(|key| (key, GoEmptyStruct::default()))
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use serde_json;

    #[test]
    fn new() {
        let keys = vec!["a".to_string(), "b".to_string()];
        let gomap = super::GoMap::new(keys);
        let json = serde_json::to_string(&gomap).unwrap();
        assert_eq!(json, r#"{"a":{},"b":{}}"#);
    }
}
