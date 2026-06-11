use std::fmt::Display;
use std::str::FromStr;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The filesystem-mediation level requested for an activation.
///
/// `Off` is the default and matches the historical behavior where no
/// sandbox is applied. `Warn`, `Enforce`, and `Ask` correspond to the
/// libsandbox levels of the same name; this type only carries the
/// selection through the CLI and activation context — it does not by
/// itself change libsandbox behavior.
#[derive(
    Debug,
    Clone,
    Copy,
    Serialize,
    Deserialize,
    Hash,
    PartialEq,
    Eq,
    Ord,
    PartialOrd,
    Default,
    JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum SandboxMode {
    /// No sandbox is applied (default).
    #[default]
    Off,
    /// Out-of-policy access is reported but permitted.
    Warn,
    /// Out-of-policy access is denied.
    Enforce,
    /// Out-of-policy access is denied and queued for approval.
    Ask,
}

impl Display for SandboxMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SandboxMode::Off => write!(f, "off"),
            SandboxMode::Warn => write!(f, "warn"),
            SandboxMode::Enforce => write!(f, "enforce"),
            SandboxMode::Ask => write!(f, "ask"),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("'{0}' is not a valid sandbox mode. Expected one of: off, warn, enforce, ask.")]
pub struct SandboxModeParseError(String);

impl FromStr for SandboxMode {
    type Err = SandboxModeParseError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "off" => Ok(SandboxMode::Off),
            "warn" => Ok(SandboxMode::Warn),
            "enforce" => Ok(SandboxMode::Enforce),
            "ask" => Ok(SandboxMode::Ask),
            other => Err(SandboxModeParseError(other.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_off() {
        assert_eq!(SandboxMode::default(), SandboxMode::Off);
    }

    #[test]
    fn display_and_from_str_round_trip() {
        let cases = [
            (SandboxMode::Off, "off"),
            (SandboxMode::Warn, "warn"),
            (SandboxMode::Enforce, "enforce"),
            (SandboxMode::Ask, "ask"),
        ];

        for (mode, rendered) in cases {
            assert_eq!(mode.to_string(), rendered);
            assert_eq!(rendered.parse::<SandboxMode>().unwrap(), mode);
        }
    }

    #[test]
    fn from_str_rejects_unknown_value() {
        let err = "bogus".parse::<SandboxMode>().unwrap_err();
        assert_eq!(
            err.to_string(),
            "'bogus' is not a valid sandbox mode. Expected one of: off, warn, enforce, ask.",
        );
    }

    #[test]
    fn serde_round_trips_kebab_case() {
        let cases = [
            (SandboxMode::Off, "\"off\""),
            (SandboxMode::Warn, "\"warn\""),
            (SandboxMode::Enforce, "\"enforce\""),
            (SandboxMode::Ask, "\"ask\""),
        ];

        for (mode, json) in cases {
            assert_eq!(serde_json::to_string(&mode).unwrap(), json);
            assert_eq!(serde_json::from_str::<SandboxMode>(json).unwrap(), mode);
        }
    }
}
