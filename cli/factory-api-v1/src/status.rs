//! Hand-written, tolerant replacement for the generated `BuildResponse.status`
//! enum.
//!
//! The Factory Service computes an effective build status server-side. We
//! deserialize it into a closed set of known variants plus an open
//! [`EffectiveBuildStatus::Unknown`] catch-all: a status the server adds in the
//! future renders as `unknown: <value>` rather than failing the whole response
//! and blanking the build list. Progenitor generates the endpoint bindings; this
//! type is spliced in via `with_replacement` in `build.rs` so the same tolerance
//! covers both the response body and the `status` query-param filter.

use std::fmt;

use serde::{Deserialize, Serialize};

/// The server-computed status of a build.
///
/// Known variants serialize to their wire word (e.g. `TimedOut` ⇄ `timed_out`).
/// Any value outside the known set deserializes into [`Self::Unknown`] and
/// serializes back to the same string, so unknown statuses round-trip.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
pub enum EffectiveBuildStatus {
    #[serde(rename = "pending")]
    Pending,
    #[serde(rename = "running")]
    Running,
    #[serde(rename = "completed")]
    Completed,
    #[serde(rename = "failed")]
    Failed,
    #[serde(rename = "timed_out")]
    TimedOut,
    #[serde(rename = "cancelled")]
    Cancelled,
    /// Any value outside the known statuses. MUST stay last: serde requires
    /// an untagged variant to trail the tagged ones so the known words are
    /// tried first.
    #[serde(untagged)]
    Unknown(String),
}

impl EffectiveBuildStatus {
    /// The known statuses, in the order the OpenAPI schema documents them.
    /// Excludes [`Self::Unknown`], which is open-ended.
    pub const KNOWN: [Self; 6] = [
        Self::Pending,
        Self::Running,
        Self::Completed,
        Self::Failed,
        Self::TimedOut,
        Self::Cancelled,
    ];

    /// The wire word for this status. For [`Self::Unknown`] this is the
    /// original, unrecognized value.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::TimedOut => "timed_out",
            Self::Cancelled => "cancelled",
            Self::Unknown(value) => value,
        }
    }
}

impl fmt::Display for EffectiveBuildStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_known_variant() {
        let status: EffectiveBuildStatus = serde_json::from_str(r#""timed_out""#).unwrap();
        assert_eq!(status, EffectiveBuildStatus::TimedOut);
    }

    #[test]
    fn deserializes_unknown_variant_tolerantly() {
        let status: EffectiveBuildStatus = serde_json::from_str(r#""queued""#).unwrap();
        assert_eq!(status, EffectiveBuildStatus::Unknown("queued".to_string()));
    }

    #[test]
    fn serializes_known_and_unknown_to_wire_word() {
        assert_eq!(
            serde_json::to_string(&EffectiveBuildStatus::Cancelled).unwrap(),
            r#""cancelled""#,
        );
        assert_eq!(
            serde_json::to_string(&EffectiveBuildStatus::Unknown("frobnicated".to_string()))
                .unwrap(),
            r#""frobnicated""#,
        );
    }

    #[test]
    fn display_matches_as_str() {
        assert_eq!(EffectiveBuildStatus::Pending.to_string(), "pending");
        assert_eq!(
            EffectiveBuildStatus::Unknown("weird".to_string()).to_string(),
            "weird",
        );
    }

    /// Pins the hand-written [`EffectiveBuildStatus::KNOWN`] set to the schema
    /// the client is generated from. If the server adds or reorders a status,
    /// this fails loudly so the enum is updated deliberately rather than the
    /// new value silently falling into `Unknown`.
    #[test]
    fn known_matches_openapi_schema() {
        let spec: serde_json::Value =
            serde_json::from_str(include_str!("../openapi.json")).unwrap();
        let schema_values: Vec<&str> = spec["components"]["schemas"]["EffectiveBuildStatus"]
            ["enum"]
            .as_array()
            .expect("EffectiveBuildStatus.enum is an array")
            .iter()
            .map(|value| value.as_str().expect("enum value is a string"))
            .collect();
        let known: Vec<&str> = EffectiveBuildStatus::KNOWN
            .iter()
            .map(EffectiveBuildStatus::as_str)
            .collect();
        assert_eq!(schema_values, known);
    }
}
