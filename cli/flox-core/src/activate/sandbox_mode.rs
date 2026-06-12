use std::fmt::Display;
use std::str::FromStr;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The filesystem-mediation level requested for an activation.
///
/// `Off` is the default and matches the historical behavior where no
/// sandbox is applied. `Warn`, `Enforce`, and `Prompt` correspond to the
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
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
pub enum SandboxMode {
    /// No sandbox is applied (default).
    #[default]
    Off,
    /// Out-of-policy access is reported but permitted.
    Warn,
    /// Out-of-policy access is denied.
    Enforce,
    /// Out-of-policy access is denied and queued for approval.
    ///
    /// The serde alias keeps activation-state files, context files, and
    /// manifests written while this mode was called `ask` parsing for one
    /// release cycle; remove the alias (and the matching `FromStr` arm)
    /// after that.
    #[serde(alias = "ask")]
    Prompt,
}

impl Display for SandboxMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SandboxMode::Off => write!(f, "off"),
            SandboxMode::Warn => write!(f, "warn"),
            SandboxMode::Enforce => write!(f, "enforce"),
            SandboxMode::Prompt => write!(f, "prompt"),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("'{0}' is not a valid sandbox mode. Expected one of: off, warn, enforce, prompt.")]
pub struct SandboxModeParseError(String);

impl FromStr for SandboxMode {
    type Err = SandboxModeParseError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "off" => Ok(SandboxMode::Off),
            "warn" => Ok(SandboxMode::Warn),
            "enforce" => Ok(SandboxMode::Enforce),
            "prompt" => Ok(SandboxMode::Prompt),
            // Transitional alias from the prototype's mode name; remove with
            // the serde alias above.
            "ask" => Ok(SandboxMode::Prompt),
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
            (SandboxMode::Prompt, "prompt"),
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
            "'bogus' is not a valid sandbox mode. Expected one of: off, warn, enforce, prompt.",
        );
    }

    #[test]
    fn serde_round_trips_kebab_case() {
        let cases = [
            (SandboxMode::Off, "\"off\""),
            (SandboxMode::Warn, "\"warn\""),
            (SandboxMode::Enforce, "\"enforce\""),
            (SandboxMode::Prompt, "\"prompt\""),
        ];

        for (mode, json) in cases {
            assert_eq!(serde_json::to_string(&mode).unwrap(), json);
            assert_eq!(serde_json::from_str::<SandboxMode>(json).unwrap(), mode);
        }
    }

    #[test]
    fn legacy_ask_still_parses_as_prompt() {
        // Activation-state files, context files, and manifests written while
        // this mode was called `ask` must keep working for one release cycle.
        assert_eq!("ask".parse::<SandboxMode>().unwrap(), SandboxMode::Prompt);
        assert_eq!(
            serde_json::from_str::<SandboxMode>("\"ask\"").unwrap(),
            SandboxMode::Prompt
        );
    }
}
