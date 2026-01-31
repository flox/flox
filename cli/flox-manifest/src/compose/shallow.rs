use std::collections::BTreeMap;

use tracing::{debug, instrument, trace};

use super::{
    KeyPath,
    ManifestMergeTrait,
    MergeError,
    Warning,
    append_optional_strings,
    deep_merge_optional_containerize_config,
    map_union,
    shallow_merge_options,
};
use crate::parsed::common::{
    ActivateOptions,
    Allows,
    Build,
    Containerize,
    Hook,
    Include,
    KnownSchemaVersion,
    Options,
    Profile,
    SemverOptions,
    Services,
    Vars,
    VersionedContainer,
};
use crate::parsed::v1::{ManifestV1, ManifestVersion};
use crate::parsed::v1_9_0::ManifestV1_9_0;
use crate::parsed::{self, Inner};
use crate::{CommonFields, Manifest, Parsed, SchemaVersion, TypedOnly};

/// Merges two manifests by applying `manifest2` on top of `manifest1` and
/// overwriting any conflicts for keys within the top-level of each `ManifestV1`
/// field, with the exception of `profile` and `hooks`.
#[derive(Clone, Debug)]
pub(crate) struct ShallowMerger;

impl ShallowMerger {
    #[instrument(skip_all)]
    fn merge_version(
        _low_priority: KnownSchemaVersion,
        high_priority: KnownSchemaVersion,
    ) -> Result<KnownSchemaVersion, MergeError> {
        // To be consistent with other "composing manfiest wins" behaviors,
        // the higher priority manifest determines the manifest version
        // and therefore 'outputs' behavior.
        Ok(high_priority)
    }

    #[instrument(skip_all)]
    fn merge_schema_version(
        _low_priority: String,
        high_priority: String,
    ) -> Result<String, MergeError> {
        // To be consistent with other "composing manfiest wins" behaviors,
        // the higher priority manifest determines the manifest schema version.
        Ok(high_priority)
    }

    #[instrument(skip_all)]
    fn merge_install(
        low_priority: VersionedContainer<&parsed::v1::Install, &parsed::v1_9_0::Install>,
        high_priority: VersionedContainer<&parsed::v1::Install, &parsed::v1_9_0::Install>,
    ) -> Result<
        (
            VersionedContainer<parsed::v1::Install, parsed::v1_9_0::Install>,
            Vec<Warning>,
        ),
        MergeError,
    > {
        // Implementation strategy:
        // - Determine schema version of high_priority
        // - Convert all package descriptors to untyped TOML values
        // - Make maps of the untyped TOML values
        // - Merge the untyped maps
        // - Convert back to typed map
        let untyped_low = match low_priority {
            VersionedContainer::V1(install) => install
                .inner()
                .iter()
                .map(|(key, value)| {
                    toml::Value::try_from(value)
                        .map_err(|_| {
                            MergeError::InternalError(
                                "failed to serialize package descriptor".into(),
                            )
                        })
                        .map(|toml_value| (key.clone(), toml_value))
                })
                .collect::<Result<BTreeMap<String, toml::Value>, MergeError>>()?,
            VersionedContainer::V1_9_0(install) => install
                .inner()
                .iter()
                .map(|(key, value)| {
                    toml::Value::try_from(value)
                        .map_err(|_| {
                            MergeError::InternalError(
                                "failed to serialize package descriptor".into(),
                            )
                        })
                        .map(|toml_value| (key.clone(), toml_value))
                })
                .collect::<Result<BTreeMap<String, toml::Value>, MergeError>>()?,
        };

        let untyped_high = match high_priority {
            VersionedContainer::V1(install) => install
                .inner()
                .iter()
                .map(|(key, value)| {
                    toml::Value::try_from(value)
                        .map_err(|_| {
                            MergeError::InternalError(
                                "failed to serialize package descriptor".into(),
                            )
                        })
                        .map(|toml_value| (key.clone(), toml_value))
                })
                .collect::<Result<BTreeMap<String, toml::Value>, MergeError>>()?,
            VersionedContainer::V1_9_0(install) => install
                .inner()
                .iter()
                .map(|(key, value)| {
                    toml::Value::try_from(value)
                        .map_err(|_| {
                            MergeError::InternalError(
                                "failed to serialize package descriptor".into(),
                            )
                        })
                        .map(|toml_value| (key.clone(), toml_value))
                })
                .collect::<Result<BTreeMap<String, toml::Value>, MergeError>>()?,
        };

        let (merged, warnings) =
            map_union(KeyPath::from_iter(["install"]), &untyped_low, &untyped_high);

        let merged_container = match high_priority.get_schema_version() {
            KnownSchemaVersion::V1 => {
                let map = merged
                    .into_iter()
                    .map(|(key, value)| {
                        value.try_into().map(|typed| (key, typed)).map_err(|_| {
                            MergeError::InternalError(
                                "failed to deserialize package descriptor".into(),
                            )
                        })
                    })
                    .collect::<Result<
                        BTreeMap<String, parsed::v1::package_descriptor::ManifestPackageDescriptor>,
                        MergeError,
                    >>()?;
                VersionedContainer::V1(parsed::v1::Install(map))
            },
            KnownSchemaVersion::V1_9_0 => {
                let map = merged
                    .into_iter()
                    .map(|(key, value)| {
                        value.try_into().map(|typed| (key, typed)).map_err(|_| {
                            MergeError::InternalError(
                                "failed to deserialize package descriptor".into(),
                            )
                        })
                    })
                    .collect::<Result<
                        BTreeMap<
                            String,
                            parsed::v1_9_0::package_descriptor::ManifestPackageDescriptor,
                        >,
                        MergeError,
                    >>()?;
                VersionedContainer::V1_9_0(parsed::v1_9_0::Install(map))
            },
        };

        Ok((merged_container, warnings))
    }

