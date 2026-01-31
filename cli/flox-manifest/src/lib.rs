use std::path::Path;
use std::str::FromStr;

#[cfg(any(test, feature = "tests"))]
use proptest::prelude::*;
use schemars::{JsonSchema, schema_for};
use serde::de::IntoDeserializer;
use serde::{Deserialize, Serialize};
use toml_edit::DocumentMut;

use crate::lockfile::{Lockfile, LockfileError};
use crate::parsed::common::KnownSchemaVersion;
use crate::parsed::latest::ManifestLatest;
use crate::parsed::v1::ManifestV1;
use crate::parsed::v1_9_0::ManifestV1_9_0;
use crate::raw::{get_schema_version_kind, get_toml_schema_version_kind};

pub mod compose;
pub mod lockfile;
pub mod parsed;
pub mod raw;
pub mod util;

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
    // Everything else
    // =========================================================================
    #[error("not a valid activation mode")]
    ActivateModeInvalid,

    #[error("outputs '{0:?}' don't exists for package {1}")]
    InvalidOutputs(Vec<String>, String),

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
    original_raw: DocumentMut,
    #[serde(skip)]
    original_parsed: Parsed,
    #[serde(skip)]
    migrated_raw: DocumentMut,
    #[serde(flatten)]
    migrated_parsed: ManifestLatest,
    #[serde(skip)]
    lockfile: Lockfile,
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
#[derive(Debug, Clone, PartialEq)]
pub struct MigratedTypedOnly {
    original_parsed: Parsed,
    migrated_parsed: ManifestLatest,
}

/// A validated, typed manifest with a known schema.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
#[serde(untagged)]
enum Parsed {
    V1(ManifestV1),
    V1_9_0(ManifestV1_9_0),
}

impl Parsed {
    /// A helper function for creating a [`Parsed`] from whatever the latest
    /// manifest schema version happens to be.
    pub(crate) fn from_latest(manifest: ManifestLatest) -> Self {
        Self::V1_9_0(manifest)
    }

