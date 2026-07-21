//! Author manifest (`flox-extension.toml`) and installed-state record
//! (`state.toml`).
//!
//! Plain `serde` structs with a `schema = "1"` string field. The
//! type-state pattern from `flox-manifest::Manifest<S>` is deliberately
//! not used here: there is one schema version and no migration history.
//!
//! P02 reads `[extension] name`. The other fields (`[extension.binary]`,
//! `[environment]`, `[on_active]`) are parsed into typed structs so the
//! schema is locked in now and round-tripped by tests; P03/P04/P06 wire
//! them into behavior without a schema bump.

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("failed to parse manifest TOML: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("failed to serialize manifest TOML: {0}")]
    Serialize(#[from] toml::ser::Error),
}

/// `flox-extension.toml` — author-supplied, optional in the source tree.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorManifest {
    #[serde(default = "default_schema")]
    pub schema: String,
    pub extension: ExtensionMeta,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<EnvironmentBehavior>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_active: Option<OnActive>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionMeta {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary: Option<BinaryMeta>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BinaryMeta {
    pub source: String,
    pub asset: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentBehavior {
    pub mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherit: Option<InheritMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherit_name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InheritMode {
    Current,
    Default,
    Named,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OnActive {
    pub inside: String,
}

/// `state.toml` — written by `install`, consumed by `list` / `remove` /
/// `upgrade`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledState {
    #[serde(default = "default_schema")]
    pub schema: String,
    pub name: String,
    pub kind: String,
    pub source: String,
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    pub repo: String,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub tag: String,
    /// Default branch name as observed at install time (used by `upgrade`
    /// to know which ref to fetch). Empty for tag-pinned and commit-pinned
    /// installs that didn't go through the default-branch fallback.
    #[serde(default)]
    pub branch: String,
    #[serde(default)]
    pub commit: String,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub asset_sha256: String,
    pub installed_at: String,
    pub path: String,
}

fn default_schema() -> String {
    "1".to_string()
}

pub fn parse_author_manifest(s: &str) -> Result<AuthorManifest, ManifestError> {
    Ok(toml::from_str(s)?)
}

pub fn parse_installed_state(s: &str) -> Result<InstalledState, ManifestError> {
    Ok(toml::from_str(s)?)
}

pub fn render_installed_state(state: &InstalledState) -> Result<String, ManifestError> {
    Ok(toml::to_string(state)?)
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn author_manifest_minimal_round_trip() {
        let src = r#"
schema = "1"

[extension]
name = "hello"
"#;
        let parsed = parse_author_manifest(src).unwrap();
        let expected = AuthorManifest {
            schema: "1".to_string(),
            extension: ExtensionMeta {
                name: "hello".to_string(),
                description: None,
                binary: None,
            },
            environment: None,
            on_active: None,
        };
        assert_eq!(parsed, expected);

        let rendered = toml::to_string(&parsed).unwrap();
        let reparsed = parse_author_manifest(&rendered).unwrap();
        assert_eq!(reparsed, expected);
    }

    #[test]
    fn author_manifest_full_round_trip() {
        let src = AuthorManifest {
            schema: "1".to_string(),
            extension: ExtensionMeta {
                name: "deploy".to_string(),
                description: Some("Deploys things".to_string()),
                binary: Some(BinaryMeta {
                    source: "github-release".to_string(),
                    asset: "flox-deploy-{platform}.tar.gz".to_string(),
                    sha256: Some("cafe".to_string()),
                }),
            },
            environment: Some(EnvironmentBehavior {
                mode: "pinned".to_string(),
                inherit: Some(InheritMode::Named),
                inherit_name: Some("dev".to_string()),
            }),
            on_active: Some(OnActive {
                inside: "use-active".to_string(),
            }),
        };
        let rendered = toml::to_string(&src).unwrap();
        let reparsed = parse_author_manifest(&rendered).unwrap();
        assert_eq!(reparsed, src);
    }

    #[test]
    fn installed_state_round_trip() {
        let src = InstalledState {
            schema: "1".to_string(),
            name: "hello".to_string(),
            kind: "local".to_string(),
            source: ".".to_string(),
            owner: String::new(),
            repo: String::new(),
            host: String::new(),
            tag: String::new(),
            branch: String::new(),
            commit: "abc123".to_string(),
            pinned: false,
            asset_sha256: String::new(),
            installed_at: "2026-04-17T12:34:56Z".to_string(),
            path: "/tmp/x/flox-hello".to_string(),
        };
        let rendered = render_installed_state(&src).unwrap();
        let reparsed = parse_installed_state(&rendered).unwrap();
        assert_eq!(reparsed, src);
    }
}
