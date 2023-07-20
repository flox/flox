use runix::types::DerivationPath;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use crate::catalog::*;
use crate::constants::DEFAULT_CHANNEL;
use crate::stability::Stability;

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

impl PublishElement {
    /// TODO drop once we have an actual FloxTuple type
    pub fn to_flox_tuple(&self) -> String {
        let mut result = Vec::new();
        if self.stability != Stability::default() {
            result.push(self.stability.to_string())
        }
        if self.original_url != DEFAULT_CHANNEL {
            result.push(self.original_url.clone())
        }
        result.extend(self.namespace.clone());
        result.join(".")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Default stability and channel are not printed
    #[test]
    fn to_flox_tuple_publish_element() {
        let publish_element = PublishElement {
            namespace: vec![
                "python3".to_string(),
                "pkgs".to_string(),
                "requests".to_string(),
            ],
            original_url: "nixpkgs-flox".to_string(),
            stability: Stability::default(),
            system: "dummy".to_string(),
            version: "dummy".to_string(),
        };
        assert_eq!(publish_element.to_flox_tuple(), "python3.pkgs.requests");
    }

    /// Non-default stability and channel are printed
    #[test]
    fn to_flox_tuple_publish_element_no_default() {
        let publish_element = PublishElement {
            namespace: vec![
                "python3".to_string(),
                "pkgs".to_string(),
                "requests".to_string(),
            ],
            original_url: "nixpkgs-acme".to_string(),
            stability: Stability::Staging,
            system: "dummy".to_string(),
            version: "dummy".to_string(),
        };
        assert_eq!(
            publish_element.to_flox_tuple(),
            "staging.nixpkgs-acme.python3.pkgs.requests"
        );
    }

    /// Non-default stability is printed even when default channel is not
    #[test]
    fn to_flox_tuple_publish_element_default_channel() {
        let publish_element = PublishElement {
            namespace: vec![
                "python3".to_string(),
                "pkgs".to_string(),
                "requests".to_string(),
            ],
            original_url: "nixpkgs-flox".to_string(),
            stability: Stability::Staging,
            system: "dummy".to_string(),
            version: "dummy".to_string(),
        };
        assert_eq!(
            publish_element.to_flox_tuple(),
            "staging.python3.pkgs.requests"
        );
    }

    /// Non-default channel is printed even when default stability is not
    #[test]
    fn to_flox_tuple_publish_element_default_stability() {
        let publish_element = PublishElement {
            namespace: vec![
                "python3".to_string(),
                "pkgs".to_string(),
                "requests".to_string(),
            ],
            original_url: "nixpkgs-acme".to_string(),
            stability: Stability::default(),
            system: "dummy".to_string(),
            version: "dummy".to_string(),
        };
        assert_eq!(
            publish_element.to_flox_tuple(),
            "nixpkgs-acme.python3.pkgs.requests"
        );
    }
}
