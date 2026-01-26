//! Get a concrete manifest from a [`Manifest`] that internally knows
//! which schema version it's wrapping.
use crate::parsed::v1::ManifestV1;
use crate::parsed::v1_10_0::ManifestV1_10_0;
use crate::{Manifest, Migrated, MigratedTypedOnly, Parsed, TypedOnly, Validated};

/// A trait that allows you to generically extract a concrete inner manifest
/// from a `Manifest` if it contains the specified (via generics) concrete
/// type.
///
/// This uses the `GetInnerManifest` trait bound to restrict the usage of
/// the trait to manifest states (`Manifest<States>`) for which we know
/// we can extract the typed manifest. By implementing `GetInnerManifest<M>`
/// on different `Manifest<State>`s, we statically define which concrete
/// manifests can be extracted from which `State`s.
pub trait InnerManifest {
    fn inner_manifest<M: InnerManifestMarker>(&self) -> Option<&M>
    where
        Self: GetInnerManifest<M>;
}

impl<State> InnerManifest for Manifest<State> {
    fn inner_manifest<M: InnerManifestMarker>(&self) -> Option<&M>
    where
        Self: GetInnerManifest<M>,
    {
        self.get_inner_manifest()
    }
}

/// This is a marker trait to restrict which types can be extracted from
/// a `Manifest` e.g. you can extract a `ManifestV1` because we
/// `impl InnerManifestMarker for ManifestV1`, but you can't extract a
/// `String` (or any other arbitrary type) because there's no implementation
/// for other types.
///
/// In short, we use this trait to mark which types are manifests.
pub trait InnerManifestMarker {}
impl InnerManifestMarker for ManifestV1 {}
impl InnerManifestMarker for ManifestV1_10_0 {}

/// This trait is used to define which concrete manifest types can
/// be extracted from `Manifest<State>` and in which `State`s.
pub trait GetInnerManifest<M> {
    fn get_inner_manifest(&self) -> Option<&M>;
}

impl GetInnerManifest<ManifestV1> for Manifest<Validated> {
    fn get_inner_manifest(&self) -> Option<&ManifestV1> {
        if let Parsed::V1(ref manifest) = self.inner.parsed {
            Some(manifest)
        } else {
            None
        }
    }
}

impl GetInnerManifest<ManifestV1_10_0> for Manifest<Validated> {
    fn get_inner_manifest(&self) -> Option<&ManifestV1_10_0> {
        if let Parsed::V1_10_0(ref manifest) = self.inner.parsed {
            Some(manifest)
        } else {
            None
        }
    }
}

impl GetInnerManifest<ManifestV1> for Manifest<TypedOnly> {
    fn get_inner_manifest(&self) -> Option<&ManifestV1> {
        if let Parsed::V1(ref manifest) = self.inner.parsed {
            Some(manifest)
        } else {
            None
        }
    }
}

impl GetInnerManifest<ManifestV1_10_0> for Manifest<TypedOnly> {
    fn get_inner_manifest(&self) -> Option<&ManifestV1_10_0> {
        if let Parsed::V1_10_0(ref manifest) = self.inner.parsed {
            Some(manifest)
        } else {
            None
        }
    }
}

impl GetInnerManifest<ManifestV1> for Manifest<Migrated> {
    fn get_inner_manifest(&self) -> Option<&ManifestV1> {
        None
    }
}

impl GetInnerManifest<ManifestV1_10_0> for Manifest<Migrated> {
    fn get_inner_manifest(&self) -> Option<&ManifestV1_10_0> {
        Some(&self.inner.migrated_parsed)
    }
}

impl GetInnerManifest<ManifestV1> for Manifest<MigratedTypedOnly> {
    fn get_inner_manifest(&self) -> Option<&ManifestV1> {
        None
    }
}

impl GetInnerManifest<ManifestV1_10_0> for Manifest<MigratedTypedOnly> {
    fn get_inner_manifest(&self) -> Option<&ManifestV1_10_0> {
        Some(&self.inner.migrated_parsed)
    }
}