    /// Keys in `manifest2` overwrite keys in `manifest1`.
    #[instrument(skip_all)]
    fn merge_vars(
        low_priority: &Vars,
        high_priority: &Vars,
    ) -> Result<(Vars, Vec<Warning>), MergeError> {
        let (merged, warnings) = map_union(
            KeyPath::from_iter(["vars"]),
            low_priority.inner(),
            high_priority.inner(),
        );
        Ok((Vars(merged), warnings))
    }

    #[instrument(skip_all)]
    fn merge_hook(
        low_priority: Option<&Hook>,
        high_priority: Option<&Hook>,
    ) -> Result<Option<Hook>, MergeError> {
        match (low_priority, high_priority) {
            (None, None) => Ok(None),
            (Some(low_priority), None) => Ok(Some(low_priority.clone())),
            (None, Some(high_priority)) => Ok(Some(high_priority.clone())),
            (Some(low_priority), Some(high_priority)) => Ok(Some(Hook {
                on_activate: append_optional_strings(
                    low_priority.on_activate.as_ref(),
                    high_priority.on_activate.as_ref(),
                ),
            })),
        }
    }

    #[instrument(skip_all)]
    fn merge_profile(
        low_priority: Option<&Profile>,
        high_priority: Option<&Profile>,
    ) -> Result<Option<Profile>, MergeError> {
        match (low_priority, high_priority) {
            (None, None) => Ok(None),
            (Some(low_priority), None) => Ok(Some(low_priority.clone())),
            (None, Some(high_priority)) => Ok(Some(high_priority.clone())),
            (Some(low_priority), Some(high_priority)) => {
                let common = append_optional_strings(
                    low_priority.common.as_ref(),
                    high_priority.common.as_ref(),
                );
                let bash = append_optional_strings(
                    low_priority.bash.as_ref(),
                    high_priority.bash.as_ref(),
                );
                let zsh =
                    append_optional_strings(low_priority.zsh.as_ref(), high_priority.zsh.as_ref());
                let tcsh = append_optional_strings(
                    low_priority.tcsh.as_ref(),
                    high_priority.tcsh.as_ref(),
                );
                let fish = append_optional_strings(
                    low_priority.fish.as_ref(),
                    high_priority.fish.as_ref(),
                );
                Ok(Some(Profile {
                    common,
                    bash,
                    zsh,
                    fish,
                    tcsh,
                }))
            },
        }
    }

