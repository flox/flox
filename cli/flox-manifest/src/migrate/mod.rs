use crate::interfaces::AsTypedOnlyManifest;
use crate::lockfile::Lockfile;
use crate::migrate::v1_to_v1_10_0::migrate_manifest_v1_to_v1_10_0;
use crate::parsed::common::KnownSchemaVersion;
use crate::raw::SyncTypedToRaw;
use crate::{Manifest, ManifestError, Migrated, MigratedTypedOnly, Parsed, TypedOnly, Validated};

mod v1_to_v1_10_0;

#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    // This variant is a catch-all for situations where the lockfile and manifest
    // aren't consistent with each other for whatever reason.
    #[error("internal error: {0}")]
    Other(String),
}

pub(crate) fn migrate_with_formatting_data(
    pre_migration_manifest: &Manifest<Validated>,
    lockfile: Option<&Lockfile>,
) -> Result<Manifest<Migrated>, ManifestError> {
    let typed_only = pre_migration_manifest.as_typed_only();
    let migrated_typed_only = migrate_typed_only(&typed_only, lockfile)?;
    let mut migrated = Manifest {
        inner: Migrated {
            original_parsed: pre_migration_manifest.inner.parsed.clone(),
            migrated_raw: pre_migration_manifest.inner.raw.clone(),
            migrated_parsed: migrated_typed_only.inner.migrated_parsed,
            lockfile: lockfile.cloned(),
        },
    };
    migrated.update_toml()?;
    Ok(migrated)
}

pub(crate) fn migrate_typed_only(
    pre_migration_manifest: &Manifest<TypedOnly>,
    lockfile: Option<&Lockfile>,
) -> Result<Manifest<MigratedTypedOnly>, ManifestError> {
    let mut inner = pre_migration_manifest.inner.parsed.clone();
    inner = loop {
        match inner {
            Parsed::V1(manifest_v1) => {
                let migrated = migrate_manifest_v1_to_v1_10_0(&manifest_v1, lockfile)?;
                inner = Parsed::V1_10_0(migrated);
            },
            Parsed::V1_10_0(manifest_v1_10_0) => break Parsed::from_latest(manifest_v1_10_0),
        }
    };
    debug_assert_eq!(inner.schema_version(), KnownSchemaVersion::latest());
    let Parsed::V1_10_0(migrated_manifest) = inner else {
        unreachable!("already checked that manifest was latest schema version")
    };
    let migrated = Manifest {
        inner: MigratedTypedOnly {
            original_parsed: pre_migration_manifest.inner.parsed.clone(),
            migrated_parsed: migrated_manifest,
        },
    };
    Ok(migrated)
}

#[cfg(test)]
mod tests {
    use flox_core::data::CanonicalPath;
    use flox_test_utils::GENERATED_DATA;

    use super::*;
    use crate::interfaces::{AsWritableManifest, SchemaVersion, WriteManifest};

    #[test]
    fn toplevel_migration_method_migrates_to_latest_schema() {
        let manifest_path = GENERATED_DATA.join("envs/krb5_prereqs/manifest.toml");
        let contents = std::fs::read_to_string(manifest_path).unwrap();
        let manifest = Manifest::parse_toml_typed(contents).unwrap();
        let lockfile_path =
            CanonicalPath::new_unchecked(GENERATED_DATA.join("envs/krb5_prereqs/manifest.lock"));
        let lockfile = Lockfile::read_from_file(&lockfile_path).unwrap();

        let migrated = manifest.migrate(Some(&lockfile)).unwrap();
        assert_eq!(migrated.get_schema_version(), KnownSchemaVersion::latest());
    }

    #[test]
    fn toplevel_migration_method_updates_toml() {
        let manifest_path = GENERATED_DATA.join("envs/krb5_prereqs/manifest.toml");
        let contents = std::fs::read_to_string(manifest_path).unwrap();
        let manifest = Manifest::parse_toml_typed(contents).unwrap();
        let lockfile_path =
            CanonicalPath::new_unchecked(GENERATED_DATA.join("envs/krb5_prereqs/manifest.lock"));
        let lockfile = Lockfile::read_from_file(&lockfile_path).unwrap();

        let migrated = manifest.migrate(Some(&lockfile)).unwrap();
        let contents = migrated.as_writable().to_string();
        let remigrated = Manifest::parse_and_migrate(&contents, Some(&lockfile)).unwrap();
        assert_eq!(migrated.as_typed_only(), remigrated.as_typed_only());
    }
}
