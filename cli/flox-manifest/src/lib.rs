use std::path::Path;

use toml_edit::DocumentMut;

use crate::parsed::common::VersionKind;
use crate::parsed::v1::ManifestV1;
use crate::parsed::v1_9_0::ManifestV1_9_0;
use crate::raw::get_schema_version_ish;

mod compose;
mod parsed;
mod raw;
mod util;


#[derive(Debug, Clone)]
pub struct Manifest {
    original: InnerOriginal,
    migrated: Option<InnerMigrated>,
}

impl Manifest {
    pub fn migrated(&self) -> Result<ManifestV1_9_0, ManifestError> {
        // FIXME: handle this unwrap before the PR
        Ok(self
            .migrated
            .clone()
            .and_then(|inner| inner.parsed.clone())
            .unwrap())
    }

    fn migrate(&mut self) -> Result<(), ManifestError> {
        // TODO: skip the migration if we have a cached copy
        todo!()
    }

    fn migrated_untyped(&mut self) -> Result<DocumentMut, ManifestError> {
        self.migrate()?;
        Ok(self.migrated.as_ref().ok_or(ManifestError::Other("internal error: migrated manifest was missing".into()))?.raw.clone())
    }

    pub fn read_untyped(p: impl AsRef<Path>) -> Result<Self, ManifestError> {
        let contents = std::fs::read_to_string(p).map_err(ManifestError::IORead)?;
        Self::parse_untyped(contents)
    }

    pub fn parse_untyped(s: impl AsRef<str>) -> Result<Self, ManifestError> {
        let toml = s
            .as_ref()
            .parse::<toml_edit::DocumentMut>()
            .map_err(ManifestError::ParseToml)?;
        Ok(Self {
            original: InnerOriginal {
                raw: toml,
                parsed: None,
            },
            migrated: None,
        })
    }

    pub fn parse(s: impl AsRef<str>) -> Result<Self, ManifestError> {
        let toml = s
            .as_ref()
            .parse::<toml_edit::DocumentMut>()
            .map_err(ManifestError::ParseToml)?;
        match get_schema_version_ish(&toml)? {
            VersionKind::Version(_) => {
                let typed: ManifestV1 =
                    toml_edit::de::from_document(toml.clone()).map_err(ManifestError::Invalid)?;
                Ok(Self {
                    original: InnerOriginal {
                        raw: toml,
                        parsed: Some(Parsed::V1(typed)),
                    },
                    migrated: None,
                })
            },
            VersionKind::SchemaVersion(_) => {
                // TODO: once we add a new schema version, we'll want to match
                //       on the schema version string here instead of assuming
                //       that it's v1.9.0.
                let typed: ManifestV1_9_0 =
                    toml_edit::de::from_document(toml.clone()).map_err(ManifestError::Invalid)?;
                Ok(Self {
                    original: InnerOriginal {
                        raw: toml,
                        parsed: Some(Parsed::V1_9_0(typed)),
                    },
                    migrated: None,
                })
            },
        }
    }

    pub fn read(p: impl AsRef<Path>) -> Result<Self, ManifestError> {
        let contents = std::fs::read_to_string(p).map_err(ManifestError::IORead)?;
        Self::parse_untyped(contents)
    }

    pub fn original_untyped_to_string(&self) -> String {
        self.original.raw.to_string()
    }

    pub fn write_migrated(&self, p: impl AsRef<Path>) -> Result<(), ManifestError> {
        let contents = self.migrated()?
    }
}

// pub trait IsManifest {}

// pub trait<T: IsManifest> ParseManifest {
//     pub fn
// }

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    // ====================================================================== //
    // Parsing manifests
    // ====================================================================== //
    /// We failed to read a manifest from disk.
    #[error("failed to read manifest file: {0}")]
    IORead(#[source] std::io::Error),

    /// We failed to read a manifest from disk.
    #[error("failed to write manifest file: {0}")]
    IOWrite(#[source] std::io::Error),

    /// The provided string failed to parse as valid TOML of any kind.
    #[error("manifest contents were not valid TOML: {0}")]
    ParseToml(#[source] toml_edit::TomlError),

    #[error("manifest had invalid schema version '{0}'")]
    InvalidSchemaVersion(String),

    #[error("manifest 'schema-version' field is missing")]
    MissingSchemaVersion,

    #[error("invalid manifest: {0}")]
    Invalid(#[source] toml_edit::de::Error),

    #[error("failed to serialize manifest: {0}")]
    Serialize(#[source] toml_edit::ser::Error),

    #[error("{0}")]
    Other(String),

    // ====================================================================== //
    // Looking up packages and package groups
    // ====================================================================== //
    #[error("no package or group named '{0}' in the manifest")]
    PkgOrGroupNotFound(String),

    #[error("no package named '{0}' in the manifest")]
    PackageNotFound(String),

    #[error(
        "multiple packages match '{0}', please specify an install id from possible matches: {1:?}"
    )]
    MultiplePackagesMatch(String, Vec<String>),

    // ====================================================================== //
    // Everything else
    // ====================================================================== //
    #[error("not a valid activation mode")]
    ActivateModeInvalid,

    #[error("outputs '{0:?}' don't exists for package {1}")]
    InvalidOutputs(Vec<String>, String),

    #[error("{0}")]
    InvalidServiceConfig(String),
}

#[derive(Debug, Clone)]
struct InnerOriginal {
    raw: toml_edit::DocumentMut,
    parsed: Option<Parsed>,
}

#[derive(Debug, Clone, PartialEq)]
enum Parsed {
    V1(ManifestV1),
    V1_9_0(ManifestV1_9_0),
}

// This is different from `InnerOriginal` in that the parsed type can _only_
// be the latest manifest version as opposed to _any_ manifest version.
#[derive(Debug, Clone)]
struct InnerMigrated {
    raw: toml_edit::DocumentMut,
    parsed: Option<ManifestV1_9_0>,
}