    #[instrument(skip_all)]
    fn merge_options(
        low_priority: &Options,
        high_priority: &Options,
    ) -> Result<(Options, Vec<Warning>), MergeError> {
        let mut warnings = vec![];
        let root_key = KeyPath::from_iter(["options"]);
        let allow_key = root_key.push("allow");

        let (merged_allow_unfree, allow_unfree_warning) = shallow_merge_options(
            allow_key.push("unfree"),
            low_priority.allow.unfree,
            high_priority.allow.unfree,
        );

        let (merged_allow_broken, allow_broken_warning) = shallow_merge_options(
            allow_key.push("broken"),
            low_priority.allow.broken,
            high_priority.allow.broken,
        );

        let (merged_allow_licenses, allow_licenses_warning) = shallow_merge_options(
            allow_key.push("licenses"),
            low_priority.allow.licenses.as_deref(),
            high_priority.allow.licenses.as_deref(),
        );

        let (merged_semver_allow_pre_releases, allow_pre_releases_warning) = shallow_merge_options(
            root_key.extend(["semver", "allow-pre-releases"]),
            low_priority.semver.allow_pre_releases,
            high_priority.semver.allow_pre_releases,
        );

        let (merged_cuda_detection, cuda_detection_warning) = shallow_merge_options(
            root_key.push("cuda-detection"),
            low_priority.cuda_detection,
            high_priority.cuda_detection,
        );

        let (merged_systems, systems_warning) = shallow_merge_options(
            root_key.push("systems"),
            low_priority.systems.clone(),
            high_priority.systems.clone(),
        );

        let (merged_activate_mode, activate_mode_warning) = shallow_merge_options(
            root_key.extend(["activate", "mode"]),
            low_priority.activate.mode.clone(),
            high_priority.activate.mode.clone(),
        );

        let merged = Options {
            systems: merged_systems,
            allow: Allows {
                unfree: merged_allow_unfree,
                broken: merged_allow_broken,
                licenses: merged_allow_licenses,
            },
            semver: SemverOptions {
                allow_pre_releases: merged_semver_allow_pre_releases,
            },
            cuda_detection: merged_cuda_detection,
            activate: ActivateOptions {
                mode: merged_activate_mode,
            },
        };

        warnings.extend(
            [
                activate_mode_warning,
                allow_unfree_warning,
                allow_broken_warning,
                allow_licenses_warning,
                allow_pre_releases_warning,
                cuda_detection_warning,
                systems_warning,
            ]
            .into_iter()
            .flatten(),
        );

        Ok((merged, warnings))
    }

    #[instrument(skip_all)]
    fn merge_services(
        low_priority: &Services,
        high_priority: &Services,
    ) -> Result<(Services, Vec<Warning>), MergeError> {
        let (merged, warnings) = map_union(
            KeyPath::from_iter(["services"]),
            low_priority.inner(),
            high_priority.inner(),
        );
        Ok((Services(merged), warnings))
    }

    #[instrument(skip_all)]
    fn merge_build(
        low_priority: &Build,
        high_priority: &Build,
    ) -> Result<(Build, Vec<Warning>), MergeError> {
        let (merged, warnings) = map_union(
            KeyPath::from_iter(["build"]),
            low_priority.inner(),
            high_priority.inner(),
        );
        Ok((Build(merged), warnings))
    }

    #[instrument(skip_all)]
    fn merge_containerize(
        low_priority: Option<&Containerize>,
        high_priority: Option<&Containerize>,
    ) -> Result<(Option<Containerize>, Vec<Warning>), MergeError> {
        match (low_priority, high_priority) {
            (None, None) => Ok((None, vec![])),
            (Some(containerize_lp), None) => Ok((Some(containerize_lp.clone()), vec![])),
            (None, Some(containerize_hp)) => Ok((Some(containerize_hp.clone()), vec![])),
            (Some(Containerize { config: cfg_lp }), Some(Containerize { config: cfg_hp })) => {
                let (merged_config, warnings) =
                    deep_merge_optional_containerize_config(cfg_lp.as_ref(), cfg_hp.as_ref());
                Ok((
                    Some(Containerize {
                        config: merged_config,
                    }),
                    warnings,
                ))
            },
        }
    }
}

