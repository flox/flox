use std::fmt::Display;

use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use crate::catalog::*;
use crate::constants::*;

#[skip_serializing_none]
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
/// Metadata needed to install a package and upgrade it over time.
///
/// The name "element" originally comes from the `elements` block of a Nix
/// profile's `manifest.json`.
/// Currently some of this information is duplicated from other portions of the
/// catalog, so it may be dropped from `Element`.
///
/// The important data at this point is `original_url` and `url`.
pub struct Element {
    pub active: Option<bool>,
    pub attr_path: AttrPath,
    /// Describes user intention rather than locked information.
    /// This allows upgrading over time.
    ///
    /// `original_url` may be indirect, and fetching it may be impure.
    pub original_url: Option<FlakeRef>,
    pub store_paths: Vec<DerivationPath>,
    /// The result of locking `original_url`.
    ///
    /// Fetching `url` is pure, and it allow reproducing a package.
    pub url: FlakeRef,
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
/// Metadata about the source of a catalog entry.
///
/// In other words, this is a publish of a publish.
/// All information in this struct could be represented as a single FlakeRef.
pub struct PublishElement {
    /// Must have length >= 1
    pub namespace: Namespace,
    pub original_url: FlakeRef,
    pub stability: Stability,
    pub system: System,
    pub version: PackageVersion,
}

impl Display for PublishElement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut result = Vec::new();
        if self.stability != DEFAULT_STABILITY {
            result.push(self.stability.clone())
        }
        if self.original_url != DEFAULT_CHANNEL {
            result.push(self.original_url.clone())
        }
        result.extend(self.namespace.clone());
        write!(f, "{}", result.join("."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_publish_element() {
        // Default stability and channel are not printed
        let publish_element = PublishElement {
            namespace: vec![
                "python3".to_string(),
                "pkgs".to_string(),
                "requests".to_string(),
            ],
            original_url: "nixpkgs-flox".to_string(),
            stability: "stable".to_string(),
            system: "dummy".to_string(),
            version: "dummy".to_string(),
        };
        assert_eq!(format!("{publish_element}"), "python3.pkgs.requests");

        // Non-default stability and channel are printed
        let publish_element = PublishElement {
            namespace: vec![
                "python3".to_string(),
                "pkgs".to_string(),
                "requests".to_string(),
            ],
            original_url: "nixpkgs-acme".to_string(),
            stability: "staging".to_string(),
            system: "dummy".to_string(),
            version: "dummy".to_string(),
        };
        assert_eq!(
            format!("{publish_element}"),
            "staging.nixpkgs-acme.python3.pkgs.requests"
        );
    }
}
