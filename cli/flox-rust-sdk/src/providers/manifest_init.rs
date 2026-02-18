use std::collections::HashSet;

use flox_core::data::System;
use flox_manifest::raw::{
    DEFAULT_SYSTEMS_STR,
    MANIFEST_BUILD_KEY,
    MANIFEST_HOOK_KEY,
    MANIFEST_INCLUDE_KEY,
    MANIFEST_INSTALL_KEY,
    MANIFEST_OPTIONS_KEY,
    MANIFEST_PROFILE_KEY,
    MANIFEST_SERVICES_KEY,
    MANIFEST_SYSTEMS_KEY,
    MANIFEST_VARS_KEY,
    MANIFEST_VERSION_KEY,
};
use flox_manifest::{Manifest, ManifestError, TomlParsed, Validated};
use indoc::indoc;
use toml_edit::{Array, DocumentMut, InlineTable, Item, Key, Table, Value};

use crate::flox::Features;
use crate::models::environment::path_environment::InitCustomization;

#[derive(Debug, thiserror::Error)]
pub enum ManifestInitError {
    #[error("internal error: init created invalid manifest: {0}")]
    InvalidManifest(#[source] ManifestError),
}

#[derive(Debug, Clone)]
pub struct ManifestInitializer;

impl ManifestInitializer {
    pub fn new_documented(
        _features: Features,
        systems: &[&System],
        customization: &InitCustomization,
    ) -> Result<Manifest<Validated>, ManifestInitError> {
        let mut manifest = DocumentMut::new();

        Self::add_header(&mut manifest);
        Self::add_version(&mut manifest);
        Self::add_install_section(&mut manifest, customization, true);
        Self::add_vars_section(&mut manifest);
        Self::add_hook_section(&mut manifest, customization, true);
        Self::add_profile_section(&mut manifest, customization, true);
        Self::add_services_section(&mut manifest);
        Self::add_include_section(&mut manifest);
        Self::add_build_section(&mut manifest);
        Self::add_options_section(&mut manifest, systems, customization);

        Manifest::<TomlParsed>::validate_toml(&manifest).map_err(ManifestInitError::InvalidManifest)
    }

    /// Create a minimal [DocumentMut] that is close to what will actually be
    /// generated, but more concise.
    /// Note that this isn't a valid TypedManifest because it doesn't include
    /// version.
    pub fn new_minimal(customization: &InitCustomization) -> DocumentMut {
        let mut manifest = DocumentMut::new();

        Self::add_install_section(&mut manifest, customization, false);
        Self::add_hook_section(&mut manifest, customization, false);
        Self::add_profile_section(&mut manifest, customization, false);
        // We don't need to call add_options_section because it's only used for
        // activate mode, which we don't need to print when showing a more
        // concise manifest to the user

        manifest
    }