impl ManifestMergeTrait for ShallowMerger {
    fn merge(
        &self,
        low_priority: &Manifest<TypedOnly>,
        high_priority: &Manifest<TypedOnly>,
    ) -> Result<(Manifest<TypedOnly>, Vec<Warning>), MergeError> {
        trace!(section = "versions", "merging manifest section");
        let schema_high = high_priority.get_schema_version();
        let schema_low = low_priority.get_schema_version();
        let schema_version = Self::merge_version(schema_low, schema_high)?;

        trace!(section = "install", "merging manifest section");
        // Yeah, this sucks
        let install_low = match schema_low {
            KnownSchemaVersion::V1 => {
                if let Manifest {
                    inner:
                        TypedOnly {
                            parsed: Parsed::V1(m),
                        },
                } = low_priority
                {
                    VersionedContainer::V1(&m.install)
                } else {
                    return Err(MergeError::InternalError(
                        "schema version mismatch during merging".into(),
                    ));
                }
            },
            KnownSchemaVersion::V1_9_0 => {
                if let Manifest {
                    inner:
                        TypedOnly {
                            parsed: Parsed::V1_9_0(m),
                        },
                } = low_priority
                {
                    VersionedContainer::V1_9_0(&m.install)
                } else {
                    return Err(MergeError::InternalError(
                        "schema version mismatch during merging".into(),
                    ));
                }
            },
        };
        let install_high = match schema_high {
            KnownSchemaVersion::V1 => {
                if let Manifest {
                    inner:
                        TypedOnly {
                            parsed: Parsed::V1(m),
                        },
                } = high_priority
                {
                    VersionedContainer::V1(&m.install)
                } else {
                    return Err(MergeError::InternalError(
                        "schema version mismatch during merging".into(),
                    ));
                }
            },
            KnownSchemaVersion::V1_9_0 => {
                if let Manifest {
                    inner:
                        TypedOnly {
                            parsed: Parsed::V1_9_0(m),
                        },
                } = high_priority
                {
                    VersionedContainer::V1_9_0(&m.install)
                } else {
                    return Err(MergeError::InternalError(
                        "schema version mismatch during merging".into(),
                    ));
                }
            },
        };
        let (install_container, install_warnings) = Self::merge_install(install_low, install_high)?;

        trace!(section = "vars", "merging manifest section");
        let (vars, vars_warnings) = Self::merge_vars(&low_priority.vars(), &high_priority.vars())?;

        trace!(section = "hook", "merging manifest section");
        let hook = Self::merge_hook(low_priority.hook(), high_priority.hook())?;

        trace!(section = "profile", "merging manifest section");
        let profile = Self::merge_profile(low_priority.profile(), high_priority.profile())?;

        trace!(section = "options", "merging manifest section");
        let (options, options_warnings) =
            Self::merge_options(&low_priority.options(), &high_priority.options())?;

        trace!(section = "services", "merging manifest section");
        let (services, services_warnings) =
            Self::merge_services(&low_priority.services(), &high_priority.services())?;

        trace!(section = "build", "merging manifest section");
        let (build, build_warnings) =
            Self::merge_build(&low_priority.build(), &high_priority.build())?;

        trace!(section = "containerize", "merging manifest section");
        let (containerize, containerize_warnings) =
            Self::merge_containerize(low_priority.containerize(), high_priority.containerize())?;

        debug!("manifest pair merged successfully");

        let warnings = [
            install_warnings,
            vars_warnings,
            options_warnings,
            services_warnings,
            build_warnings,
            containerize_warnings,
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();

        let merged_manifest = match schema_version {
            KnownSchemaVersion::V1 => {
                let install = install_container
                    .into_v1()
                    .ok_or(MergeError::InternalError(
                        "unexpected schema version mismatch".into(),
                    ))?;
                let manifest = ManifestV1 {
                    version: ManifestVersion::from(1),
                    install,
                    vars,
                    hook,
                    profile,
                    options,
                    services,
                    build,
                    containerize,
                    // Intentionally blank out the includes since the includes are
                    // inputs to the merge operation.
                    include: Include::default(),
                };
                Manifest {
                    inner: TypedOnly {
                        parsed: Parsed::V1(manifest),
                    },
                }
            },
            KnownSchemaVersion::V1_9_0 => {
                let install = install_container
                    .into_v1_9_0()
                    .ok_or(MergeError::InternalError(
                        "unexpected schema version mismatch".into(),
                    ))?;
                let manifest = ManifestV1_9_0 {
                    version: parsed::v1_9_0::ManifestVersion::from(1),
                    install,
                    vars,
                    hook,
                    profile,
                    options,
                    services,
                    build,
                    containerize,
                    // Intentionally blank out the includes since the includes are
                    // inputs to the merge operation.
                    include: Include::default(),
                };
                Manifest {
                    inner: TypedOnly {
                        parsed: Parsed::V1_9_0(manifest),
                    },
                }
            },
        };

        Ok((merged_manifest, warnings))
    }
}

#[cfg(test)]
mod tests {

    use flox_test_utils::proptest::btree_maps_overlapping_keys;
    use pretty_assertions::assert_eq;
    use proptest::prelude::*;

    use super::*;
    use crate::parsed::common::{
        Allows,
        BuildDescriptor,
        ContainerizeConfig,
        SemverOptions,
        ServiceDescriptor,
    };
    use crate::parsed::v1::Install;
    use crate::parsed::v1::package_descriptor::ManifestPackageDescriptor;

