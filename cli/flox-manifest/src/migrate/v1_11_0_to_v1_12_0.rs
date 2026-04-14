use crate::migrate::MigrationError;
use crate::parsed::v1_11_0::ManifestV1_11_0;
use crate::parsed::v1_12_0::{ManifestV1_12_0, Services};

/// Migrate a v1.11.0 manifest to a v1.12.0 manifest.
///
/// This is a lossless migration: V1_12_0 adds an optional `auto-start` field
/// to the `[services]` section. All V1_11_0 manifests are valid V1_12_0
/// manifests with `auto_start: None`.
pub(crate) fn migrate_manifest_v1_11_0_to_v1_12_0(
    manifest: ManifestV1_11_0,
) -> Result<ManifestV1_12_0, MigrationError> {
    Ok(ManifestV1_12_0 {
        schema_version: "1.12.0".to_string(),
        minimum_cli_version: manifest.minimum_cli_version,
        install: manifest.install,
        vars: manifest.vars,
        hook: manifest.hook,
        profile: manifest.profile,
        options: manifest.options,
        services: Services {
            auto_start: None,
            service_map: manifest.services,
        },
        build: manifest.build,
        containerize: manifest.containerize,
        include: manifest.include,
    })
}