    /// Returns the known schema version of the contained manifest.
    // This is pub(crate) because this type is kind of an implementation detail,
    // and you should probably access schema version information from a
    // `Manifest` instead.
    pub(crate) fn schema_version(&self) -> KnownSchemaVersion {
        match self {
            Parsed::V1(_) => KnownSchemaVersion::V1,
            Parsed::V1_9_0(_) => KnownSchemaVersion::V1_9_0,
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
impl ManifestState for TypedOnly {}
impl ManifestState for MigratedTypedOnly {}

#[derive(Debug, Clone)]
pub struct Manifest<S = Init> {
    inner: S,
}

// =============================================================================
// Implementation
// =============================================================================

/// A trait implemented by states that have access to a typed, migrated manifest.
///
/// This is helpful in cases where you don't care where the manifest came from
/// (migrating from on-disk manifest vs. from a lockfile).
pub trait MigratedManifest {
    fn migrated_manifest(&self) -> &ManifestLatest;
}

impl MigratedManifest for Manifest<Migrated> {
    fn migrated_manifest(&self) -> &ManifestLatest {
        &self.inner.migrated_parsed
    }
}

impl MigratedManifest for Manifest<MigratedTypedOnly> {
    fn migrated_manifest(&self) -> &ManifestLatest {
        &self.inner.migrated_parsed
    }
}

/// A trait for retrieving a `TypedOnly` manifest from states that possess one.
///
/// For states that have not yet been migrated, this will return the "original"
/// manifest. For states that _have_ been migrated, this SHOULD return the
/// migrated manifest.
pub trait AsTypedOnlyManifest {
    fn as_typed_only(&self) -> Manifest<TypedOnly>;
}

impl AsTypedOnlyManifest for Manifest<Validated> {
    fn as_typed_only(&self) -> Manifest<TypedOnly> {
        Manifest {
            inner: TypedOnly {
                parsed: self.inner.parsed.clone(),
            },
        }
    }
}

impl AsTypedOnlyManifest for Manifest<MigratedTypedOnly> {
    fn as_typed_only(&self) -> Manifest<TypedOnly> {
        Manifest {
            inner: TypedOnly {
                parsed: Parsed::from_latest(self.inner.migrated_parsed.clone()),
            },
        }
    }
}

impl AsTypedOnlyManifest for Manifest<Migrated> {
    fn as_typed_only(&self) -> Manifest<TypedOnly> {
        Manifest {
            inner: TypedOnly {
                parsed: Parsed::from_latest(self.inner.migrated_parsed.clone()),
            },
        }
    }
}

pub trait CommonFields {
    fn vars(&self) -> &parsed::common::Vars;
    fn hook(&self) -> Option<&parsed::common::Hook>;
    fn profile(&self) -> Option<&parsed::common::Profile>;
    fn services(&self) -> &parsed::common::Services;
    fn include(&self) -> &parsed::common::Include;
    fn build(&self) -> &parsed::common::Build;
    fn containerize(&self) -> Option<&parsed::common::Containerize>;
    fn options(&self) -> &parsed::common::Options;

    fn vars_mut(&mut self) -> &mut parsed::common::Vars;
    fn hook_mut(&mut self) -> Option<&mut parsed::common::Hook>;
    fn profile_mut(&mut self) -> Option<&mut parsed::common::Profile>;
    fn services_mut(&mut self) -> &mut parsed::common::Services;
    fn include_mut(&mut self) -> &mut parsed::common::Include;
    fn build_mut(&mut self) -> &mut parsed::common::Build;
    fn containerize_mut(&mut self) -> Option<&mut parsed::common::Containerize>;
    fn options_mut(&mut self) -> &mut parsed::common::Options;
}

impl CommonFields for Parsed {
    fn vars(&self) -> &parsed::common::Vars {
        match self {
            Parsed::V1(inner) => inner.vars(),
            Parsed::V1_9_0(inner) => inner.vars(),
        }
    }

    fn hook(&self) -> Option<&parsed::common::Hook> {
        match self {
            Parsed::V1(inner) => inner.hook(),
            Parsed::V1_9_0(inner) => inner.hook(),
        }
    }

    fn profile(&self) -> Option<&parsed::common::Profile> {
        match self {
            Parsed::V1(inner) => inner.profile(),
            Parsed::V1_9_0(inner) => inner.profile(),
        }
    }

    fn services(&self) -> &parsed::common::Services {
        match self {
            Parsed::V1(inner) => inner.services(),
            Parsed::V1_9_0(inner) => inner.services(),
        }
    }

    fn include(&self) -> &parsed::common::Include {
        match self {
            Parsed::V1(inner) => inner.include(),
            Parsed::V1_9_0(inner) => inner.include(),
        }
    }

    fn build(&self) -> &parsed::common::Build {
        match self {
            Parsed::V1(inner) => inner.build(),
            Parsed::V1_9_0(inner) => inner.build(),
        }
    }

    fn containerize(&self) -> Option<&parsed::common::Containerize> {
        match self {
            Parsed::V1(inner) => inner.containerize(),
            Parsed::V1_9_0(inner) => inner.containerize(),
        }
    }

    fn options(&self) -> &parsed::common::Options {
        match self {
            Parsed::V1(inner) => inner.options(),
            Parsed::V1_9_0(inner) => inner.options(),
        }
    }

    fn vars_mut(&mut self) -> &mut parsed::common::Vars {
        match self {
            Parsed::V1(inner) => inner.vars_mut(),
            Parsed::V1_9_0(inner) => inner.vars_mut(),
        }
    }

    fn hook_mut(&mut self) -> Option<&mut parsed::common::Hook> {
        match self {
            Parsed::V1(inner) => inner.hook_mut(),
            Parsed::V1_9_0(inner) => inner.hook_mut(),
        }
    }

    fn profile_mut(&mut self) -> Option<&mut parsed::common::Profile> {
        match self {
            Parsed::V1(inner) => inner.profile_mut(),
            Parsed::V1_9_0(inner) => inner.profile_mut(),
        }
    }

    fn services_mut(&mut self) -> &mut parsed::common::Services {
        match self {
            Parsed::V1(inner) => inner.services_mut(),
            Parsed::V1_9_0(inner) => inner.services_mut(),
        }
    }

    fn include_mut(&mut self) -> &mut parsed::common::Include {
        match self {
            Parsed::V1(inner) => inner.include_mut(),
            Parsed::V1_9_0(inner) => inner.include_mut(),
        }
    }

    fn build_mut(&mut self) -> &mut parsed::common::Build {
        match self {
            Parsed::V1(inner) => inner.build_mut(),
            Parsed::V1_9_0(inner) => inner.build_mut(),
        }
    }

    fn containerize_mut(&mut self) -> Option<&mut parsed::common::Containerize> {
        match self {
            Parsed::V1(inner) => inner.containerize_mut(),
            Parsed::V1_9_0(inner) => inner.containerize_mut(),
        }
    }

    fn options_mut(&mut self) -> &mut parsed::common::Options {
        match self {
            Parsed::V1(inner) => inner.options_mut(),
            Parsed::V1_9_0(inner) => inner.options_mut(),
        }
    }
}

impl CommonFields for Manifest<TypedOnly> {
    fn vars(&self) -> &parsed::common::Vars {
        self.inner.parsed.vars()
    }

    fn hook(&self) -> Option<&parsed::common::Hook> {
        self.inner.parsed.hook()
    }

    fn profile(&self) -> Option<&parsed::common::Profile> {
        self.inner.parsed.profile()
    }

    fn services(&self) -> &parsed::common::Services {
        self.inner.parsed.services()
    }

    fn include(&self) -> &parsed::common::Include {
        self.inner.parsed.include()
    }

    fn build(&self) -> &parsed::common::Build {
        self.inner.parsed.build()
    }

    fn containerize(&self) -> Option<&parsed::common::Containerize> {
        self.inner.parsed.containerize()
    }

    fn options(&self) -> &parsed::common::Options {
        self.inner.parsed.options()
    }

    fn vars_mut(&mut self) -> &mut parsed::common::Vars {
        self.inner.parsed.vars_mut()
    }

    fn hook_mut(&mut self) -> Option<&mut parsed::common::Hook> {
        self.inner.parsed.hook_mut()
    }

    fn profile_mut(&mut self) -> Option<&mut parsed::common::Profile> {
        self.inner.parsed.profile_mut()
    }

    fn services_mut(&mut self) -> &mut parsed::common::Services {
        self.inner.parsed.services_mut()
    }

    fn include_mut(&mut self) -> &mut parsed::common::Include {
        self.inner.parsed.include_mut()
    }

    fn build_mut(&mut self) -> &mut parsed::common::Build {
        self.inner.parsed.build_mut()
    }

    fn containerize_mut(&mut self) -> Option<&mut parsed::common::Containerize> {
        self.inner.parsed.containerize_mut()
    }

    fn options_mut(&mut self) -> &mut parsed::common::Options {
        self.inner.parsed.options_mut()
    }
}

impl CommonFields for Manifest<MigratedTypedOnly> {
    fn vars(&self) -> &parsed::common::Vars {
        self.inner.migrated_parsed.vars()
    }

    fn hook(&self) -> Option<&parsed::common::Hook> {
        self.inner.migrated_parsed.hook()
    }

    fn profile(&self) -> Option<&parsed::common::Profile> {
        self.inner.migrated_parsed.profile()
    }

    fn services(&self) -> &parsed::common::Services {
        self.inner.migrated_parsed.services()
    }

    fn include(&self) -> &parsed::common::Include {
        self.inner.migrated_parsed.include()
    }

    fn build(&self) -> &parsed::common::Build {
        self.inner.migrated_parsed.build()
    }

    fn containerize(&self) -> Option<&parsed::common::Containerize> {
        self.inner.migrated_parsed.containerize()
    }

    fn options(&self) -> &parsed::common::Options {
        self.inner.migrated_parsed.options()
    }

    fn vars_mut(&mut self) -> &mut parsed::common::Vars {
        self.inner.migrated_parsed.vars_mut()
    }

    fn hook_mut(&mut self) -> Option<&mut parsed::common::Hook> {
        self.inner.migrated_parsed.hook_mut()
    }

    fn profile_mut(&mut self) -> Option<&mut parsed::common::Profile> {
        self.inner.migrated_parsed.profile_mut()
    }

    fn services_mut(&mut self) -> &mut parsed::common::Services {
        self.inner.migrated_parsed.services_mut()
    }

    fn include_mut(&mut self) -> &mut parsed::common::Include {
        self.inner.migrated_parsed.include_mut()
    }

    fn build_mut(&mut self) -> &mut parsed::common::Build {
        self.inner.migrated_parsed.build_mut()
    }

    fn containerize_mut(&mut self) -> Option<&mut parsed::common::Containerize> {
        self.inner.migrated_parsed.containerize_mut()
    }

    fn options_mut(&mut self) -> &mut parsed::common::Options {
        self.inner.migrated_parsed.options_mut()
    }
}

impl CommonFields for Manifest<Migrated> {
    fn vars(&self) -> &parsed::common::Vars {
        self.inner.migrated_parsed.vars()
    }

    fn hook(&self) -> Option<&parsed::common::Hook> {
        self.inner.migrated_parsed.hook()
    }

    fn profile(&self) -> Option<&parsed::common::Profile> {
        self.inner.migrated_parsed.profile()
    }

    fn services(&self) -> &parsed::common::Services {
        self.inner.migrated_parsed.services()
    }

    fn include(&self) -> &parsed::common::Include {
        self.inner.migrated_parsed.include()
    }

    fn build(&self) -> &parsed::common::Build {
        self.inner.migrated_parsed.build()
    }

    fn containerize(&self) -> Option<&parsed::common::Containerize> {
        self.inner.migrated_parsed.containerize()
    }

    fn options(&self) -> &parsed::common::Options {
        self.inner.migrated_parsed.options()
    }

    fn vars_mut(&mut self) -> &mut parsed::common::Vars {
        self.inner.migrated_parsed.vars_mut()
    }

    fn hook_mut(&mut self) -> Option<&mut parsed::common::Hook> {
        self.inner.migrated_parsed.hook_mut()
    }

    fn profile_mut(&mut self) -> Option<&mut parsed::common::Profile> {
        self.inner.migrated_parsed.profile_mut()
    }

    fn services_mut(&mut self) -> &mut parsed::common::Services {
        self.inner.migrated_parsed.services_mut()
    }

    fn include_mut(&mut self) -> &mut parsed::common::Include {
        self.inner.migrated_parsed.include_mut()
    }

    fn build_mut(&mut self) -> &mut parsed::common::Build {
        self.inner.migrated_parsed.build_mut()
    }

    fn containerize_mut(&mut self) -> Option<&mut parsed::common::Containerize> {
        self.inner.migrated_parsed.containerize_mut()
    }

    fn options_mut(&mut self) -> &mut parsed::common::Options {
        self.inner.migrated_parsed.options_mut()
    }
}

/// A trait for retrieving the schema version from typed manifests.
pub trait SchemaVersion {
    fn get_schema_version(&self) -> KnownSchemaVersion;
}

impl SchemaVersion for Manifest<TypedOnly> {
    fn get_schema_version(&self) -> KnownSchemaVersion {
        self.inner.parsed.schema_version()
    }
}

impl SchemaVersion for Manifest<MigratedTypedOnly> {
    fn get_schema_version(&self) -> KnownSchemaVersion {
        self.inner.migrated_parsed.get_schema_version()
    }
}

impl SchemaVersion for Manifest<Migrated> {
    fn get_schema_version(&self) -> KnownSchemaVersion {
        self.inner.migrated_parsed.get_schema_version()
    }
}

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
        Self::parse_untyped(s)?.validate_toml()
    }

    /// Read the TOML file at the given path, parse it into a typed and validated manifest.
    pub fn read_typed(p: impl AsRef<Path>) -> Result<Manifest<Validated>, ManifestError> {
        Self::read_untyped(p)?.validate_toml()
    }

    /// Use the provided TOML manifest contents and JSON lockfile contents to
    /// parse a manifest and migrate it to the latest schema version.
    pub fn parse_and_migrate(
        manifest_contents: impl AsRef<str>,
        lockfile_contents: impl AsRef<str>,
    ) -> Result<Manifest<Migrated>, ManifestError> {
        let lockfile = Lockfile::from_str(lockfile_contents.as_ref())
            .map_err(|err| ManifestError::Lockfile(Box::new(err)))?;
        Self::parse_untyped(manifest_contents)?
            .validate_toml()?
            .migrate(&lockfile)
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
        Self::parse_and_migrate(manifest_contents, lockfile_contents)
    }
}

impl Manifest<TomlParsed> {
    pub fn validate_toml(&self) -> Result<Manifest<Validated>, ManifestError> {
        let schema_version: KnownSchemaVersion =
            get_schema_version_kind(&self.inner.raw)?.try_into()?;
        let parsed = Manifest::<Init>::parse_with_schema(&self.inner.raw, schema_version)?;
        Ok(Manifest {
            inner: Validated {
                raw: self.inner.raw.clone(),
                parsed,
            },
        })
    }
}

impl Manifest<Validated> {
    pub fn to_deserialized(&self) -> Manifest<TypedOnly> {
        Manifest {
            inner: TypedOnly {
                parsed: self.inner.parsed.clone(),
            },
        }
    }

