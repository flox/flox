use flox_core::Version;

use super::{
    append_optional_strings,
    deep_merge_optional_containerize_config,
    map_union,
    ManifestMergeStrategy,
    MergeError,
};
use crate::models::manifest::typed::{
    Allows,
    Build,
    Containerize,
    Hook,
    Include,
    Inner,
    Install,
    Manifest,
    Options,
    Profile,
    SemverOptions,
    Services,
    Vars,
};

/// Merges two manifests by applying `manifest2` on top of `manifest1` and
/// overwriting any conflicts for keys within the top-level of each `Manifest`
/// field, with the exception of `profile` and `hooks`.
pub(crate) struct ShallowMerger;

impl ManifestMergeStrategy for ShallowMerger {
    fn merge_version(
        low_priority: &Version<1>,
        high_priority: &Version<1>,
    ) -> Result<Version<1>, MergeError> {
        if low_priority != high_priority {
            unreachable!("versions are hardcoded into Manifest");
        }

        Ok(high_priority.clone())
    }

    fn merge_install(
        low_priority: &Install,
        high_priority: &Install,
    ) -> Result<Install, MergeError> {
        let merged = map_union(low_priority.inner(), high_priority.inner());
        Ok(Install(merged))
    }

    /// Keys in `manifest2` overwrite keys in `manifest1`.
    fn merge_vars(low_priority: &Vars, high_priority: &Vars) -> Result<Vars, MergeError> {
        let merged = map_union(low_priority.inner(), high_priority.inner());
        Ok(Vars(merged))
    }

    fn merge_hook(low_priority: &Hook, high_priority: &Hook) -> Result<Hook, MergeError> {
        Ok(Hook {
            on_activate: append_optional_strings(
                low_priority.on_activate.as_ref(),
                high_priority.on_activate.as_ref(),
            ),
        })
    }

    fn merge_profile(
        low_priority: &Profile,
        high_priority: &Profile,
    ) -> Result<Profile, MergeError> {
        let common =
            append_optional_strings(low_priority.common.as_ref(), high_priority.common.as_ref());
        let bash = append_optional_strings(low_priority.bash.as_ref(), high_priority.bash.as_ref());
        let zsh = append_optional_strings(low_priority.zsh.as_ref(), high_priority.zsh.as_ref());
        let tcsh = append_optional_strings(low_priority.tcsh.as_ref(), high_priority.tcsh.as_ref());
        let fish = append_optional_strings(low_priority.fish.as_ref(), high_priority.fish.as_ref());
        let merged = Profile {
            common,
            bash,
            zsh,
            fish,
            tcsh,
        };
        Ok(merged)
    }

    fn merge_options(
        low_priority: &Options,
        high_priority: &Options,
    ) -> Result<Options, MergeError> {
        let merged_allow_unfree = high_priority.allow.unfree.or(low_priority.allow.unfree);
        let merged_allow_broken = high_priority.allow.broken.or(low_priority.allow.broken);
        let merged_allow_licenses = if high_priority.allow.licenses.is_empty() {
            low_priority.allow.licenses.clone()
        } else {
            high_priority.allow.licenses.clone()
        };
        let merged_semver_allow_pre_releases = high_priority
            .semver
            .allow_pre_releases
            .or(low_priority.semver.allow_pre_releases);
        let merged_cuda_detection = high_priority.cuda_detection.or(low_priority.cuda_detection);
        let merged_systems = high_priority
            .systems
            .clone()
            .or(low_priority.systems.clone());
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
        };
        Ok(merged)
    }

    fn merge_services(
        low_priority: &Services,
        high_priority: &Services,
    ) -> Result<Services, MergeError> {
        let merged = map_union(low_priority.inner(), high_priority.inner());
        Ok(Services(merged))
    }

    fn merge_build(low_priority: &Build, high_priority: &Build) -> Result<Build, MergeError> {
        let merged = map_union(low_priority.inner(), high_priority.inner());
        Ok(Build(merged))
    }

    fn merge_containerize(
        low_priority: Option<&Containerize>,
        high_priority: Option<&Containerize>,
    ) -> Result<Option<Containerize>, MergeError> {
        match (low_priority, high_priority) {
            (None, None) => Ok(None),
            (Some(containerize_lp), None) => Ok(Some(containerize_lp.clone())),
            (None, Some(containerize_hp)) => Ok(Some(containerize_hp.clone())),
            (Some(Containerize { config: cfg_lp }), Some(Containerize { config: cfg_hp })) => {
                let merged =
                    deep_merge_optional_containerize_config(cfg_lp.as_ref(), cfg_hp.as_ref());
                Ok(Some(Containerize { config: merged }))
            },
        }
    }

    fn merge(
        &self,
        low_priority: &Manifest,
        high_priority: &Manifest,
    ) -> Result<Manifest, MergeError> {
        let manifest = Manifest {
            version: Self::merge_version(&low_priority.version, &high_priority.version)?,
            install: Self::merge_install(&low_priority.install, &high_priority.install)?,
            vars: Self::merge_vars(&low_priority.vars, &high_priority.vars)?,
            hook: Self::merge_hook(&low_priority.hook, &high_priority.hook)?,
            profile: Self::merge_profile(&low_priority.profile, &high_priority.profile)?,
            options: Self::merge_options(&low_priority.options, &high_priority.options)?,
            services: Self::merge_services(&low_priority.services, &high_priority.services)?,
            build: Self::merge_build(&low_priority.build, &high_priority.build)?,
            containerize: Self::merge_containerize(
                low_priority.containerize.as_ref(),
                high_priority.containerize.as_ref(),
            )?,
            // Intentionally blank out the includes since the includes are
            // inputs to the merge operation.
            include: Include::default(),
        };

        Ok(manifest)
    }
}

