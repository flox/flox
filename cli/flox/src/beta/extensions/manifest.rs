//! Author manifest (`flox-extension.toml`) and installed-state record
//! (`state.toml`).
//!
//! Plain `serde` structs with a `schema = "1"` string field. The
//! type-state pattern from `flox-manifest::Manifest<S>` is deliberately
//! not used here: there is one schema version and no migration history.
//!
//! `[extension] name` is what install reads; `description` is recorded
//! for future display.

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
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionMeta {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// `state.toml` — written by `install`, consumed by `list` / `remove` and
/// the `flox <name>` dispatch path.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledState {
    #[serde(default = "default_schema")]
    pub schema: String,
    pub name: String,
    /// The absolute source directory the extension was installed from.
    pub source: String,
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
#[cfg(feature = "beta-tests")]
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
            },
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
            },
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
            source: "/home/u/src/flox-hello".to_string(),
            installed_at: "2026-04-17T12:34:56Z".to_string(),
            path: "/tmp/x/flox-hello".to_string(),
        };
        let rendered = render_installed_state(&src).unwrap();
        let reparsed = parse_installed_state(&rendered).unwrap();
        assert_eq!(reparsed, src);
    }
}
