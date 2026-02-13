//!```text
//!                                  ┌───────────────────────────────────────┐
//! Manifests on this side of the    │    Manifest state machine             │ Manifest states on this side of the graph
//! graph maintain information about │                                       │ don't contain the original, formatted
//! the formatting and comments from │    Init                               │ contents of the manifest, and only contain
//! the user's on-disk manifest.     │     │                                 │ the typed contents of the manifest.
//!                                  │     ▼                                 │
//! This means you can only write a  │    ParsedToml                         │ You'll get these manifests when you read
//! manifest from one of these       │     │                                 │ the contents of a lockfile.
//! states. A manifest serialized    │     ▼                                 │
//! via serde will have a different  │ ┌──Validated───────►TypedOnly         │
//! style, won't have a user's       │ │   │               ▲   │             │
//! comments, and won't have a       │ │   │    ┌──────────┘   │             │
//! user's formatting, which is a    │ │   ▼    │              ▼             │
//! dealbreaker.                     │ │  Migrated───────► MigratedTypedOnly │
//!                                  │ │   │                                 │
//!                                  │ │   ▼                                 │
//!                                  │ └─►Writable                           │
//!                                  └───────────────────────────────────────┘
//! ```

use std::path::Path;
use std::str::FromStr;

#[cfg(any(test, feature = "tests"))]
use proptest::prelude::*;
use schemars::{JsonSchema, schema_for};
use serde::de::IntoDeserializer;
use serde::{Deserialize, Serialize};
use toml_edit::DocumentMut;

use crate::interfaces::{AsTypedOnlyManifest, OriginalSchemaVersion, SchemaVersion};
use crate::lockfile::{Lockfile, LockfileError};
use crate::migrate::{MigrationError, migrate_typed_only, migrate_with_formatting_data};
use crate::parsed::common::KnownSchemaVersion;
use crate::parsed::latest::ManifestLatest;
use crate::parsed::v1::ManifestV1;
use crate::parsed::v1_10_0::ManifestV1_10_0;
use crate::raw::{TomlEditError, get_schema_version_kind, get_toml_schema_version_kind};

pub mod compose;
pub mod interfaces;
pub mod lockfile;
mod migrate;
pub mod parsed;
pub mod raw;
pub mod util;

