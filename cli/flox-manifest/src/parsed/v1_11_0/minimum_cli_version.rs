use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The minimum CLI version required to use the environment.
///
/// Accepts either a plain version string:
///   minimum-cli-version = "1.11.0"
///
/// Or an inline table with a reason:
///   minimum-cli-version = { version = "1.11.0", reason = "needs feature X" }
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
#[serde(
    untagged,
    expecting = "Expected a version string or { version = \"<version>\", reason = \"<reason>\" }"
)]
pub enum MinimumCliVersion {
    Version(
        #[schemars(with = "String")]
        #[cfg_attr(
            any(test, feature = "tests"),
            proptest(strategy = "arbitrary_semver_version()")
        )]
        semver::Version,
    ),
    WithReason {
        #[schemars(with = "String")]
        #[cfg_attr(
            any(test, feature = "tests"),
            proptest(strategy = "arbitrary_semver_version()")
        )]
        version: semver::Version,
        reason: String,
    },
}

impl MinimumCliVersion {
    pub fn version(&self) -> &semver::Version {
        match self {
            Self::Version(v) => v,
            Self::WithReason { version, .. } => version,
        }
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Version(_) => None,
            Self::WithReason { reason, .. } => Some(reason.as_str()),
        }
    }
}

#[cfg(any(test, feature = "tests"))]
fn arbitrary_semver_version() -> impl proptest::strategy::Strategy<Value = semver::Version> {
    use proptest::prelude::*;
    (0..100u64, 0..100u64, 0..100u64).prop_map(|(ma, mi, pa)| semver::Version::new(ma, mi, pa))
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn version_accessor_for_plain_variant() {
        let mcv = MinimumCliVersion::Version(semver::Version::new(1, 0, 0));
        assert_eq!(*mcv.version(), semver::Version::new(1, 0, 0));
        assert_eq!(mcv.reason(), None);
    }

    #[test]
    fn version_and_reason_accessors_for_with_reason_variant() {
        let mcv = MinimumCliVersion::WithReason {
            version: semver::Version::new(2, 3, 4),
            reason: "needs feature X".to_string(),
        };
        assert_eq!(*mcv.version(), semver::Version::new(2, 3, 4));
        assert_eq!(mcv.reason(), Some("needs feature X"));
    }
}