    proptest! {
        // Ensures that the vars unique to each manifest are present in the merged output,
        // and that where the two manifests overlap the higher priority manifest is present
        // in the merged output.
        #[test]
        fn merges_vars_section(maps in btree_maps_overlapping_keys::<String>(1, 3)) {
            let vars1 = Vars(maps.map1.clone());
            let vars2 = Vars(maps.map2.clone());
            let (merged, warnings) = ShallowMerger::merge_vars(&vars1, &vars2).unwrap();
            let merged = merged.inner();
            for key in maps.unique_keys_map1.iter() {
                prop_assert_eq!(maps.map1.get(key), merged.get(key));
            }
            for key in maps.unique_keys_map2.iter() {
                prop_assert_eq!(maps.map2.get(key), merged.get(key));
            }
            for key in maps.duplicate_keys.iter() {
                prop_assert_eq!(maps.map2.get(key), merged.get(key));
                prop_assert!(
                    warnings.contains(&Warning::Overriding(KeyPath::from_iter(["vars", key]))),
                    "Expected a warning about overriding the var {key} in {warnings:?}"
                );
            }
        }

        // Ensures that the package descriptors unique to each manifest are present in the merged output,
        // and that where the two manifests overlap the higher priority manifest is present
        // in the merged output.
        #[test]
        fn merges_install_section(maps in btree_maps_overlapping_keys::<ManifestPackageDescriptor>(1, 3)) {
            let install1 = VersionedContainer::V1(&Install(maps.map1.clone()));
            let install2 = VersionedContainer::V1(&Install(maps.map2.clone()));
            let (merged, warnings) = ShallowMerger::merge_install(install1, install2).unwrap();
            let merged = merged.into_v1().unwrap();
            let merged = merged.inner();
            for key in maps.unique_keys_map1.iter() {
                prop_assert_eq!(maps.map1.get(key), merged.get(key));
            }
            for key in maps.unique_keys_map2.iter() {
                prop_assert_eq!(maps.map2.get(key), merged.get(key));
            }
            for key in maps.duplicate_keys.iter() {
                prop_assert_eq!(maps.map2.get(key), merged.get(key));
                prop_assert!(
                    warnings.contains(&Warning::Overriding(KeyPath::from_iter(["install", key]))),
                    "Expected a warning about overriding the package descriptor {key} in {warnings:?}"
                );
            }
        }

        // Ensures that the service descriptors unique to each manifest are present in the merged output,
        // and that where the two manifests overlap the higher priority manifest is present
        // in the merged output.
        #[test]
        fn merges_services_section(maps in btree_maps_overlapping_keys::<ServiceDescriptor>(1, 3)) {
            let services1 = Services(maps.map1.clone());
            let services2 = Services(maps.map2.clone());
            let (merged, warnings) = ShallowMerger::merge_services(&services1, &services2).unwrap();
            let merged = merged.inner();
            for key in maps.unique_keys_map1.iter() {
                prop_assert_eq!(maps.map1.get(key), merged.get(key));
            }
            for key in maps.unique_keys_map2.iter() {
                prop_assert_eq!(maps.map2.get(key), merged.get(key));
            }
            for key in maps.duplicate_keys.iter() {
                prop_assert_eq!(maps.map2.get(key), merged.get(key));
                prop_assert!(
                    warnings.contains(&Warning::Overriding(KeyPath::from_iter(["services", key]))),
                    "Expected a warning about overriding the service descriptor {key} in {warnings:?}"
                );
            }
        }

        // Ensures that the build descriptors unique to each manifest are present in the merged output,
        // and that where the two manifests overlap the higher priority manifest is present
        // in the merged output.
        #[test]
        fn merges_build_section(maps in btree_maps_overlapping_keys::<BuildDescriptor>(1, 3)) {
            let build1 = Build(maps.map1.clone());
            let build2 = Build(maps.map2.clone());
            let (merged, warnings) = ShallowMerger::merge_build(&build1, &build2).unwrap();
            let merged = merged.inner();
            for key in maps.unique_keys_map1.iter() {
                prop_assert_eq!(maps.map1.get(key), merged.get(key));
            }
            for key in maps.unique_keys_map2.iter() {
                prop_assert_eq!(maps.map2.get(key), merged.get(key));
            }
            for key in maps.duplicate_keys.iter() {
                prop_assert_eq!(maps.map2.get(key), merged.get(key));
                prop_assert!(
                    warnings.contains(&Warning::Overriding(KeyPath::from_iter(["build", key]))),
                    "Expected a warning about overriding the build descriptor {key} in {warnings:?}"
                );
            }
        }

        // Ensures that for any two manifests if they both have hooks, the merge joins them with a newline.
        // When one manifest has a hook and the other doesn't the hook that's present should be passed
        // straight through.
        #[test]
        fn merges_hook_section(hook1 in any::<Option<Hook>>(), hook2 in any::<Option<Hook>>()) {
            let merged = ShallowMerger::merge_hook(hook1.as_ref(), hook2.as_ref()).unwrap();
            let expected = match (hook1.unwrap_or_default().on_activate, hook2.unwrap_or_default().on_activate) {
                (Some(h1), Some(h2)) => Some(format!("{h1}\n{h2}")),
                (Some(h1), None) => Some(h1.clone()),
                (None, Some(h2)) => Some(h2.clone()),
                (None, None) => None,
            };
            prop_assert_eq!(merged.unwrap_or_default().on_activate, expected);
        }

        // Ensures that two arbitrary options sections are deep merged with the exception of
        // `options.systems` and `options.allow.licenses` which should be shallow merged.
        #[test]
        fn merges_options_section(options1 in any::<Options>(), options2 in any::<Options>()) {
            let (merged, _warnings) = ShallowMerger::merge_options(&options1, &options2).unwrap();
            let systems = options2.systems.or(options1.systems);
            let licenses = if options2.allow.licenses.is_some() {
                options2.allow.licenses.clone()
            } else {
                options1.allow.licenses.clone()
            };
            let allow = Allows {
                unfree: options2.allow.unfree.or(options1.allow.unfree),
                broken: options2.allow.broken.or(options1.allow.broken),
                licenses
            };
            let semver = SemverOptions { allow_pre_releases: options2.semver.allow_pre_releases.or(options1.semver.allow_pre_releases) };
            let cuda_detection = options2.cuda_detection.or(options1.cuda_detection);
            let activate = ActivateOptions {
                mode: options2.activate.mode.or(options1.activate.mode),
            };
            let expected = Options { systems, allow, semver, cuda_detection, activate };
            prop_assert_eq!(merged, expected);
        }

        // Ensures that a merged config retains either user, giving precedence to the higher
        // priority config.
        #[test]
        fn containerize_cfg_shallow_merges_user(
            cfg_lp in any::<ContainerizeConfig>(),
            cfg_hp in any::<ContainerizeConfig>(),
        ) {
            let (merged, warnings) = deep_merge_optional_containerize_config(Some(&cfg_lp), Some(&cfg_hp));
            let merged = merged.unwrap();
            if cfg_hp.user.is_some() {
                prop_assert_eq!(&merged.user, &cfg_hp.user);
            } else {
                prop_assert_eq!(&merged.user, &cfg_lp.user);
            }
            if cfg_hp.user.is_some() && cfg_lp.user.is_some() {
                prop_assert!(
                    warnings.contains(&Warning::Overriding(KeyPath::from_iter(["containerize", "config", "user"]))),
                    "Expected a warning about overriding the user in {warnings:?}"
                );
            }
        }

        // Ensures that a merged config deep merges the exposed ports.
        #[test]
        fn containerize_cfg_deep_merges_ports(
            cfg_lp in any::<ContainerizeConfig>(),
            cfg_hp in any::<ContainerizeConfig>(),
        ) {
            let (merged, warnings) = deep_merge_optional_containerize_config(Some(&cfg_lp), Some(&cfg_hp));
            let merged = merged.unwrap();
            match (cfg_lp.exposed_ports, cfg_hp.exposed_ports) {
                (None, None) => prop_assert!(merged.exposed_ports.is_none()),
                (Some(lp), None) => {
                    prop_assert_eq!(merged.exposed_ports, Some(lp));
                }
                (None, Some(hp)) => {
                    prop_assert_eq!(merged.exposed_ports, Some(hp));
                }
                (Some(lp), Some(hp)) => {
                    prop_assert!(merged.exposed_ports.is_some());
                    let merged_ports = merged.exposed_ports.unwrap();
                    for port in merged_ports.iter() {
                        prop_assert!(hp.contains(port) || lp.contains(port));
                    }

                    // No warnings should be generated for extending the exposed ports.
                    for warning in warnings.iter() {
                        prop_assert_ne!(warning, &Warning::Overriding(KeyPath::from_iter(["containerize", "exposed-ports"])));
                    }

                }
            }
        }

        // Ensures that a merged config shallow merges the `cmd` since appending two
        // argument lists likely produces an invalid command.
        #[test]
        fn containerize_cfg_shallow_merges_cmd(
            cfg_lp in any::<ContainerizeConfig>(),
            cfg_hp in any::<ContainerizeConfig>(),
        ) {
            let (merged, warnings) = deep_merge_optional_containerize_config(Some(&cfg_lp), Some(&cfg_hp));
            let merged = merged.unwrap();
            if cfg_hp.cmd.is_some() {
                prop_assert_eq!(&merged.cmd, &cfg_hp.cmd);
            } else {
                prop_assert_eq!(&merged.cmd, &cfg_lp.cmd);
            }

            if cfg_hp.cmd.is_some() && cfg_lp.cmd.is_some() {
                prop_assert!(warnings.contains(&Warning::Overriding(KeyPath::from_iter(["containerize", "config", "cmd"]))), "Expected a warning about overriding the cmd in {warnings:?}");
            }
        }

        // Ensures that volumes are deep merged.
        #[test]
        fn containerize_cfg_deep_merges_volumes(
            cfg_lp in any::<ContainerizeConfig>(),
            cfg_hp in any::<ContainerizeConfig>(),
        ) {
            let (merged, warnings) = deep_merge_optional_containerize_config(Some(&cfg_lp), Some(&cfg_hp));
            let merged = merged.unwrap();
            match (cfg_lp.volumes, cfg_hp.volumes) {
                (None, None) => prop_assert!(merged.volumes.is_none()),
                (Some(lp), None) => {
                    prop_assert_eq!(merged.volumes, Some(lp));
                }
                (None, Some(hp)) => {
                    prop_assert_eq!(merged.volumes, Some(hp));
                }
                (Some(lp), Some(hp)) => {
                    prop_assert!(merged.volumes.is_some());
                    let merged_volumes = merged.volumes.unwrap();
                    for vol in merged_volumes.iter() {
                        prop_assert!(hp.contains(vol) || lp.contains(vol));
                    }
                }
            }

            // No warnings should be generated for extending the volumes set.
            for warning in warnings.iter() {
                prop_assert_ne!(warning, &Warning::Overriding(KeyPath::from_iter(["containerize", "volumes"])));
            }
        }

        // Ensures that a merged config retains a single working directory, preferrably
        // the one from the higher priority config.
        #[test]
        fn containerize_cfg_shallow_merges_working_dir(
            cfg_lp in any::<ContainerizeConfig>(),
            cfg_hp in any::<ContainerizeConfig>(),
        ) {
            let (merged, warnings) = deep_merge_optional_containerize_config(Some(&cfg_lp), Some(&cfg_hp));
            let merged = merged.unwrap();
            if cfg_hp.working_dir.is_some() {
                prop_assert_eq!(&merged.working_dir, &cfg_hp.working_dir);
            } else {
                prop_assert_eq!(&merged.working_dir, &cfg_lp.working_dir);
            }

            if cfg_hp.working_dir.is_some() && cfg_lp.working_dir.is_some() {
                prop_assert!(
                    warnings.contains(&Warning::Overriding(KeyPath::from_iter(["containerize", "config", "working-dir"]))),
                    "Expected a warning about overriding the working dir in {warnings:?}"
                );
            }
        }

        // Ensures that the labels from a merged config are deep merged.
        #[test]
        fn containerize_cfg_deep_merges_labels(
            cfg_lp in any::<ContainerizeConfig>(),
            cfg_hp in any::<ContainerizeConfig>(),
        ) {
            let (merged, warnings) = deep_merge_optional_containerize_config(Some(&cfg_lp), Some(&cfg_hp));
            let merged = merged.unwrap();
            match (cfg_lp.labels, cfg_hp.labels) {
                (None, None) => prop_assert!(merged.labels.is_none()),
                (Some(lp), None) => {
                    prop_assert_eq!(merged.labels, Some(lp));
                }
                (None, Some(hp)) => {
                    prop_assert_eq!(merged.labels, Some(hp));
                }
                (Some(lp), Some(hp)) => {
                    prop_assert!(merged.labels.is_some());
                    let merged_labels = merged.labels.unwrap();
                    for key in merged_labels.keys() {
                        if hp.contains_key(key) {
                            prop_assert_eq!(merged_labels.get(key), hp.get(key));
                        } else {
                            prop_assert_eq!(merged_labels.get(key), lp.get(key));
                        }

                        if hp.contains_key(key) && lp.contains_key(key) {
                            prop_assert!(
                                warnings.contains(&Warning::Overriding(KeyPath::from_iter(["containerize", "config", "labels", key]))),
                                "Expected a warning about overriding the label {key} in {warnings:?}"
                            );
                        }
                    }
                }
            }
        }

        // Ensures that a single stop signal is retain in the merge.
        #[test]
        fn containerize_cfg_shallow_merges_stop_signal(
            cfg_lp in any::<ContainerizeConfig>(),
            cfg_hp in any::<ContainerizeConfig>(),
        ) {
            let (merged, warnings) = deep_merge_optional_containerize_config(Some(&cfg_lp), Some(&cfg_hp));
            let merged = merged.unwrap();
            if cfg_hp.stop_signal.is_some() {
                prop_assert_eq!(&merged.stop_signal, &cfg_hp.stop_signal);
            } else {
                prop_assert_eq!(&merged.stop_signal, &cfg_lp.stop_signal);
            }

            if cfg_hp.stop_signal.is_some() && cfg_lp.stop_signal.is_some() {
                prop_assert!(
                    warnings.contains(&Warning::Overriding(KeyPath::from_iter(["containerize", "config", "stop-signal"]))),
                    "Expected a warning about overriding the stop signal in {warnings:?}"
                );
            }
        }

        // This is essentially checking that the deep merge happens at all.
        // The details/correctness of the deep merge are verified by the
        // more focused tests above.
        #[test]
        fn containerize_deep_merges_config(
            cfg_lp in any::<ContainerizeConfig>(),
            cfg_hp in any::<ContainerizeConfig>()
        ) {
            let cont_lp = Containerize { config: Some(cfg_lp.clone())};
            let cont_hp = Containerize { config: Some(cfg_hp.clone())};
            let (maybe_merged, _warnings) = ShallowMerger::merge_containerize(Some(&cont_lp), Some(&cont_hp)).unwrap();
            prop_assert!(maybe_merged.is_some()); // They were both Some(_) to start out
            let merged_cont = maybe_merged.unwrap();
            prop_assert!(merged_cont.config.is_some());
            let merged_cfg = merged_cont.config.unwrap();
            let (expected_cfg, _) = deep_merge_optional_containerize_config(Some(&cfg_lp), Some(&cfg_hp));
            let expected_cfg = expected_cfg.unwrap();
            prop_assert_eq!(merged_cfg, expected_cfg);
        }
    }