pub static MANIFEST_FILENAME: &str = "manifest.toml";

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    // =========================================================================
    // Parsing manifests
    // =========================================================================
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

    #[error("failed to serialize manifest to lockfile: {0}")]
    SerializeJson(#[source] serde_json::Error),

    #[error("{0}")]
    Other(String),

    #[error(transparent)]
    Lockfile(Box<LockfileError>),

    // =========================================================================
    // Looking up packages and package groups
    // =========================================================================
    #[error("no package or group named '{0}' in the manifest")]
    PkgOrGroupNotFound(String),

    #[error("no package named '{0}' in the manifest")]
    PackageNotFound(String),

    #[error(
        "multiple packages match '{0}', please specify an install id from possible matches: {1:?}"
    )]
    MultiplePackagesMatch(String, Vec<String>),

    // =========================================================================
    // Install/uninstall errors
    // =========================================================================
    #[error(transparent)]
    TomlEdit(#[from] TomlEditError),

    // =========================================================================
    // Everything else
    // =========================================================================
    #[error("not a valid activation mode")]
    ActivateModeInvalid,

    #[error("outputs '{0:?}' don't exists for package {1}")]
    InvalidOutputs(Vec<String>, String),

    #[error(transparent)]
    Migration(#[from] MigrationError),

    #[error("{0}")]
    InvalidServiceConfig(String),
}

// =============================================================================
// State machine
// =============================================================================

/// The initial state of a manifest (e.g. nothing).
#[derive(Debug, Clone)]
pub struct Init;

/// String contents that have successfully parsed as TOML, but that we don't
/// know is a valid manifest yet.
#[derive(Debug, Clone)]
pub struct TomlParsed {
    raw: DocumentMut,
}

/// A manifest that has successfully been loaded from disk, parsed as valid
/// TOML, and validated as a manifest with a known schema version.
#[derive(Debug, Clone, Serialize)]
pub struct Validated {
    #[serde(skip)]
    raw: DocumentMut,
    #[serde(flatten)]
    parsed: Parsed,
}

/// A manifest that has successfully migrated forwards, but has not been
/// checked as to whether we can preserve the original schema version.
#[derive(Debug, Clone, Serialize)]
pub struct Migrated {
    #[serde(skip)]
    original_parsed: Parsed,
    #[serde(skip)]
    migrated_raw: DocumentMut,
    #[serde(flatten)]
    migrated_parsed: ManifestLatest,
    #[serde(skip)]
    lockfile: Option<Lockfile>,
}

/// A manifest that we've determined which schema to write it as after
/// examining whether it's compatible with the pre-migration schema version.
///
/// This is intended to be the terminal state of the manifest, and only
/// exists for the purpose of writing the TOML manifest to disk.
#[derive(Debug, Clone)]
pub struct Writable {
    raw: DocumentMut,
}

/// A manifest that has been deserialized directly into its typed form rather
/// than going through a format-preserving intermediate step like `DocumentMut`.
///
/// In this state we never had access to the raw contents of the manifest,
/// so there's no way we could parse into a `DocumentMut`. You'll see this
/// when you need to parse a `Manifest` out of a `Lockfile`. You can't properly
/// migrate this manifest because we don't have a `DocumentMut` to make edits to.
#[derive(Debug, Clone, Serialize, PartialEq, JsonSchema)]
pub struct TypedOnly {
    #[serde(flatten)]
    parsed: Parsed,
}

/// A migrated manifest that started out as a deserialized manifest.
///
/// This manifest has been internally migrated, but can never be directly written to
/// disk itself as TOML because it didn't start out as TOML, and therefore has none
/// of the comments or formatting that would typically be present in the user's
/// manifest. In other words, writing this manifest to disk would delete all of
/// a user's formatting and comments.
///
/// That said, you could still update a `DocumentMut` with the _contents_ of this
/// manifest in order to write a manifest to disk.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MigratedTypedOnly {
    #[serde(skip)]
    original_parsed: Parsed,
    #[serde(flatten)]
    migrated_parsed: ManifestLatest,
}

/// A validated, typed manifest with a known schema.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
#[serde(untagged)]
enum Parsed {
    V1(ManifestV1),
    V1_10_0(ManifestV1_10_0),
}

impl Parsed {
    /// A helper function for creating a [`Parsed`] from whatever the latest
    /// manifest schema version happens to be.
    pub(crate) fn from_latest(manifest: ManifestLatest) -> Self {
        Self::V1_10_0(manifest)
    }

    /// Returns the known schema version of the contained manifest.
    // This is pub(crate) because this type is kind of an implementation detail,
    // and you should probably access schema version information from a
    // `Manifest` instead.
    pub(crate) fn schema_version(&self) -> KnownSchemaVersion {
        match self {
            Parsed::V1(_) => KnownSchemaVersion::V1,
            Parsed::V1_10_0(_) => KnownSchemaVersion::V1_10_0,
        }
    }
}

/// Type states for the state machine that represents loading, parsing,
/// validating, and migrating manifests.
pub trait ManifestState {}
impl ManifestState for Init {}
impl ManifestState for TomlParsed {}
impl ManifestState for Validated {}
impl ManifestState for Migrated {}
impl ManifestState for Writable {}
impl ManifestState for TypedOnly {}
impl ManifestState for MigratedTypedOnly {}

#[derive(Debug, Clone)]
pub struct Manifest<S = Init> {
    inner: S,
}

// =============================================================================
// Implementation
// =============================================================================

impl Manifest<Init> {
    /// Parse the given TOML into an untyped manifest.
    pub fn parse_untyped(s: impl AsRef<str>) -> Result<Manifest<TomlParsed>, ManifestError> {
        let toml = s
            .as_ref()
            .parse::<toml_edit::DocumentMut>()
            .map_err(ManifestError::ParseToml)?;
        Ok(Manifest {
            inner: TomlParsed { raw: toml },
        })
    }

