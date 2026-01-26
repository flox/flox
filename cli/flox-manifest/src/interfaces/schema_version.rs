use crate::parsed::common::KnownSchemaVersion;
use crate::{Manifest, Migrated, MigratedTypedOnly, TypedOnly, Validated};

/// A trait for retrieving the schema version from typed manifests.
pub trait SchemaVersion {
    fn get_schema_version(&self) -> KnownSchemaVersion;
}

impl SchemaVersion for Manifest<Validated> {
    fn get_schema_version(&self) -> KnownSchemaVersion {
        self.inner.parsed.schema_version()
    }
}

impl SchemaVersion for &Manifest<Validated> {
    fn get_schema_version(&self) -> KnownSchemaVersion {
        self.inner.parsed.schema_version()
    }
}

impl SchemaVersion for Manifest<TypedOnly> {
    fn get_schema_version(&self) -> KnownSchemaVersion {
        self.inner.parsed.schema_version()
    }
}

impl SchemaVersion for &Manifest<TypedOnly> {
    fn get_schema_version(&self) -> KnownSchemaVersion {
        self.inner.parsed.schema_version()
    }
}

impl SchemaVersion for Manifest<MigratedTypedOnly> {
    fn get_schema_version(&self) -> KnownSchemaVersion {
        self.inner.migrated_parsed.get_schema_version()
    }
}

impl SchemaVersion for &Manifest<MigratedTypedOnly> {
    fn get_schema_version(&self) -> KnownSchemaVersion {
        self.inner.migrated_parsed.get_schema_version()
    }
}

impl SchemaVersion for Manifest<Migrated> {
    fn get_schema_version(&self) -> KnownSchemaVersion {
        self.inner.migrated_parsed.get_schema_version()
    }
}

impl SchemaVersion for &Manifest<Migrated> {
    fn get_schema_version(&self) -> KnownSchemaVersion {
        self.inner.migrated_parsed.get_schema_version()
    }
}

pub trait OriginalSchemaVersion {
    fn original_schema(&self) -> KnownSchemaVersion;
}

impl OriginalSchemaVersion for Manifest<Migrated> {
    fn original_schema(&self) -> KnownSchemaVersion {
        self.inner.original_parsed.schema_version()
    }
}

impl OriginalSchemaVersion for Manifest<MigratedTypedOnly> {
    fn original_schema(&self) -> KnownSchemaVersion {
        self.inner.original_parsed.schema_version()
    }
}
