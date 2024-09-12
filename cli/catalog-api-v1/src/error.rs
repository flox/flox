use std::hash::Hash;

use serde::{Deserialize, Serialize};

/// Type of the error returned by the catalog API
/// Since we were unable to represent earlier error structures returned by the API
/// using the progenitor client generator,
/// errors are now serialized as a blob of values
/// (`context` in [crate::ResolutionMessageGeneral]).
///
/// The context may be parsed into a higher level structure later,
/// or ignored in which case the `message` field in [crate::ResolutionMessageGeneral]
/// is expected to provide a relevant fallback message.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum MessageType {
    #[serde(rename = "general")]
    General,
    #[serde(rename = "resolution_trace")]
    ResolutionTrace,
    #[serde(rename = "attr_path_not_found")]
    AttrPathNotFound,
    #[serde(rename = "constraints_too_tight")]
    ConstraintsTooTight,

    #[serde(untagged)]
    Unknown(String),
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::json;

    use super::*;
    #[test]
    #[ignore = "useful when developing"]
    fn deserializes_known_and_unknown_variants() {
        let map: HashMap<String, MessageType> = serde_json::from_value(json!({
         "known_type": "constraints_too_tight",
         "unknown_type": "something unknown"
        }))
        .unwrap();

        assert_eq!(map["known_type"], MessageType::ConstraintsTooTight);
        assert_eq!(
            map["unknown_type"],
            MessageType::Unknown("something unknown".to_string())
        );
    }
}