    /// Populates a header at the top of the manifest with a link to
    /// the documentation.
    fn add_header(manifest: &mut DocumentMut) {
        manifest.decor_mut().set_prefix(indoc! {r#"
            ## Flox Environment Manifest -----------------------------------------
            ##
            ##   _Everything_ you need to know about the _manifest_ is here:
            ##
            ##   https://flox.dev/docs/reference/command-reference/manifest.toml/
            ##
            ## -------------------------------------------------------------------
            # Flox manifest version managed by Flox CLI
        "#});
    }

    /// Populates the manifest schema version.
    fn add_version(manifest: &mut DocumentMut) {
        // `version` number
        manifest.insert(MANIFEST_VERSION_KEY, toml_edit::value(1));
    }

    /// Populates an example install section with any packages necessary for
    /// init customizations.
    fn add_install_section(
        manifest: &mut DocumentMut,
        customization: &InitCustomization,
        documented: bool,
    ) {
        let packages_vec = vec![];
        let packages = customization.packages.as_ref().unwrap_or(&packages_vec);

        // We don't want to add an empty [install] table
        if packages.is_empty() && !documented {
            return;
        };

        let mut install_table = if packages.is_empty() {
            // Add comment with example packages
            let mut table = Table::new();

            table.decor_mut().set_suffix(indoc! {r#"

                # gum.pkg-path = "gum"
                # gum.version = "^0.14.5""#
            });

            table
        } else {
            Table::from_iter(packages.iter().map(|pkg| {
                let mut table = InlineTable::from(pkg);
                table.set_dotted(true);
                (&pkg.id, table)
            }))
        };

        if documented {
            install_table.decor_mut().set_prefix(indoc! {r#"


            ## Install Packages --------------------------------------------------
            ##  $ flox install gum  <- puts a package in [install] section below
            ##  $ flox search gum   <- search for a package
            ##  $ flox show gum     <- show all versions of a package
            ## -------------------------------------------------------------------
            "#});
        }

        manifest.insert(MANIFEST_INSTALL_KEY, Item::Table(install_table));
    }

    /// Populates an example vars section.
    fn add_vars_section(manifest: &mut DocumentMut) {
        let mut vars_table = Table::new();

        vars_table.decor_mut().set_prefix(indoc! {r#"


            ## Environment Variables ---------------------------------------------
            ##  ... available for use in the activated environment
            ##      as well as [hook], [profile] scripts and [services] below.
            ## -------------------------------------------------------------------
        "#});

        // [sic]: vars not customized using InitCustomization yet
        vars_table.decor_mut().set_suffix(indoc! {r#"

            # INTRO_MESSAGE = "It's gettin' Flox in here""#});

        manifest.insert(MANIFEST_VARS_KEY, Item::Table(vars_table));
    }

    /// Populates an example hook section with any automatic setup added by
    /// init customizations.
    fn add_hook_section(
        manifest: &mut DocumentMut,
        customization: &InitCustomization,
        documented: bool,
    ) {
        let mut hook_table = Table::new();

        if documented {
            hook_table.decor_mut().set_prefix(indoc! {r#"


                ## Activation Hook ---------------------------------------------------
                ##  ... run by _bash_ shell when you run 'flox activate'.
                ## -------------------------------------------------------------------
            "#});
        }

        if let Some(ref hook_on_activate_script) = customization.hook_on_activate {
            let on_activate_content: String = indent::indent_all_by(2, hook_on_activate_script);

            hook_table.insert("on-activate", toml_edit::value(on_activate_content));
        } else {
            // We don't want to add an empty [hook] table
            if !documented {
                return;
            }
            hook_table.decor_mut().set_suffix(indoc! {r#"

                # on-activate = '''
                #   # -> Set variables, create files and directories
                #   # -> Perform initialization steps, e.g. create a python venv
                #   # -> Useful environment variables:
                #   #      - FLOX_ENV_PROJECT=/home/user/example
                #   #      - FLOX_ENV=/home/user/example/.flox/run
                #   #      - FLOX_ENV_CACHE=/home/user/example/.flox/cache
                # '''"#
            });
        };

        manifest.insert(MANIFEST_HOOK_KEY, Item::Table(hook_table));
    }

    /// Populates an example profile section with any automatic setup added by
    /// init customizations.
    fn add_profile_section(
        manifest: &mut DocumentMut,
        customization: &InitCustomization,
        documented: bool,
    ) {
        let mut profile_table = Table::new();

        if documented {
            profile_table.decor_mut().set_prefix(indoc! {r#"


                ## Profile script ----------------------------------------------------
                ## ... sourced by _your shell_ when you run 'flox activate'.
                ## -------------------------------------------------------------------
            "#});
        }

        match customization {
            InitCustomization {
                profile_common: None,
                profile_bash: None,
                profile_fish: None,
                profile_tcsh: None,
                profile_zsh: None,
                ..
            } => {
                // We don't want to add an empty [profile] table
                if !documented {
                    return;
                }
                profile_table.decor_mut().set_suffix(indoc! {r#"

                    # common = '''
                    #   gum style \
                    #   --foreground 212 --border-foreground 212 --border double \
                    #   --align center --width 50 --margin "1 2" --padding "2 4" \
                    #     $INTRO_MESSAGE
                    # '''
                    ## Shell-specific customizations such as setting aliases go here:
                    # bash = ...
                    # zsh  = ...
                    # fish = ..."#
                });
            },
            _ => {
                if let Some(profile_common) = &customization.profile_common {
                    profile_table.insert(
                        "common",
                        toml_edit::value(indent::indent_all_by(2, profile_common)),
                    );
                }
                if let Some(profile_bash) = &customization.profile_bash {
                    profile_table.insert(
                        "bash",
                        toml_edit::value(indent::indent_all_by(2, profile_bash)),
                    );
                }
                if let Some(profile_fish) = &customization.profile_fish {
                    profile_table.insert(
                        "fish",
                        toml_edit::value(indent::indent_all_by(2, profile_fish)),
                    );
                }
                if let Some(profile_tcsh) = &customization.profile_tcsh {
                    profile_table.insert(
                        "tcsh",
                        toml_edit::value(indent::indent_all_by(2, profile_tcsh)),
                    );
                }
                if let Some(profile_zsh) = &customization.profile_zsh {
                    profile_table.insert(
                        "zsh",
                        toml_edit::value(indent::indent_all_by(2, profile_zsh)),
                    );
                }
            },
        };

        manifest.insert(MANIFEST_PROFILE_KEY, Item::Table(profile_table));
    }

    /// Populates an example services section.
    fn add_services_section(manifest: &mut DocumentMut) {
        let mut services_table = Table::new();

        services_table.decor_mut().set_prefix(indoc! {r#"


                ## Services ---------------------------------------------------------
                ##  $ flox services start             <- Starts all services
                ##  $ flox services status            <- Status of running services
                ##  $ flox activate --start-services  <- Activates & starts all
                ## ------------------------------------------------------------------
            "#});

        services_table.decor_mut().set_suffix(indoc! {r#"

                # myservice.command = "python3 -m http.server""#});

        manifest.insert(MANIFEST_SERVICES_KEY, Item::Table(services_table));
    }

    /// Populates an example build section.
    fn add_build_section(manifest: &mut DocumentMut) {
        let mut build_table = Table::new();

        build_table.decor_mut().set_prefix(indoc! {r#"


                 ## Build and publish your own packages ------------------------------
                 ##  $ flox build
                 ##  $ flox publish
                 ## ------------------------------------------------------------------
            "#});

        build_table.decor_mut().set_suffix(indoc! {r#"

                # [build.myproject]
                # description = "The coolest project ever"
                # version = "0.0.1"
                # command = """
                #   mkdir -p $out/bin
                #   cargo build --release
                #   cp target/release/myproject $out/bin/myproject
                # """"#});

        manifest.insert(MANIFEST_BUILD_KEY, Item::Table(build_table));
    }

    /// Populates an example include section.
    fn add_include_section(manifest: &mut DocumentMut) {
        let mut include_table = Table::new();

        include_table.decor_mut().set_prefix(indoc! {r#"


                 ## Include ----------------------------------------------------------
                 ## ... environments to create a composed environment
                 ## ------------------------------------------------------------------
            "#});

        include_table.decor_mut().set_suffix(indoc! {r#"

                # environments = [
                #     { dir = "../common" }
                # ]"#});

        manifest.insert(MANIFEST_INCLUDE_KEY, Item::Table(include_table));
    }

    /// Populates an example options section.
    fn add_options_section(
        manifest: &mut DocumentMut,
        systems: &[&System],
        customization: &InitCustomization,
    ) {
        let mut options_table = Table::new();

        options_table.decor_mut().set_prefix(indoc! {r#"


            ## Other Environment Options -----------------------------------------
        "#});

        // `systems` array with custom formatting
        let these_systems: HashSet<&String> = HashSet::from_iter(systems.iter().cloned());
        let all_systems = HashSet::from_iter(DEFAULT_SYSTEMS_STR.iter());
        if these_systems != all_systems {
            // If somehow we init with something *other* than the default systems,
            // add those.
            let mut systems_array = Array::new();
            for system in systems {
                let mut item = Value::from(system.to_string());
                item.decor_mut().set_prefix("\n  "); // Indent each item with two spaces
                if Some(system) == systems.last() {
                    item.decor_mut().set_suffix(",\n"); // Add a newline before the first item
                }
                systems_array.push_formatted(item);
            }

            let systems_key = Key::new(MANIFEST_SYSTEMS_KEY);
            options_table.insert(&systems_key, toml_edit::value(systems_array));
            if let Some((mut key, _)) = options_table.get_key_value_mut(&systems_key) {
                key.leaf_decor_mut().set_prefix(indoc! {r#"
                    # Systems that environment is compatible with
                    "#});
            }
        } else {
            // If we init with the default systems, we can omit those.
            options_table.decor_mut().set_suffix(indoc! {r#"

                # Systems that environment is compatible with
                # systems = [
                #   "aarch64-darwin",
                #   "aarch64-linux",
                #   "x86_64-darwin",
                #   "x86_64-linux",
                # ]"#});
        }

        let cuda_detection_key = Key::new("cuda-detection");
        options_table.insert(&cuda_detection_key, toml_edit::value(false));
        if let Some((mut key, _)) = options_table.get_key_value_mut(&cuda_detection_key) {
            key.leaf_decor_mut().set_prefix(indoc! {r#"
            # Uncomment to disable CUDA detection.
            # "#});
        }

        // `options.activate.mode`, only when customized.
        if let Some(activate_mode) = &customization.activate_mode {
            let activate_key = Key::new("activate");
            let mut activate_table = Table::new();

            let mode_key = Key::new("mode");
            activate_table.insert(&mode_key, toml_edit::value(activate_mode.to_string()));
            options_table.insert(&activate_key, Item::Table(activate_table));
        }

        manifest.insert(MANIFEST_OPTIONS_KEY, Item::Table(options_table));
    }
}

#[cfg(test)]
mod tests {
    use flox_core::activate::mode::ActivateMode;
    use flox_manifest::interfaces::ContentsMatch;
    use flox_manifest::raw::CatalogPackage;

    use super::*;

    #[test]
    fn create_documented_manifest_not_customized() {
        let systems = &*DEFAULT_SYSTEMS_STR.iter().collect::<Vec<_>>();
        let customization = InitCustomization {
            ..Default::default()
        };

        let expected_string = indoc! {r#"
            ## Flox Environment Manifest -----------------------------------------
            ##
            ##   _Everything_ you need to know about the _manifest_ is here:
            ##
            ##   https://flox.dev/docs/reference/command-reference/manifest.toml/
            ##
            ## -------------------------------------------------------------------
            # Flox manifest version managed by Flox CLI
            version = 1


            ## Install Packages --------------------------------------------------
            ##  $ flox install gum  <- puts a package in [install] section below
            ##  $ flox search gum   <- search for a package
            ##  $ flox show gum     <- show all versions of a package
            ## -------------------------------------------------------------------
            [install]
            # gum.pkg-path = "gum"
            # gum.version = "^0.14.5"


            ## Environment Variables ---------------------------------------------
            ##  ... available for use in the activated environment
            ##      as well as [hook], [profile] scripts and [services] below.
            ## -------------------------------------------------------------------
            [vars]
            # INTRO_MESSAGE = "It's gettin' Flox in here"


            ## Activation Hook ---------------------------------------------------
            ##  ... run by _bash_ shell when you run 'flox activate'.
            ## -------------------------------------------------------------------
            [hook]
            # on-activate = '''
            #   # -> Set variables, create files and directories
            #   # -> Perform initialization steps, e.g. create a python venv
            #   # -> Useful environment variables:
            #   #      - FLOX_ENV_PROJECT=/home/user/example
            #   #      - FLOX_ENV=/home/user/example/.flox/run
            #   #      - FLOX_ENV_CACHE=/home/user/example/.flox/cache
            # '''


            ## Profile script ----------------------------------------------------
            ## ... sourced by _your shell_ when you run 'flox activate'.
            ## -------------------------------------------------------------------
            [profile]
            # common = '''
            #   gum style \
            #   --foreground 212 --border-foreground 212 --border double \
            #   --align center --width 50 --margin "1 2" --padding "2 4" \
            #     $INTRO_MESSAGE
            # '''
            ## Shell-specific customizations such as setting aliases go here:
            # bash = ...
            # zsh  = ...
            # fish = ...


            ## Services ---------------------------------------------------------
            ##  $ flox services start             <- Starts all services
            ##  $ flox services status            <- Status of running services
            ##  $ flox activate --start-services  <- Activates & starts all
            ## ------------------------------------------------------------------
            [services]
            # myservice.command = "python3 -m http.server"


            ## Include ----------------------------------------------------------
            ## ... environments to create a composed environment
            ## ------------------------------------------------------------------
            [include]
            # environments = [
            #     { dir = "../common" }
            # ]


            ## Build and publish your own packages ------------------------------
            ##  $ flox build
            ##  $ flox publish
            ## ------------------------------------------------------------------
            [build]
            # [build.myproject]
            # description = "The coolest project ever"
            # version = "0.0.1"
            # command = """
            #   mkdir -p $out/bin
            #   cargo build --release
            #   cp target/release/myproject $out/bin/myproject
            # """


            ## Other Environment Options -----------------------------------------
            [options]
            # Systems that environment is compatible with
            # systems = [
            #   "aarch64-darwin",
            #   "aarch64-linux",
            #   "x86_64-darwin",
            #   "x86_64-linux",
            # ]
            # Uncomment to disable CUDA detection.
            # cuda-detection = false
        "#};

        let manifest =
            ManifestInitializer::new_documented(Features::default(), systems, &customization)
                .unwrap();
        assert!(manifest.contents_match(expected_string));
    }

    #[test]
    fn create_documented_manifest_with_packages() {
        let systems = &*DEFAULT_SYSTEMS_STR.iter().collect::<Vec<_>>();
        let customization = InitCustomization {
            packages: Some(vec![CatalogPackage {
                id: "python3".to_string(),
                pkg_path: "python3".to_string(),
                version: Some("3.11.6".to_string()),
                systems: None,
                outputs: None,
            }]),
            ..Default::default()
        };

        let expected_string = indoc! {r#"
            ## Flox Environment Manifest -----------------------------------------
            ##
            ##   _Everything_ you need to know about the _manifest_ is here:
            ##
            ##   https://flox.dev/docs/reference/command-reference/manifest.toml/
            ##
            ## -------------------------------------------------------------------
            # Flox manifest version managed by Flox CLI
            version = 1


            ## Install Packages --------------------------------------------------
            ##  $ flox install gum  <- puts a package in [install] section below
            ##  $ flox search gum   <- search for a package
            ##  $ flox show gum     <- show all versions of a package
            ## -------------------------------------------------------------------
            [install]
            python3.pkg-path = "python3"
            python3.version = "3.11.6"


            ## Environment Variables ---------------------------------------------
            ##  ... available for use in the activated environment
            ##      as well as [hook], [profile] scripts and [services] below.
            ## -------------------------------------------------------------------
            [vars]
            # INTRO_MESSAGE = "It's gettin' Flox in here"


            ## Activation Hook ---------------------------------------------------
            ##  ... run by _bash_ shell when you run 'flox activate'.
            ## -------------------------------------------------------------------
            [hook]
            # on-activate = '''
            #   # -> Set variables, create files and directories
            #   # -> Perform initialization steps, e.g. create a python venv
            #   # -> Useful environment variables:
            #   #      - FLOX_ENV_PROJECT=/home/user/example
            #   #      - FLOX_ENV=/home/user/example/.flox/run
            #   #      - FLOX_ENV_CACHE=/home/user/example/.flox/cache
            # '''


            ## Profile script ----------------------------------------------------
            ## ... sourced by _your shell_ when you run 'flox activate'.
            ## -------------------------------------------------------------------
            [profile]
            # common = '''
            #   gum style \
            #   --foreground 212 --border-foreground 212 --border double \
            #   --align center --width 50 --margin "1 2" --padding "2 4" \
            #     $INTRO_MESSAGE
            # '''
            ## Shell-specific customizations such as setting aliases go here:
            # bash = ...
            # zsh  = ...
            # fish = ...


            ## Services ---------------------------------------------------------
            ##  $ flox services start             <- Starts all services
            ##  $ flox services status            <- Status of running services
            ##  $ flox activate --start-services  <- Activates & starts all
            ## ------------------------------------------------------------------
            [services]
            # myservice.command = "python3 -m http.server"


            ## Include ----------------------------------------------------------
            ## ... environments to create a composed environment
            ## ------------------------------------------------------------------
            [include]
            # environments = [
            #     { dir = "../common" }
            # ]


            ## Build and publish your own packages ------------------------------
            ##  $ flox build
            ##  $ flox publish
            ## ------------------------------------------------------------------
            [build]
            # [build.myproject]
            # description = "The coolest project ever"
            # version = "0.0.1"
            # command = """
            #   mkdir -p $out/bin
            #   cargo build --release
            #   cp target/release/myproject $out/bin/myproject
            # """


            ## Other Environment Options -----------------------------------------
            [options]
            # Systems that environment is compatible with
            # systems = [
            #   "aarch64-darwin",
            #   "aarch64-linux",
            #   "x86_64-darwin",
            #   "x86_64-linux",
            # ]
            # Uncomment to disable CUDA detection.
            # cuda-detection = false
        "#};

        let manifest =
            ManifestInitializer::new_documented(Features::default(), systems, &customization)
                .unwrap();
        assert!(manifest.contents_match(expected_string));
    }

    #[test]
    fn create_documented_manifest_hook() {
        let systems = [&"x86_64-linux".to_string()];
        let customization = InitCustomization {
            hook_on_activate: Some(
                indoc! {r#"
                    # Print something
                    echo "hello world"

                    # Set a environment variable
                    $FOO="bar"
                "#}
                .to_string(),
            ),
            ..Default::default()
        };

        let expected_string = indoc! {r#"
            ## Flox Environment Manifest -----------------------------------------
            ##
            ##   _Everything_ you need to know about the _manifest_ is here:
            ##
            ##   https://flox.dev/docs/reference/command-reference/manifest.toml/
            ##
            ## -------------------------------------------------------------------
            # Flox manifest version managed by Flox CLI
            version = 1


            ## Install Packages --------------------------------------------------
            ##  $ flox install gum  <- puts a package in [install] section below
            ##  $ flox search gum   <- search for a package
            ##  $ flox show gum     <- show all versions of a package
            ## -------------------------------------------------------------------
            [install]
            # gum.pkg-path = "gum"
            # gum.version = "^0.14.5"


            ## Environment Variables ---------------------------------------------
            ##  ... available for use in the activated environment
            ##      as well as [hook], [profile] scripts and [services] below.
            ## -------------------------------------------------------------------
            [vars]
            # INTRO_MESSAGE = "It's gettin' Flox in here"


            ## Activation Hook ---------------------------------------------------
            ##  ... run by _bash_ shell when you run 'flox activate'.
            ## -------------------------------------------------------------------
            [hook]
            on-activate = """
              # Print something
              echo "hello world"

              # Set a environment variable
              $FOO="bar"
            """


            ## Profile script ----------------------------------------------------
            ## ... sourced by _your shell_ when you run 'flox activate'.
            ## -------------------------------------------------------------------
            [profile]
            # common = '''
            #   gum style \
            #   --foreground 212 --border-foreground 212 --border double \
            #   --align center --width 50 --margin "1 2" --padding "2 4" \
            #     $INTRO_MESSAGE
            # '''
            ## Shell-specific customizations such as setting aliases go here:
            # bash = ...
            # zsh  = ...
            # fish = ...


            ## Services ---------------------------------------------------------
            ##  $ flox services start             <- Starts all services
            ##  $ flox services status            <- Status of running services
            ##  $ flox activate --start-services  <- Activates & starts all
            ## ------------------------------------------------------------------
            [services]
            # myservice.command = "python3 -m http.server"


            ## Include ----------------------------------------------------------
            ## ... environments to create a composed environment
            ## ------------------------------------------------------------------
            [include]
            # environments = [
            #     { dir = "../common" }
            # ]


            ## Build and publish your own packages ------------------------------
            ##  $ flox build
            ##  $ flox publish
            ## ------------------------------------------------------------------
            [build]
            # [build.myproject]
            # description = "The coolest project ever"
            # version = "0.0.1"
            # command = """
            #   mkdir -p $out/bin
            #   cargo build --release
            #   cp target/release/myproject $out/bin/myproject
            # """


            ## Other Environment Options -----------------------------------------
            [options]
            # Systems that environment is compatible with
            systems = [
              "x86_64-linux",
            ]
            # Uncomment to disable CUDA detection.
            # cuda-detection = false
        "#};

        let manifest = ManifestInitializer::new_documented(
            Features::default(),
            systems.as_slice(),
            &customization,
        )
        .unwrap();
        assert!(manifest.contents_match(expected_string));
    }

    #[test]
    fn create_documented_profile_script() {
        let systems = [&"x86_64-linux".to_string()];
        let customization = InitCustomization {
            profile_common: Some(
                indoc! { r#"
                    echo "Hello from Flox"
                "#}
                .to_string(),
            ),
            ..Default::default()
        };

        let expected_string = indoc! {r#"
            ## Flox Environment Manifest -----------------------------------------
            ##
            ##   _Everything_ you need to know about the _manifest_ is here:
            ##
            ##   https://flox.dev/docs/reference/command-reference/manifest.toml/
            ##
            ## -------------------------------------------------------------------
            # Flox manifest version managed by Flox CLI
            version = 1


            ## Install Packages --------------------------------------------------
            ##  $ flox install gum  <- puts a package in [install] section below
            ##  $ flox search gum   <- search for a package
            ##  $ flox show gum     <- show all versions of a package
            ## -------------------------------------------------------------------
            [install]
            # gum.pkg-path = "gum"
            # gum.version = "^0.14.5"


            ## Environment Variables ---------------------------------------------
            ##  ... available for use in the activated environment
            ##      as well as [hook], [profile] scripts and [services] below.
            ## -------------------------------------------------------------------
            [vars]
            # INTRO_MESSAGE = "It's gettin' Flox in here"


            ## Activation Hook ---------------------------------------------------
            ##  ... run by _bash_ shell when you run 'flox activate'.
            ## -------------------------------------------------------------------
            [hook]
            # on-activate = '''
            #   # -> Set variables, create files and directories
            #   # -> Perform initialization steps, e.g. create a python venv
            #   # -> Useful environment variables:
            #   #      - FLOX_ENV_PROJECT=/home/user/example
            #   #      - FLOX_ENV=/home/user/example/.flox/run
            #   #      - FLOX_ENV_CACHE=/home/user/example/.flox/cache
            # '''


            ## Profile script ----------------------------------------------------
            ## ... sourced by _your shell_ when you run 'flox activate'.
            ## -------------------------------------------------------------------
            [profile]
            common = """
              echo "Hello from Flox"
            """


            ## Services ---------------------------------------------------------
            ##  $ flox services start             <- Starts all services
            ##  $ flox services status            <- Status of running services
            ##  $ flox activate --start-services  <- Activates & starts all
            ## ------------------------------------------------------------------
            [services]
            # myservice.command = "python3 -m http.server"


            ## Include ----------------------------------------------------------
            ## ... environments to create a composed environment
            ## ------------------------------------------------------------------
            [include]
            # environments = [
            #     { dir = "../common" }
            # ]


            ## Build and publish your own packages ------------------------------
            ##  $ flox build
            ##  $ flox publish
            ## ------------------------------------------------------------------
            [build]
            # [build.myproject]
            # description = "The coolest project ever"
            # version = "0.0.1"
            # command = """
            #   mkdir -p $out/bin
            #   cargo build --release
            #   cp target/release/myproject $out/bin/myproject
            # """


            ## Other Environment Options -----------------------------------------
            [options]
            # Systems that environment is compatible with
            systems = [
              "x86_64-linux",
            ]
            # Uncomment to disable CUDA detection.
            # cuda-detection = false
        "#};

        let manifest = ManifestInitializer::new_documented(
            Features::default(),
            systems.as_slice(),
            &customization,
        )
        .unwrap();
        assert!(manifest.contents_match(expected_string));
    }

    #[test]
    fn create_documented_manifest_with_activate_mode() {
        let systems = [&"x86_64-linux".to_string()];
        let customization = InitCustomization {
            activate_mode: Some(ActivateMode::Run),
            ..Default::default()
        };

        let expected_string = indoc! {r#"
            ## Flox Environment Manifest -----------------------------------------
            ##
            ##   _Everything_ you need to know about the _manifest_ is here:
            ##
            ##   https://flox.dev/docs/reference/command-reference/manifest.toml/
            ##
            ## -------------------------------------------------------------------
            # Flox manifest version managed by Flox CLI
            version = 1


            ## Install Packages --------------------------------------------------
            ##  $ flox install gum  <- puts a package in [install] section below
            ##  $ flox search gum   <- search for a package
            ##  $ flox show gum     <- show all versions of a package
            ## -------------------------------------------------------------------
            [install]
            # gum.pkg-path = "gum"
            # gum.version = "^0.14.5"


            ## Environment Variables ---------------------------------------------
            ##  ... available for use in the activated environment
            ##      as well as [hook], [profile] scripts and [services] below.
            ## -------------------------------------------------------------------
            [vars]
            # INTRO_MESSAGE = "It's gettin' Flox in here"


            ## Activation Hook ---------------------------------------------------
            ##  ... run by _bash_ shell when you run 'flox activate'.
            ## -------------------------------------------------------------------
            [hook]
            # on-activate = '''
            #   # -> Set variables, create files and directories
            #   # -> Perform initialization steps, e.g. create a python venv
            #   # -> Useful environment variables:
            #   #      - FLOX_ENV_PROJECT=/home/user/example
            #   #      - FLOX_ENV=/home/user/example/.flox/run
            #   #      - FLOX_ENV_CACHE=/home/user/example/.flox/cache
            # '''


            ## Profile script ----------------------------------------------------
            ## ... sourced by _your shell_ when you run 'flox activate'.
            ## -------------------------------------------------------------------
            [profile]
            # common = '''
            #   gum style \
            #   --foreground 212 --border-foreground 212 --border double \
            #   --align center --width 50 --margin "1 2" --padding "2 4" \
            #     $INTRO_MESSAGE
            # '''
            ## Shell-specific customizations such as setting aliases go here:
            # bash = ...
            # zsh  = ...
            # fish = ...


            ## Services ---------------------------------------------------------
            ##  $ flox services start             <- Starts all services
            ##  $ flox services status            <- Status of running services
            ##  $ flox activate --start-services  <- Activates & starts all
            ## ------------------------------------------------------------------
            [services]
            # myservice.command = "python3 -m http.server"


            ## Include ----------------------------------------------------------
            ## ... environments to create a composed environment
            ## ------------------------------------------------------------------
            [include]
            # environments = [
            #     { dir = "../common" }
            # ]


            ## Build and publish your own packages ------------------------------
            ##  $ flox build
            ##  $ flox publish
            ## ------------------------------------------------------------------
            [build]
            # [build.myproject]
            # description = "The coolest project ever"
            # version = "0.0.1"
            # command = """
            #   mkdir -p $out/bin
            #   cargo build --release
            #   cp target/release/myproject $out/bin/myproject
            # """


            ## Other Environment Options -----------------------------------------
            [options]
            # Systems that environment is compatible with
            systems = [
              "x86_64-linux",
            ]
            # Uncomment to disable CUDA detection.
            # cuda-detection = false

            [options.activate]
            mode = "run"
        "#};

        let manifest = ManifestInitializer::new_documented(
            Features::default(),
            systems.as_slice(),
            &customization,
        )
        .unwrap();
        assert!(manifest.contents_match(expected_string));
    }
}
