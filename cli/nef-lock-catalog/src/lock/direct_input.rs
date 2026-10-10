//! The direct (first-order) catalog inputs a build lock pins, as persisted
//! on disk.

use floxhub_client::{BuildType, LockedGitSource, LockedInputEntry};
use serde::{Deserialize, Serialize};

use super::flakeref::RawNixFlakerefAttrs;

/// A direct catalog input of a build lock: one entry of
/// `direct_catalog_inputs`, keyed by the server's canonical
/// `<catalog>/<attr-path>` form.
///
/// This is the lock's own type rather than the generated wire
/// [LockedInputEntry] so that the on-disk format is owned here, not by the
/// catalog OpenAPI spec, and so that `source` can hold any flakeref
/// attribute set the NEF can fetch — the wire type admits only a locked git
/// source. The field set and serialization mirror the wire type exactly, so
/// a lock written from a lookup response reads back unchanged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DirectInput {
    pub attr_path: Vec<String>,
    pub build_type: BuildType,
    pub catalog: String,
    /// Direct inputs of this input by key, as the server reported them;
    /// `None` when the server did not state them. Carried through so a
    /// publish can hand the server back exactly what it was given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inputs: Option<Vec<String>>,
    pub locked_inputs_hash: String,
    /// The locked source, stored verbatim. See [RawNixFlakerefAttrs] for
    /// the invariant it carries.
    pub source: RawNixFlakerefAttrs,
}

impl DirectInput {
    /// The entry's canonical `<catalog>/<attr-path>` key, for messages.
    pub fn key(&self) -> String {
        format!("{}/{}", self.catalog, self.attr_path.join("."))
    }
}

/// A direct input whose source is not a locked git source, which is the
/// only source the catalog accepts back in a publish.
#[derive(Debug, thiserror::Error)]
#[error("The catalog lock entry for '{key}' is not a locked git source")]
pub struct NotAGitSourceError {
    /// The entry's canonical `<catalog>/<attr-path>` key.
    pub key: String,
    #[source]
    source: serde_json::Error,
}

impl From<LockedInputEntry> for DirectInput {
    fn from(entry: LockedInputEntry) -> Self {
        let LockedInputEntry {
            attr_path,
            build_type,
            catalog,
            inputs,
            locked_inputs_hash,
            source,
        } = entry;
        DirectInput {
            attr_path,
            build_type,
            catalog,
            inputs,
            locked_inputs_hash,
            source: source.into(),
        }
    }
}

/// The wire form of a direct input, for a publish. Fails when the source is
/// not a locked git source (every git field present and `type == "git"` is
/// what the wire type requires).
impl TryFrom<&DirectInput> for LockedInputEntry {
    type Error = NotAGitSourceError;

    fn try_from(input: &DirectInput) -> Result<Self, Self::Error> {
        let source: LockedGitSource = serde_json::from_value(input.source.as_value().clone())
            .map_err(|source| NotAGitSourceError {
                key: input.key(),
                source,
            })?;
        Ok(LockedInputEntry {
            attr_path: input.attr_path.clone(),
            build_type: input.build_type,
            catalog: input.catalog.clone(),
            inputs: input.inputs.clone(),
            locked_inputs_hash: input.locked_inputs_hash.clone(),
            source,
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn wire_entry() -> LockedInputEntry {
        LockedInputEntry {
            attr_path: vec!["python3Packages".to_string(), "boolex".to_string()],
            build_type: BuildType::Nef,
            catalog: "myorg".to_string(),
            inputs: Some(vec!["myorg/dep".to_string()]),
            locked_inputs_hash: "sha256-test".to_string(),
            source: LockedGitSource {
                dir: ".flox".to_string(),
                ref_: "refs/heads/main".to_string(),
                rev: "abc".to_string(),
                type_: "git".to_string(),
                url: "https://example.com/repo".to_string(),
            },
        }
    }

    /// The lock's type serializes exactly as the wire type does, so the
    /// on-disk format is unchanged by owning it here.
    #[test]
    fn serializes_identically_to_the_wire_entry() {
        let wire = wire_entry();
        let input = DirectInput::from(wire.clone());
        assert_eq!(
            serde_json::to_value(&input).unwrap(),
            serde_json::to_value(&wire).unwrap()
        );
    }

    #[test]
    fn git_source_round_trips_to_the_wire_entry() {
        let wire = wire_entry();
        let input = DirectInput::from(wire.clone());
        assert_eq!(LockedInputEntry::try_from(&input).unwrap(), wire);
        assert_eq!(input.key(), "myorg/python3Packages.boolex");
    }

    /// A source the NEF can fetch but the catalog cannot accept back — here
    /// a `path` flakeref — reads into the lock, and is refused by name only
    /// when converted for the wire.
    #[test]
    fn non_git_source_reads_but_does_not_convert_for_the_wire() {
        let input: DirectInput = serde_json::from_value(json!({
            "attr_path": ["hello"],
            "build_type": "nef",
            "catalog": "myorg",
            "locked_inputs_hash": "sha256-test",
            "source": { "type": "path", "path": "/src/hello", "dir": ".flox" },
        }))
        .unwrap();
        assert_eq!(input.inputs, None);

        let err = LockedInputEntry::try_from(&input).expect_err("a path source has no wire form");
        assert_eq!(err.key, "myorg/hello");
    }
}
