use crate::parsed::v1::ManifestV1;
use crate::parsed::v1_9_0::ManifestV1_9_0;

mod parsed;
mod raw;

#[derive(Debug, Clone)]
pub struct Manifest {
    original: Inner,
    migrated: Option<Inner>,
}

#[derive(Debug, Clone)]
struct Inner {
    raw: toml_edit::DocumentMut,
    parsed: Option<Parsed>,
}

#[derive(Debug, Clone, PartialEq)]
enum Parsed {
    V1(ManifestV1),
    V1_9_0(ManifestV1_9_0),
}