    #[test]
    fn containerize_does_trivial_merge() {
        let (merged, _warnings) = ShallowMerger::merge_containerize(None, None).unwrap();
        assert_eq!(None, merged);

        let low_priority = Some(Containerize::default());
        let high_priority = None;
        let (merged, _warnings) =
            ShallowMerger::merge_containerize(low_priority.as_ref(), high_priority.as_ref())
                .unwrap();
        assert_eq!(low_priority, merged);

        let low_priority = None;
        let high_priority = Some(Containerize::default());
        let (merged, _warnings) =
            ShallowMerger::merge_containerize(low_priority.as_ref(), high_priority.as_ref())
                .unwrap();
        assert_eq!(high_priority, merged);
    }

    #[test]
    fn merges_profile_sections_both_some() {
        let low_priority = Some(Profile {
            common: Some("common1".to_string()),
            bash: Some("bash1".to_string()),
            zsh: Some("zsh1".to_string()),
            fish: Some("fish1".to_string()),
            tcsh: Some("tcsh1".to_string()),
        });
        let high_priority = Some(Profile {
            common: Some("common2".to_string()),
            bash: Some("bash2".to_string()),
            zsh: Some("zsh2".to_string()),
            fish: Some("fish2".to_string()),
            tcsh: Some("tcsh2".to_string()),
        });
        let expected = Some(Profile {
            common: Some("common1\ncommon2".to_string()),
            bash: Some("bash1\nbash2".to_string()),
            zsh: Some("zsh1\nzsh2".to_string()),
            fish: Some("fish1\nfish2".to_string()),
            tcsh: Some("tcsh1\ntcsh2".to_string()),
        });
        let merged =
            ShallowMerger::merge_profile(low_priority.as_ref(), high_priority.as_ref()).unwrap();
        assert_eq!(merged, expected);
    }

