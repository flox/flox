use crate::parsed::latest::ManifestLatest;
use crate::{Manifest, Migrated, MigratedTypedOnly};

/// A trait implemented by states that have access to a typed, migrated manifest.
///
/// This is helpful in cases where you don't care where the manifest came from
/// (migrating from on-disk manifest vs. from a lockfile).
pub trait AsLatestSchema {
    fn as_latest_schema(&self) -> &ManifestLatest;
    fn as_latest_schema_mut(&mut self) -> &mut ManifestLatest;
}

impl AsLatestSchema for Manifest<Migrated> {
    fn as_latest_schema(&self) -> &ManifestLatest {
        &self.inner.migrated_parsed
    }

    fn as_latest_schema_mut(&mut self) -> &mut ManifestLatest {
        &mut self.inner.migrated_parsed
    }
}

impl AsLatestSchema for Manifest<MigratedTypedOnly> {
    fn as_latest_schema(&self) -> &ManifestLatest {
        &self.inner.migrated_parsed
    }

    fn as_latest_schema_mut(&mut self) -> &mut ManifestLatest {
        &mut self.inner.migrated_parsed
    }
}
