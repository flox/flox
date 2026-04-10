use crate::migrate::MigrationError;
use crate::parsed::v1_11_0::ManifestV1_11_0;
use crate::parsed::v1_12_0::{ActivateOptions, ManifestV1_12_0, Options, Services};

/// Migrate a v1.11.0 manifest to a v1.12.0 manifest.
///
/// This is a lossless migration. V1_12_0 adds:
/// - an optional `auto-start` field to the `[services]` section
/// - an optional `add-sbin` field to `[options.activate]`
///
/// All V1_11_0 manifests are valid V1_12_0 manifests with those new fields
/// defaulting to `None`.
pub(crate) fn migrate_manifest_v1_11_0_to_v1_12_0(
    manifest: &ManifestV1_11_0,
) -> Result<ManifestV1_12_0, MigrationError> {
    let old_options = &manifest.options;
    let options = Options {
        systems: old_options.systems.clone(),
        allow: old_options.allow.clone(),
        semver: old_options.semver.clone(),
        cuda_detection: old_options.cuda_detection,
        activate: ActivateOptions {
            mode: old_options.activate.mode.clone(),
            add_sbin: None,
        },
    };
    Ok(ManifestV1_12_0 {
        schema_version: "1.12.0".to_string(),
        minimum_cli_version: manifest.minimum_cli_version.clone(),
        install: manifest.install.clone(),
        vars: manifest.vars.clone(),
        hook: manifest.hook.clone(),
        profile: manifest.profile.clone(),
        options,
        services: Services {
            auto_start: None,
            service_map: manifest.services.clone(),
        },
        build: manifest.build.clone(),
        containerize: manifest.containerize.clone(),
        include: manifest.include.clone(),
    })
}
