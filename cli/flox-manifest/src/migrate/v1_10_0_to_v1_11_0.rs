use crate::migrate::MigrationError;
use crate::parsed::v1_10_0::ManifestV1_10_0;
use crate::parsed::v1_11_0::ManifestV1_11_0;

/// Migrate a v1.10.0 manifest to a v1.11.0 manifest.
///
/// This is a trivial migration that copies all fields and updates the
/// schema version. The two schemas are structurally identical except
/// `minimum-cli-version` is now validated as semver.
pub(crate) fn migrate_manifest_v1_10_0_to_v1_11_0(
    manifest: &ManifestV1_10_0,
) -> Result<ManifestV1_11_0, MigrationError> {
    Ok(ManifestV1_11_0 {
        schema_version: "1.11.0".to_string(),
        minimum_cli_version: manifest
            .minimum_cli_version
            .as_deref()
            .map(semver::Version::parse)
            .transpose()
            .map_err(|e| MigrationError::Other(format!("invalid minimum-cli-version: {e}")))?,
        install: manifest.install.clone(),
        vars: manifest.vars.clone(),
        hook: manifest.hook.clone(),
        profile: manifest.profile.clone(),
        options: manifest.options.clone(),
        services: manifest.services.clone(),
        build: manifest.build.clone(),
        containerize: manifest.containerize.clone(),
        include: manifest.include.clone(),
    })
}