    #[test]
    fn merges_profile_sections_only_low_priority() {
        let low_priority = Some(Profile {
            common: Some("common1".to_string()),
            bash: Some("bash1".to_string()),
            zsh: Some("zsh1".to_string()),
            fish: Some("fish1".to_string()),
            tcsh: Some("tcsh1".to_string()),
        });
        let high_priority = Some(Profile::default());

        assert_eq!(
            ShallowMerger::merge_profile(low_priority.as_ref(), high_priority.as_ref()).unwrap(),
            low_priority
        );
        assert_eq!(
            ShallowMerger::merge_profile(low_priority.as_ref(), None).unwrap(),
            low_priority
        );
    }

    #[test]
    fn merges_profile_sections_only_high_priority() {
        let low_priority = Some(Profile::default());
        let high_priority = Some(Profile {
            common: Some("common2".to_string()),
            bash: Some("bash2".to_string()),
            zsh: Some("zsh2".to_string()),
            fish: Some("fish2".to_string()),
            tcsh: Some("tcsh2".to_string()),
        });

        assert_eq!(
            ShallowMerger::merge_profile(low_priority.as_ref(), high_priority.as_ref()).unwrap(),
            high_priority,
        );
        assert_eq!(
            ShallowMerger::merge_profile(None, high_priority.as_ref()).unwrap(),
            high_priority,
        );
    }

    #[test]
    fn merges_profile_sections_both_inner_none() {
        assert_eq!(
            ShallowMerger::merge_profile(
                Some(Profile::default()).as_ref(),
                Some(Profile::default()).as_ref()
            )
            .unwrap(),
            Some(Profile::default()),
        );
    }

    #[test]
    fn merges_profile_sections_both_outer_none() {
        assert_eq!(ShallowMerger::merge_profile(None, None).unwrap(), None);
    }
}
