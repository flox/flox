use flox_core::data::System;
use flox_rust_sdk::flox::Features;
use flox_rust_sdk::models::environment::path_environment::InitCustomization;
use indoc::indoc;

pub(crate) trait InitManifest {
    fn new_documented(
        _features: Features,
        systems: &[&System],
        customization: &InitCustomization,
    ) -> toml_edit::DocumentMut;
    fn new_minimal(customization: &InitCustomization) -> toml_edit::DocumentMut;
}

#[cfg(test)]
mod tests {
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

        let manifest = RawManifest::new_documented(Features::default(), systems, &customization);
        assert_eq!(manifest.to_string(), expected_string.to_string());
        manifest.to_typed().expect("should parse as typed");
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

        let manifest = RawManifest::new_documented(Features::default(), systems, &customization);
        assert_eq!(manifest.to_string(), expected_string.to_string());
        manifest.to_typed().expect("should parse as typed");
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

        let manifest =
            RawManifest::new_documented(Features::default(), systems.as_slice(), &customization);
        assert_eq!(manifest.to_string(), expected_string.to_string());
        manifest.to_typed().expect("should parse as typed");
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

        let manifest =
            RawManifest::new_documented(Features::default(), systems.as_slice(), &customization);
        assert_eq!(manifest.to_string(), expected_string.to_string());
        manifest.to_typed().expect("should parse as typed");
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

        let manifest =
            RawManifest::new_documented(Features::default(), systems.as_slice(), &customization);
        assert_eq!(manifest.to_string(), expected_string.to_string());
        manifest.to_typed().expect("should parse as typed");
    }
}
