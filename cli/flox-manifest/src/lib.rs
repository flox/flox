use crate::parsed::v1::ManifestV1;
use crate::parsed::v1_9_0::ManifestV1_9_0;

mod compose;
mod parsed;
mod raw;

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

    pub fn parse(s: impl AsRef<str>) -> Result<Self, ManifestError> {
        let toml = s
            .as_ref()
            .parse::<toml_edit::DocumentMut>()
            .map_err(ManifestError::ParseToml)?;
        todo!()
    }
}

// pub trait IsManifest {}

// pub trait<T: IsManifest> ParseManifest {
//     pub fn
// }

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ManifestError {
    // ====================================================================== //
    // Parsing manifests
    // ====================================================================== //
    /// The provided string failed to parse as valid TOML of any kind.
    #[error("manifest contents were not valid TOML: {0}")]
    ParseToml(#[source] toml_edit::TomlError),

    #[error("manifest had invalid schema version '{0}'")]
    InvalidSchemaVersion(String),

    #[error("manifest 'schema-version' field is missing")]
    MissingSchemaVersion,

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
