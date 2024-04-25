use std::fmt::Debug;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, JsonSchema)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub struct Version<const V: u8>;

impl<const V: u8> Default for Version<V> {
    fn default() -> Self {
        Self
    }
}

impl<const V: u8> Debug for Version<V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Version").field("value", &V).finish()
    }
}

#[derive(Debug, Error)]
#[error("Invalid version")]
struct VersionError;

impl<const V: u8> Serialize for Version<V> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_u8(V)
    }
}

impl<'de, const V: u8> Deserialize<'de> for Version<V> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = u8::deserialize(deserializer)?;
        if value == V {
            Ok(Version::<V>)
        } else {
            Err(serde::de::Error::custom(VersionError))
        }
    }
}

#[cfg(test)]
mod tests {

    #[derive(Serialize, Deserialize, Debug)]
    #[serde(untagged)]
    pub enum Bla {
        V2 {
            foo: String,
            version: Version<2>,
        },

        V1 {
            hello: String,
            // implicit/optional
            #[serde(default)]
            version: Version<1>,
        },
    }

    use serde_json::json;

    use super::*;

    #[test]
    fn parse_v1() {
        let bla = serde_json::from_value::<Bla>(json!({
            "hello": "world",
            "version": 1
        }))
        .expect("Should parse explicit V1");

        assert!(matches!(bla, Bla::V1 { .. }));

        let bla = serde_json::from_value::<Bla>(json!({
            "hello": "world",
        }))
        .expect("Should parse implicit V1");

        assert!(matches!(bla, Bla::V1 { .. }));

        serde_json::from_value::<Bla>(json!({
            "hello": "world",
            "version": 3
        }))
        .expect_err("Shouldn't parse wrong version");
    }

    #[test]
    fn parse_v2() {
        let bla = serde_json::from_value::<Bla>(json!({
            "foo": "bar",
            "version": 2
        }))
        .expect("Should parse explicit V2");

        assert!(matches!(bla, Bla::V2 { .. }));

        serde_json::from_value::<Bla>(json!({
            "foo": "bar",
        }))
        .expect_err("Should'nt parse implicit V2");

        serde_json::from_value::<Bla>(json!({
            "foo": "bar",
            "version": 1
        }))
        .expect_err("Shouldn't parse wrong version");
    }
}
