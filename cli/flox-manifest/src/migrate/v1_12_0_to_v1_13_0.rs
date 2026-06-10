use crate::migrate::MigrationError;
use crate::parsed::v1_12_0::ManifestV1_12_0;
use crate::parsed::v1_13_0::{ManifestV1_13_0, Profile};

/// Migrate a v1.12.0 manifest to a v1.13.0 manifest.
///
/// This is a lossless migration: V1_13_0 adds an optional `deactivate` table
/// to the `[profile]` section for symmetric per-shell deactivation hooks. All
/// V1_12_0 manifests are valid V1_13_0 manifests with `profile.deactivate:
/// None`.
pub(crate) fn migrate_manifest_v1_12_0_to_v1_13_0(
    manifest: ManifestV1_12_0,
) -> Result<ManifestV1_13_0, MigrationError> {
    let profile = manifest.profile.map(|p| Profile {
        common: p.common,
        bash: p.bash,
        zsh: p.zsh,
        fish: p.fish,
        tcsh: p.tcsh,
        deactivate: None,
    });
    Ok(ManifestV1_13_0 {
        schema_version: "1.13.0".to_string(),
        minimum_cli_version: manifest.minimum_cli_version,
        install: manifest.install,
        vars: manifest.vars,
        hook: manifest.hook,
        profile,
        options: manifest.options,
        services: manifest.services,
        build: manifest.build,
        containerize: manifest.containerize,
        include: manifest.include,
    })
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        #[test]
        fn migration_is_lossless_for_any_manifest(manifest in any::<ManifestV1_12_0>()) {
            let migrated = migrate_manifest_v1_12_0_to_v1_13_0(manifest.clone()).unwrap();

            let expected = ManifestV1_13_0 {
                schema_version: "1.13.0".to_string(),
                minimum_cli_version: manifest.minimum_cli_version,
                install: manifest.install,
                vars: manifest.vars,
                hook: manifest.hook,
                profile: manifest.profile.map(|p| Profile {
                    common: p.common,
                    bash: p.bash,
                    zsh: p.zsh,
                    fish: p.fish,
                    tcsh: p.tcsh,
                    deactivate: None,
                }),
                options: manifest.options,
                services: manifest.services,
                build: manifest.build,
                containerize: manifest.containerize,
                include: manifest.include,
            };
            prop_assert_eq!(migrated, expected);
        }
    }
}
