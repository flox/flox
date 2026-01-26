use std::path::Path;

use crate::{Manifest, ManifestError, Migrated, Validated, Writable};

/// An interface for writing manifests, only implemented on manifest states that support
/// directly writing the contained manifest.
///
/// This is only to be implemented on manifest states that contain a manifest with
/// retained formatting, comments, etc. Writing other manifests to disk will remove
/// a user's formatting, and will not follow the same style guidelines that we
/// normally adhere to.
pub trait AsWritableManifest {
    fn as_writable(&self) -> Manifest<Writable>;
}

impl AsWritableManifest for Manifest<Validated> {
    fn as_writable(&self) -> Manifest<Writable> {
        Manifest {
            inner: Writable {
                raw: self.inner.raw.clone(),
            },
        }
    }
}

impl AsWritableManifest for &Manifest<Validated> {
    fn as_writable(&self) -> Manifest<Writable> {
        Manifest {
            inner: Writable {
                raw: self.inner.raw.clone(),
            },
        }
    }
}

impl AsWritableManifest for Manifest<Migrated> {
    fn as_writable(&self) -> Manifest<Writable> {
        Manifest {
            inner: Writable {
                raw: self.inner.migrated_raw.clone(),
            },
        }
    }
}

impl AsWritableManifest for &Manifest<Migrated> {
    fn as_writable(&self) -> Manifest<Writable> {
        Manifest {
            inner: Writable {
                raw: self.inner.migrated_raw.clone(),
            },
        }
    }
}

pub trait WriteManifest {
    fn to_string(&self) -> String;
    fn write_to_file(&self, p: impl AsRef<Path>) -> Result<(), ManifestError>;
}

impl WriteManifest for Manifest<Writable> {
    fn to_string(&self) -> String {
        self.inner.raw.to_string()
    }

    fn write_to_file(&self, p: impl AsRef<Path>) -> Result<(), ManifestError> {
        std::fs::write(p, self.inner.raw.to_string()).map_err(ManifestError::IOWrite)
    }
}