#[cfg(test)]
mod tests {

    use flox_test_utils::proptest::btree_maps_overlapping_keys;
    use pretty_assertions::assert_eq;
    use proptest::prelude::*;

    use super::*;
    use crate::models::manifest::typed::{
        Allows,
        BuildDescriptor,
        ContainerizeConfig,
        ManifestPackageDescriptor,
        SemverOptions,
        ServiceDescriptor,
    };

    proptest! {
        // Ensures that the vars unique to each manifest are present in the merged output,
        // and that where the two manifests overlap the higher priority manifest is present
        // in the merged output.
        #[test]
        fn merges_vars_section(maps in btree_maps_overlapping_keys::<String>(1, 3)) {
            let vars1 = Vars(maps.map1.clone());
            let vars2 = Vars(maps.map2.clone());
            let merged = ShallowMerger::merge_vars(&vars1, &vars2).unwrap();
            let merged = merged.inner();
            for key in maps.unique_keys_map1.iter() {
                prop_assert_eq!(maps.map1.get(key), merged.get(key));
            }
            for key in maps.unique_keys_map2.iter() {
                prop_assert_eq!(maps.map2.get(key), merged.get(key));
            }
            for key in maps.duplicate_keys.iter() {
                prop_assert_eq!(maps.map2.get(key), merged.get(key));
            }
        }

        // Ensures that the package descriptors unique to each manifest are present in the merged output,
        // and that where the two manifests overlap the higher priority manifest is present
        // in the merged output.
        #[test]
        fn merges_install_section(maps in btree_maps_overlapping_keys::<ManifestPackageDescriptor>(1, 3)) {
            let install1 = Install(maps.map1.clone());
            let install2 = Install(maps.map2.clone());
            let merged = ShallowMerger::merge_install(&install1, &install2).unwrap();
            let merged = merged.inner();
            for key in maps.unique_keys_map1.iter() {
                prop_assert_eq!(maps.map1.get(key), merged.get(key));
            }
            for key in maps.unique_keys_map2.iter() {
                prop_assert_eq!(maps.map2.get(key), merged.get(key));
            }
            for key in maps.duplicate_keys.iter() {
                prop_assert_eq!(maps.map2.get(key), merged.get(key));
            }
        }

        // Ensures that the service descriptors unique to each manifest are present in the merged output,
        // and that where the two manifests overlap the higher priority manifest is present
        // in the merged output.
        #[test]
        fn merges_services_section(maps in btree_maps_overlapping_keys::<ServiceDescriptor>(1, 3)) {
            let services1 = Services(maps.map1.clone());
            let services2 = Services(maps.map2.clone());
            let merged = ShallowMerger::merge_services(&services1, &services2).unwrap();
            let merged = merged.inner();
            for key in maps.unique_keys_map1.iter() {
                prop_assert_eq!(maps.map1.get(key), merged.get(key));
            }
            for key in maps.unique_keys_map2.iter() {
                prop_assert_eq!(maps.map2.get(key), merged.get(key));
            }
            for key in maps.duplicate_keys.iter() {
                prop_assert_eq!(maps.map2.get(key), merged.get(key));
            }
        }

        // Ensures that the build descriptors unique to each manifest are present in the merged output,
        // and that where the two manifests overlap the higher priority manifest is present
        // in the merged output.
        #[test]
        fn merges_build_section(maps in btree_maps_overlapping_keys::<BuildDescriptor>(1, 3)) {
            let build1 = Build(maps.map1.clone());
            let build2 = Build(maps.map2.clone());
            let merged = ShallowMerger::merge_build(&build1, &build2).unwrap();
            let merged = merged.inner();
            for key in maps.unique_keys_map1.iter() {
                prop_assert_eq!(maps.map1.get(key), merged.get(key));
            }
            for key in maps.unique_keys_map2.iter() {
                prop_assert_eq!(maps.map2.get(key), merged.get(key));
            }
            for key in maps.duplicate_keys.iter() {
                prop_assert_eq!(maps.map2.get(key), merged.get(key));
            }
        }

        // Ensures that for any two manifests if they both have hooks, the merge joins them with a newline.
        // When one manifest has a hook and the other doesn't the hook that's present should be passed
        // straight through.
        #[test]
        fn merges_hook_section(hook1 in any::<Hook>(), hook2 in any::<Hook>()) {
            let merged = ShallowMerger::merge_hook(&hook1, &hook2).unwrap();
            let expected = match (hook1.on_activate, hook2.on_activate) {
                (Some(h1), Some(h2)) => Some(format!("{h1}\n{h2}")),
                (Some(h1), None) => Some(h1.clone()),
                (None, Some(h2)) => Some(h2.clone()),
                (None, None) => None,
            };
            prop_assert_eq!(merged.on_activate, expected);
        }

        // Ensures that two arbitrary options sections are deep merged with the exception of
        // `options.systems` and `options.allow.licenses` which should be shallow merged.
        #[test]
        fn merges_options_section(options1 in any::<Options>(), options2 in any::<Options>()) {
            let merged = ShallowMerger::merge_options(&options1, &options2).unwrap();
            let systems = options2.systems.or(options1.systems);
            let allow = Allows {
                unfree: options2.allow.unfree.or(options1.allow.unfree),
                broken: options2.allow.broken.or(options1.allow.broken),
                licenses: if options2.allow.licenses.is_empty() { options1.allow.licenses.clone()} else { options2.allow.licenses.clone() }
            };
            let semver = SemverOptions { allow_pre_releases: options2.semver.allow_pre_releases.or(options1.semver.allow_pre_releases) };
            let cuda_detection = options2.cuda_detection.or(options1.cuda_detection);
            let expected = Options { systems, allow, semver, cuda_detection, };
            prop_assert_eq!(merged, expected);
        }

        // Ensures that a merged config retains either user, giving precedence to the higher
        // priority config.
        #[test]
        fn containerize_cfg_shallow_merges_user(
            cfg_lp in any::<ContainerizeConfig>(),
            cfg_hp in any::<ContainerizeConfig>(),
        ) {
            let merged = deep_merge_optional_containerize_config(Some(&cfg_lp), Some(&cfg_hp)).unwrap();
            if cfg_hp.user.is_some() {
                prop_assert_eq!(merged.user, cfg_hp.user);
            } else {
                prop_assert_eq!(merged.user, cfg_lp.user);
            }
        }

        // Ensures that a merged config deep merges the exposed ports.
        #[test]
        fn containerize_cfg_deep_merges_ports(
            cfg_lp in any::<ContainerizeConfig>(),
            cfg_hp in any::<ContainerizeConfig>(),
        ) {
            let merged = deep_merge_optional_containerize_config(Some(&cfg_lp), Some(&cfg_hp)).unwrap();
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
            let merged = deep_merge_optional_containerize_config(Some(&cfg_lp), Some(&cfg_hp)).unwrap();
            if cfg_hp.cmd.is_some() {
                prop_assert_eq!(merged.cmd, cfg_hp.cmd);
            } else {
                prop_assert_eq!(merged.cmd, cfg_lp.cmd);
            }
        }

        // Ensures that volumes are deep merged.
        #[test]
        fn containerize_cfg_deep_merges_volumes(
            cfg_lp in any::<ContainerizeConfig>(),
            cfg_hp in any::<ContainerizeConfig>(),
        ) {
            let merged = deep_merge_optional_containerize_config(Some(&cfg_lp), Some(&cfg_hp)).unwrap();
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
        }

        // Ensures that a merged config retains a single working directory, preferrably
        // the one from the higher priority config.
        #[test]
        fn containerize_cfg_shallow_merges_working_dir(
            cfg_lp in any::<ContainerizeConfig>(),
            cfg_hp in any::<ContainerizeConfig>(),
        ) {
            let merged = deep_merge_optional_containerize_config(Some(&cfg_lp), Some(&cfg_hp)).unwrap();
            if cfg_hp.working_dir.is_some() {
                prop_assert_eq!(merged.working_dir, cfg_hp.working_dir);
            } else {
                prop_assert_eq!(merged.working_dir, cfg_lp.working_dir);
            }
        }

        // Ensures that the labels from a merged config are deep merged.
        #[test]
        fn containerize_cfg_deep_merges_labels(
            cfg_lp in any::<ContainerizeConfig>(),
            cfg_hp in any::<ContainerizeConfig>(),
        ) {
            let merged = deep_merge_optional_containerize_config(Some(&cfg_lp), Some(&cfg_hp)).unwrap();
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
            let merged = deep_merge_optional_containerize_config(Some(&cfg_lp), Some(&cfg_hp)).unwrap();
            if cfg_hp.stop_signal.is_some() {
                prop_assert_eq!(merged.stop_signal, cfg_hp.stop_signal);
            } else {
                prop_assert_eq!(merged.stop_signal, cfg_lp.stop_signal);
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
            let maybe_merged = ShallowMerger::merge_containerize(Some(&cont_lp), Some(&cont_hp)).unwrap();
            prop_assert!(maybe_merged.is_some()); // They were both Some(_) to start out
            let merged_cont = maybe_merged.unwrap();
            prop_assert!(merged_cont.config.is_some());
            let merged_cfg = merged_cont.config.unwrap();
            let expected_cfg = deep_merge_optional_containerize_config(Some(&cfg_lp), Some(&cfg_hp)).unwrap();
            prop_assert_eq!(merged_cfg, expected_cfg);
        }
    }

    #[test]
    fn containerize_does_trivial_merge() {
        assert_eq!(None, ShallowMerger::merge_containerize(None, None).unwrap());
        let low_priority = Some(Containerize::default());
        let high_priority = None;
        assert_eq!(
            low_priority,
            ShallowMerger::merge_containerize(low_priority.as_ref(), high_priority.as_ref())
                .unwrap()
        );
        let low_priority = None;
        let high_priority = Some(Containerize::default());
        assert_eq!(
            high_priority,
            ShallowMerger::merge_containerize(low_priority.as_ref(), high_priority.as_ref())
                .unwrap()
        );
    }

    #[test]
    fn merges_profile_sections_both_some() {
        let low_priority = Profile {
            common: Some("common1".to_string()),
            bash: Some("bash1".to_string()),
            zsh: Some("zsh1".to_string()),
            fish: Some("fish1".to_string()),
            tcsh: Some("tcsh1".to_string()),
        };
        let high_priority = Profile {
            common: Some("common2".to_string()),
            bash: Some("bash2".to_string()),
            zsh: Some("zsh2".to_string()),
            fish: Some("fish2".to_string()),
            tcsh: Some("tcsh2".to_string()),
        };
        let expected = Profile {
            common: Some("common1\ncommon2".to_string()),
            bash: Some("bash1\nbash2".to_string()),
            zsh: Some("zsh1\nzsh2".to_string()),
            fish: Some("fish1\nfish2".to_string()),
            tcsh: Some("tcsh1\ntcsh2".to_string()),
        };
        let merged = ShallowMerger::merge_profile(&low_priority, &high_priority).unwrap();
        assert_eq!(merged, expected);
    }

    #[test]
    fn merges_profile_sections_only_low_priority() {
        let low_priority = Profile {
            common: Some("common1".to_string()),
            bash: Some("bash1".to_string()),
            zsh: Some("zsh1".to_string()),
            fish: Some("fish1".to_string()),
            tcsh: Some("tcsh1".to_string()),
        };
        let high_priority = Profile::default();
        let merged = ShallowMerger::merge_profile(&low_priority, &high_priority).unwrap();
        assert_eq!(merged, low_priority);
    }

    #[test]
    fn merges_profile_sections_only_high_priority() {
        let low_priority = Profile::default();
        let high_priority = Profile {
            common: Some("common2".to_string()),
            bash: Some("bash2".to_string()),
            zsh: Some("zsh2".to_string()),
            fish: Some("fish2".to_string()),
            tcsh: Some("tcsh2".to_string()),
        };
        let merged = ShallowMerger::merge_profile(&low_priority, &high_priority).unwrap();
        assert_eq!(merged, high_priority);
    }

    #[test]
    fn merges_profile_sections_both_none() {
        assert_eq!(
            Profile::default(),
            ShallowMerger::merge_profile(&Profile::default(), &Profile::default()).unwrap()
        );
    }
}