    /// Read the TOML file at the given path and parse it into an untyped manifest.
    pub fn read_untyped(p: impl AsRef<Path>) -> Result<Manifest<TomlParsed>, ManifestError> {
        let contents = std::fs::read_to_string(p).map_err(ManifestError::IORead)?;
        Self::parse_untyped(contents)
    }

    /// Parse the given TOML into a typed and validated manifest.
    pub fn parse_typed(s: impl AsRef<str>) -> Result<Manifest<Validated>, ManifestError> {
        Self::parse_untyped(s)?.validate()
    }

    /// Read the TOML file at the given path, parse it into a typed and validated manifest.
    pub fn read_typed(p: impl AsRef<Path>) -> Result<Manifest<Validated>, ManifestError> {
        Self::read_untyped(p)?.validate()
    }

    /// Use the provided TOML manifest contents and JSON lockfile contents to
    /// parse a manifest and migrate it to the latest schema version.
    pub fn parse_and_migrate(
        manifest_contents: impl AsRef<str>,
        maybe_lockfile: Option<&Lockfile>,
    ) -> Result<Manifest<Migrated>, ManifestError> {
        Self::parse_untyped(manifest_contents)?
            .validate()?
            .migrate(maybe_lockfile)
    }

    /// Read the TOML manifest and JSON lockfile at the provided paths, then
    /// parse a manifest and migrate it to the latest schema version.
    pub fn read_and_migrate(
        manifest_path: impl AsRef<Path>,
        lockfile_path: impl AsRef<Path>,
    ) -> Result<Manifest<Migrated>, ManifestError> {
        let manifest_contents =
            std::fs::read_to_string(manifest_path).map_err(ManifestError::IORead)?;
        let lockfile_contents = std::fs::read_to_string(lockfile_path)
            .map_err(|err| ManifestError::Lockfile(Box::new(LockfileError::IORead(err))))?;
        let lockfile = Lockfile::from_str(lockfile_contents.as_ref())
            .map_err(|err| ManifestError::Lockfile(Box::new(err)))?;
        Self::parse_and_migrate(manifest_contents, Some(&lockfile))
    }
}

impl Manifest<TomlParsed> {
    pub fn validate(&self) -> Result<Manifest<Validated>, ManifestError> {
        Self::validate_toml(&self.inner.raw)
    }

    pub fn validate_toml(doc: &DocumentMut) -> Result<Manifest<Validated>, ManifestError> {
        let schema_version: KnownSchemaVersion = get_schema_version_kind(doc)?.try_into()?;
        let parsed = Manifest::<Init>::parse_with_schema(doc, schema_version)?;
        Ok(Manifest {
            inner: Validated {
                raw: doc.clone(),
                parsed,
            },
        })
    }
}

impl Manifest<Validated> {
    /// Migrate a manifest with an optional lockfile.
    ///
    /// In some cases the lockfile simply isn't present and you stil must migrate.
    /// In those cases and in cases where a lockfile is present but stale (i.e. incomplete),
    /// the migration process will proceed in full for packages for which we have package data,
    /// but for packages that are missing package data we can only set `outputs = "all"` and
    /// hope for the best.
    pub fn migrate(
        &self,
        lockfile: Option<&Lockfile>,
    ) -> Result<Manifest<Migrated>, ManifestError> {
        migrate_with_formatting_data(self, lockfile)
    }
}

impl Manifest<Migrated> {
    /// Returns the lockfile that was used during the migration.
    pub fn pre_migration_lockfile(&self) -> Option<&Lockfile> {
        self.inner.lockfile.as_ref()
    }

    /// Returns the pre-migration typed manifest.
    pub fn pre_migration_manifest(&self) -> Manifest<TypedOnly> {
        self.inner.original_parsed.as_typed_only()
    }

    /// Returns a `Manifest<MigratedTypedOnly>` copy of the manifest.
    ///
    /// This is mostly useful for locking, where you need a `ManifestLatest`,
    /// but you may not yet have a lockfile with which to migrate an older
    /// schema version. A pathway exists for migrating without a lockfile,
    /// which breaks the need
    pub fn as_migrated_typed_only(&self) -> Manifest<MigratedTypedOnly> {
        Manifest {
            inner: MigratedTypedOnly {
                original_parsed: self.inner.original_parsed.clone(),
                migrated_parsed: self.inner.migrated_parsed.clone(),
            },
        }
    }

