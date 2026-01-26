use crate::{Manifest, Migrated, MigratedTypedOnly, Parsed, TypedOnly, Validated};

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

impl AsTypedOnlyManifest for Parsed {
    fn as_typed_only(&self) -> Manifest<TypedOnly> {
        Manifest {
            inner: TypedOnly {
                parsed: self.clone(),
            },
        }
    }
}
