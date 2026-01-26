use crate::interfaces::{AsLatestSchema, AsTypedOnlyManifest, SchemaVersion};
use crate::lockfile::Lockfile;
use crate::parsed::common::KnownSchemaVersion;
pub use crate::parsed::v1_10_0::{
    AllSentinel,
    Install,
    ManifestPackageDescriptor,
    PackageDescriptorCatalog,
    PackageDescriptorFlake,
    SelectedOutputs,
};
use crate::{Manifest, ManifestError, TypedOnly};
pub type ManifestLatest = crate::parsed::v1_10_0::ManifestV1_10_0;

impl ManifestLatest {
    fn as_original_schema(
        &self,
        original_schema: KnownSchemaVersion,
    ) -> Result<Option<Manifest<TypedOnly>>, ManifestError> {
        let mut untyped = serde_json::to_value(self).map_err(ManifestError::SerializeJson)?;
        if self.get_schema_version() != original_schema {
            match original_schema {
                KnownSchemaVersion::V1 => {
                    let map = untyped
                        .as_object_mut()
                        .expect("all valid manifests should serialize to JSON objects");
                    map.remove("version");
                    map.insert("schema-version".into(), "1.10.0".into());
                },
                KnownSchemaVersion::V1_10_0 => {},
            }
        }

        let maybe_typed = serde_json::from_value::<Manifest<TypedOnly>>(untyped);
        if maybe_typed.is_err() {
            return Ok(None);
        }
        let Ok(typed_original_schema) = maybe_typed else {
            unreachable!("already checked that deserialization succeeded");
        };
        Ok(Some(typed_original_schema))
    }

    pub fn as_maybe_backwards_compatible(
        &self,
        original_schema: KnownSchemaVersion,
        lockfile: Option<&Lockfile>,
    ) -> Result<Manifest<TypedOnly>, ManifestError> {
        let maybe_backwards_compatible = self.as_original_schema(original_schema)?;
        if maybe_backwards_compatible.is_none() {
            // If this was `None` it means we couldn't represent the current
            // manifest in the old schema at all (there could be new fields,
            // syntax, etc). In that case, we *must* migrate.
            return Ok(self.as_typed_only());
        }
        let backwards_compatible =
            maybe_backwards_compatible.expect("just verified that option is some");
        let migrated_again = backwards_compatible.migrate_typed_only(lockfile)?;
        let migrated_again = migrated_again.as_latest_schema();
        if migrated_again == self {
            Ok(backwards_compatible)
        } else {
            Ok(self.as_typed_only())
        }
    }
}