    /// Returns whether the migrated manifest can be written in
    /// the original schema.
    pub fn is_backwards_compatible(&self) -> Result<bool, ManifestError> {
        let matches_original_schema = self
            .inner
            .migrated_parsed
            .as_maybe_backwards_compatible(self.original_schema(), self.pre_migration_lockfile())?
            .get_schema_version()
            == self.original_schema();
        Ok(matches_original_schema)
    }
}

impl Manifest<TypedOnly> {
    pub fn migrate_typed_only(
        &self,
        lockfile: Option<&Lockfile>,
    ) -> Result<Manifest<MigratedTypedOnly>, ManifestError> {
        migrate_typed_only(self, lockfile)
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
            KnownSchemaVersion::V1_10_0 => {
                let manifest = toml_edit::de::from_document::<ManifestV1_10_0>(toml.clone())
                    .map_err(ManifestError::Invalid)?;
                Ok(Parsed::V1_10_0(manifest))
            },
        }
    }
}

#[cfg(any(test, feature = "tests"))]
impl Arbitrary for Manifest<TypedOnly> {
    type Parameters = ();
    type Strategy = BoxedStrategy<Self>;

    fn arbitrary_with(_: Self::Parameters) -> Self::Strategy {
        any::<Parsed>()
            .prop_map(|parsed| Manifest {
                inner: TypedOnly { parsed },
            })
            .boxed()
    }
}

// =============================================================================
// (De)serialization and JSON schema
// =============================================================================

impl<'de> Deserialize<'de> for Manifest<TypedOnly> {
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
                    inner: TypedOnly {
                        parsed: Parsed::V1(manifest),
                    },
                })
            },
            KnownSchemaVersion::V1_10_0 => {
                let d = untyped.into_deserializer();
                let manifest = ManifestV1_10_0::deserialize(d)
                    .map_err(|err| serde::de::Error::custom(err.to_string()))?;
                Ok(Manifest {
                    inner: TypedOnly {
                        parsed: Parsed::V1_10_0(manifest),
                    },
                })
            },
        }
    }
}

impl Serialize for Manifest<Validated> {
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

impl Serialize for Manifest<TypedOnly> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.inner.serialize(serializer)
    }
}

impl Serialize for Manifest<MigratedTypedOnly> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.inner.serialize(serializer)
    }
}

impl Default for Manifest<TypedOnly> {
    fn default() -> Self {
        let manifest = ManifestLatest::default();
        Manifest {
            inner: TypedOnly {
                parsed: Parsed::from_latest(manifest),
            },
        }
    }
}

impl PartialEq for Manifest<TypedOnly> {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

impl PartialEq for Manifest<Migrated> {
    fn eq(&self, other: &Self) -> bool {
        let raw_manifests_match =
            self.inner.migrated_raw.to_string() == other.inner.migrated_raw.to_string();
        let typed_manifests_match = self.inner.migrated_parsed == other.inner.migrated_parsed;
        raw_manifests_match && typed_manifests_match
    }
}

impl JsonSchema for Manifest<TypedOnly> {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Manifest".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schema_for!(Parsed)
    }
}

#[cfg(any(test, feature = "tests"))]
pub mod test_helpers {
    use indoc::formatdoc;

    use crate::parsed::common::KnownSchemaVersion;

    /// Prepends a `schema-version = "..."` string with the latest schema
    /// version to the provided manifest body.
    pub fn with_latest_schema(body: impl AsRef<str>) -> String {
        with_schema(KnownSchemaVersion::latest(), body)
    }

    /// Prepends a `schema-version = "..."` string with the specified schema
    /// version to the provided manifest body.
    pub fn with_schema(schema: KnownSchemaVersion, body: impl AsRef<str>) -> String {
        let schema_str = match schema {
            KnownSchemaVersion::V1 => "version = 1".into(),
            _ => format!("schema-version = \"{schema}\""),
        };
        formatdoc! {r#"
            {}

            {}
        "#,  schema_str, body.as_ref()}
    }
}
