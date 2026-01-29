use std::path::Path;
use std::str::FromStr;

use schemars::{JsonSchema, schema_for};
use serde::de::IntoDeserializer;
use serde::{Deserialize, Serialize};
use toml_edit::DocumentMut;

use crate::lockfile::{Lockfile, LockfileError};
use crate::parsed::common::KnownSchemaVersion;
use crate::parsed::v1::ManifestV1;
use crate::parsed::v1_9_0::ManifestV1_9_0;
use crate::raw::{get_schema_version_kind, get_toml_schema_version_kind};

pub mod compose;
pub mod lockfile;
pub mod parsed;
pub mod raw;
pub mod util;

// There's a well-defined state machine for loading, parsing, and migrating
// manifests that we need to enforce.
#[derive(Debug, Clone)]
pub struct Init;
#[derive(Debug, Clone)]
pub struct TomlParsed {
    raw: DocumentMut,
}
#[derive(Debug, Clone, Serialize)]
pub struct ManifestParsed {
    #[serde(skip)]
    raw: DocumentMut,
    #[serde(flatten)]
    parsed: Parsed,
}
#[derive(Debug, Clone, Serialize)]
pub struct Migrated {
    #[serde(skip)]
    original_raw: DocumentMut,
    #[serde(skip)]
    original_parsed: Parsed,
    #[serde(skip)]
    migrated_raw: DocumentMut,
    #[serde(flatten)]
    migrated_parsed: ManifestV1_9_0,
    #[serde(skip)]
    lockfile: Lockfile,
}

// In this state we never had access to the raw contents of the manifest,
// so there's no way we could parse into a `DocumentMut`. You'll see this
// when you need to parse a `Manifest` out of a `Lockfile`. You can't properly
// migrate this manifest because we don't have a `DocumentMut` to make edits to.
#[derive(Debug, Clone, Serialize, PartialEq, JsonSchema)]
pub struct Deserialized {
    #[serde(flatten)]
    original_parsed: Parsed,
}

pub trait ManifestState {}
impl ManifestState for Init {}
impl ManifestState for TomlParsed {}
impl ManifestState for ManifestParsed {}
impl ManifestState for Migrated {}
impl ManifestState for Deserialized {}

#[derive(Debug, Clone)]
pub struct Manifest<S = Init> {
    inner: S,
}

impl Manifest<Init> {
    pub fn parse_untyped(s: impl AsRef<str>) -> Result<Manifest<TomlParsed>, ManifestError> {
        let toml = s
            .as_ref()
            .parse::<toml_edit::DocumentMut>()
            .map_err(ManifestError::ParseToml)?;
        Ok(Manifest {
            inner: TomlParsed { raw: toml },
        })
    }

    pub fn read_untyped(p: impl AsRef<Path>) -> Result<Manifest<TomlParsed>, ManifestError> {
        let contents = std::fs::read_to_string(p).map_err(ManifestError::IORead)?;
        Self::parse_untyped(contents)
    }

    pub fn parse_typed(s: impl AsRef<str>) -> Result<Manifest<ManifestParsed>, ManifestError> {
        Self::parse_untyped(s)?.validate_toml()
    }

    pub fn read_typed(p: impl AsRef<Path>) -> Result<Manifest<ManifestParsed>, ManifestError> {
        Self::read_untyped(p)?.validate_toml()
    }

    pub fn parse_and_migrate(
        manifest_contents: impl AsRef<str>,
        lockfile_contents: impl AsRef<str>,
    ) -> Result<Manifest<Migrated>, ManifestError> {
        let lockfile =
            Lockfile::from_str(lockfile_contents.as_ref()).map_err(ManifestError::Lockfile)?;
        Self::parse_untyped(manifest_contents)?
            .validate_toml()?
            .migrate(&lockfile)
    }
}

impl Manifest<TomlParsed> {
    pub fn validate_toml(&self) -> Result<Manifest<ManifestParsed>, ManifestError> {
        let schema_version: KnownSchemaVersion =
            get_schema_version_kind(&self.inner.raw)?.try_into()?;
        let parsed = Manifest::<Init>::parse_with_schema(&self.inner.raw, schema_version)?;
        Ok(Manifest {
            inner: ManifestParsed {
                raw: self.inner.raw.clone(),
                parsed,
            },
        })
    }
}

impl Manifest<ManifestParsed> {
    pub fn to_deserialized(&self) -> Manifest<Deserialized> {
        Manifest {
            inner: Deserialized {
                original_parsed: self.inner.parsed.clone(),
            },
        }
    }

    pub fn migrate(&self, _lockfile: &Lockfile) -> Result<Manifest<Migrated>, ManifestError> {
        todo!()
    }
}

impl<S: ManifestState> Manifest<S> {
    fn parse_with_schema(
        toml: &DocumentMut,
        schema: KnownSchemaVersion,
    ) -> Result<Parsed, ManifestError> {
        match schema {
            KnownSchemaVersion::V1 => {
                let manifest = toml_edit::de::from_document::<ManifestV1>(toml.clone())
                    .map_err(ManifestError::Invalid)?;
                Ok(Parsed::V1(manifest))
            },
            KnownSchemaVersion::V1_9_0 => {
                let manifest = toml_edit::de::from_document::<ManifestV1_9_0>(toml.clone())
                    .map_err(ManifestError::Invalid)?;
                Ok(Parsed::V1_9_0(manifest))
            },
        }
    }
}

impl<'de> Deserialize<'de> for Manifest<Deserialized> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let untyped = toml::Value::deserialize(deserializer)?;
        let version: KnownSchemaVersion = get_toml_schema_version_kind(&untyped)
            .map_err(|err| serde::de::Error::custom(err.to_string()))?
            .try_into()
            .map_err(|err: ManifestError| serde::de::Error::custom(err.to_string()))?;
        match version {
            KnownSchemaVersion::V1 => {
                let d = untyped.into_deserializer();
                let manifest = ManifestV1::deserialize(d)
                    .map_err(|err| serde::de::Error::custom(err.to_string()))?;
                Ok(Manifest {
                    inner: Deserialized {
                        original_parsed: Parsed::V1(manifest),
                    },
                })
            },
            KnownSchemaVersion::V1_9_0 => {
                let d = untyped.into_deserializer();
                let manifest = ManifestV1_9_0::deserialize(d)
                    .map_err(|err| serde::de::Error::custom(err.to_string()))?;
                Ok(Manifest {
                    inner: Deserialized {
                        original_parsed: Parsed::V1_9_0(manifest),
                    },
                })
            },
        }
    }
}

impl Serialize for Manifest<ManifestParsed> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.inner.serialize(serializer)
    }
}

impl Serialize for Manifest<Migrated> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.inner.serialize(serializer)
    }
}

impl Serialize for Manifest<Deserialized> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.inner.serialize(serializer)
    }
}

impl Default for Manifest<Deserialized> {
    fn default() -> Self {
        Manifest {
            inner: Deserialized {
                original_parsed: Parsed::V1_9_0(ManifestV1_9_0::default()),
            },
        }
    }
}

impl PartialEq for Manifest<Deserialized> {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

impl JsonSchema for Manifest<Deserialized> {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Manifest".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schema_for!(Parsed)
    }
}

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

    #[error(transparent)]
    Lockfile(LockfileError),

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

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(untagged)]
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
