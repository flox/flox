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
use strum::IntoEnumIterator;

/// The server-computed status of a build.
///
/// Known variants serialize to their wire word (e.g. `TimedOut` ⇄ `timed_out`).
/// Any value outside the known set deserializes into [`Self::Unknown`] and
/// serializes back to the same string, so unknown statuses round-trip.
///
/// The two directions are deliberately asymmetric: deserialization (serde) is
/// tolerant so a response never fails on a new server status, while `FromStr`
/// (derived by strum, with [`Self::Unknown`] disabled) is strict so user input
/// is rejected unless it names a known status; the [`ParseStatusError`] it
/// returns names the accepted values. Iteration (`EnumIter`, also skipping the
/// disabled [`Self::Unknown`]) yields the known statuses in the order the
/// OpenAPI schema documents them.
#[derive(
    Clone,
    Debug,
    Deserialize,
    Serialize,
    PartialEq,
    Eq,
    Hash,
    strum::EnumIter,
    strum::EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(
    serialize_all = "snake_case",
    parse_err_fn = unknown_status,
    parse_err_ty = ParseStatusError
)]
pub enum EffectiveBuildStatus {
    Pending,
    Running,
    Completed,
    Failed,
    TimedOut,
    Cancelled,
    /// Any value outside the known statuses. MUST stay last: serde requires
    /// an untagged variant to trail the tagged ones so the known words are
    /// tried first.
    #[serde(untagged)]
    #[strum(disabled)]
    Unknown(String),
}

impl EffectiveBuildStatus {
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

/// Rejection of a word outside the known status vocabulary.
///
/// Produced by the enum's `FromStr`; the message names the accepted values,
/// so a caller can show it to a user as-is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseStatusError(String);

impl fmt::Display for ParseStatusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let valid = EffectiveBuildStatus::iter()
            .map(|status| status.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        write!(f, "Invalid status '{}'; valid values are: {valid}.", self.0)
    }
}

impl std::error::Error for ParseStatusError {}

/// The `parse_err_fn` for the strum-derived `FromStr`.
fn unknown_status(s: &str) -> ParseStatusError {
    ParseStatusError(s.to_string())
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator;

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

    /// `FromStr` is the strict direction: known wire words parse, anything
    /// else is rejected rather than falling into `Unknown`. Pins the strum
    /// wiring (`serialize_all` casing and the disabled catch-all) and the
    /// rejection message naming the accepted values.
    #[test]
    fn from_str_parses_known_words_and_rejects_the_rest() {
        assert_eq!("timed_out".parse(), Ok(EffectiveBuildStatus::TimedOut));
        assert_eq!(
            "queued".parse::<EffectiveBuildStatus>().unwrap_err().to_string(),
            "Invalid status 'queued'; valid values are: pending, running, completed, failed, timed_out, cancelled."
        );
        assert!("".parse::<EffectiveBuildStatus>().is_err());
    }

    /// Pins the known variants to the schema the client is generated from. If
    /// the server adds or reorders a status, this fails loudly so the enum is
    /// updated deliberately rather than the new value silently falling into
    /// `Unknown`.
    #[test]
    fn known_matches_openapi_schema() {
        let spec: serde_json::Value =
            serde_json::from_str(include_str!("../openapi.json")).unwrap();
        let schema_values: Vec<String> = spec["components"]["schemas"]["EffectiveBuildStatus"]
            ["enum"]
            .as_array()
            .expect("EffectiveBuildStatus.enum is an array")
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .expect("enum value is a string")
                    .to_string()
            })
            .collect();
        let known: Vec<String> = EffectiveBuildStatus::iter()
            .map(|status| status.to_string())
            .collect();
        assert_eq!(schema_values, known);
    }
}