    pub fn migrate(&self, _lockfile: &Lockfile) -> Result<Manifest<Migrated>, ManifestError> {
        todo!()
    }
}

impl Manifest<TypedOnly> {
    pub fn migrate_deserialized(
        &self,
        _lockfile: &Lockfile,
    ) -> Result<Manifest<MigratedTypedOnly>, ManifestError> {
        todo!()
    }

    /// Bootstrap a [`Manifest<Deserialized>`] from the inner [`ManifestLatest`].
    pub(crate) fn from_latest(manifest: ManifestLatest) -> Self {
        Manifest {
            inner: TypedOnly {
                parsed: Parsed::from_latest(manifest),
            },
        }
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
            KnownSchemaVersion::V1_9_0 => {
                let d = untyped.into_deserializer();
                let manifest = ManifestV1_9_0::deserialize(d)
                    .map_err(|err| serde::de::Error::custom(err.to_string()))?;
                Ok(Manifest {
                    inner: TypedOnly {
                        parsed: Parsed::V1_9_0(manifest),
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

impl Default for Manifest<TypedOnly> {
    fn default() -> Self {
        Manifest {
            inner: TypedOnly {
                parsed: Parsed::V1_9_0(ManifestV1_9_0::default()),
            },
        }
    }
}

impl PartialEq for Manifest<TypedOnly> {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
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
