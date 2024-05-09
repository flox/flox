use std::fs;
use std::path::Path;

use anyhow::{anyhow, Result};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::path_environment::InitCustomization;
use flox_rust_sdk::models::manifest::PackageToInstall;
use indoc::{formatdoc, indoc};

use super::{
    format_customization,
    try_find_compatible_version,
    InitHook,
    ProvidedPackage,
    ProvidedVersion,
    AUTO_SETUP_HINT,
};
use crate::utils::dialog::{Dialog, Select};
use crate::utils::message;

const GO_MOD_FILENAME: &str = "go.mod";
const GO_WORK_FILENAME: &str = "go.work";

const GO_HOOK: &str = indoc! {"
    # Point GOENV to Flox environment cache
    export GOENV=\"$FLOX_ENV_CACHE/goenv\"

    # Install Go dependencies
    go get ."
};

/// The Go hook handles installation and configuration suggestions for projects using Go.
/// The general flow of the Go hook is:
///
/// - [Self::new]: Detects files of type [GoModuleSystemKind] in the current working directory.
/// - [Self::prompt_user]: Prints the customization from [Self::get_init_customization]
///   if user commands to do so. Else, return `true` or `false` based on whether
///   the user desires or not the presented customization.
/// - [Self::get_init_customization]: Returns a Go specific customization based on [Self::module_system].
#[derive(Debug, Clone)]
pub(super) struct Go {
    /// Stores the version required to generate a customization with [Self::get_init_customization].
    /// Becomes initialized in [Self::new].
    module_system: GoModuleSystemKind,
}

impl Go {
    /// Creates and returns the [Go] hook with the detected [GoModuleSystemKind] module system.
    /// If no module system is detected, returns [None].
    pub async fn new(flox: &Flox, path: &Path) -> Result<Option<Self>> {
        Ok(Self::detect_module_system(flox, path)
            .await?
            .map(|module_system| Self { module_system }))
    }

    /// Determines which [GoModuleSystemKind] is being used.
    /// Since the [GO_WORK_FILENAME] file declares a multiple module based workspace, it takes
    /// precedence over any other [GO_MOD_FILENAME] file that could possibly be found.
    async fn detect_module_system(flox: &Flox, path: &Path) -> Result<Option<GoModuleSystemKind>> {
        if let Some(go_work) = GoWorkSystem::try_new_from_path(flox, path).await? {
            return Ok(Some(GoModuleSystemKind::Workspace(go_work)));
        }

        if let Some(go_mod) = GoModSystem::try_new_from_path(flox, path).await? {
            return Ok(Some(GoModuleSystemKind::Module(go_mod)));
        }

        Ok(None)
    }
}

impl InitHook for Go {
    /// Returns `true` if the user accepts the prompt. In that case,
    /// the hook customizes the manifest with the default Go environment.
    async fn prompt_user(&mut self, _flox: &Flox, _path: &Path) -> Result<bool> {
        let module_system = self.module_system.get_system();

        message::plain(formatdoc! {"
            Flox detected a {} file in the current directory.

            Go projects typically need:
            * Go
            * A shell hook to apply environment variables

        ", module_system.get_filename()});

        let message = formatdoc! {"
        Would you like Flox to apply the standard Go environment?
        You can always revisit the environment's declaration with 'flox edit'"};

        let accept_options = ["Yes".to_string()];
        let accept_options_offset = accept_options.len();
        let cancel_options = ["No".to_string()];
        let cancel_options_offset = accept_options_offset + cancel_options.len();

        let show_environment_manifest_option = ["Show environment manifest".to_string()];

        let options = accept_options
            .iter()
            .chain(cancel_options.iter())
            .chain(show_environment_manifest_option.iter())
            .collect::<Vec<_>>();

        let n_options = options.len();

        loop {
            let dialog = Dialog {
                message: &message,
                help_message: Some(AUTO_SETUP_HINT),
                typed: Select {
                    options: options.clone(),
                },
            };

            let (choice, _) = dialog.raw_prompt()?;

            match choice {
                accept if accept < accept_options_offset => return Ok(true),
                cancel if cancel < cancel_options_offset => return Ok(false),
                show_environment if show_environment < n_options => {
                    message::plain(format_customization(&self.get_init_customization())?);
                },
                _ => unreachable!("Option selection is out of valid option bounds"),
            }
        }
    }

    /// Returns an [InitCustomization] with the customization associated to the Go
    /// module system detected.
    fn get_init_customization(&self) -> InitCustomization {
        let go_version = match self.module_system.get_system().get_version() {
            ProvidedVersion::Compatible { requested, .. } => requested,
            ProvidedVersion::Incompatible { .. } => unreachable!(
                "The Go hook should not be running if the requested module system \
                    version is incompatible"
            ),
        };

        InitCustomization {
            hook_on_activate: Some(GO_HOOK.to_string()),
            profile_common: None,
            profile_bash: None,
            profile_zsh: None,
            packages: Some(vec![PackageToInstall {
                id: "go".to_string(),
                pkg_path: "go".to_string(),
                version: go_version,
                input: None,
            }]),
        }
    }
}

/// Represents Go module system files.
#[derive(Debug, Clone, PartialEq)]
enum GoModuleSystemKind {
    /// Single module based system [GoModSystem].
    Module(GoModSystem),
    /// Workspace system [GoWorkSystem].
    Workspace(GoWorkSystem),
}

impl GoModuleSystemKind {
    /// Resolves the enum to any of the contained Go module systems.
    fn get_system(&self) -> &dyn GoModuleSystemMode {
        match self {
            GoModuleSystemKind::Workspace(workspace) => workspace,
            GoModuleSystemKind::Module(module) => module,
        }
    }
}

/// Represents the common functionality between Module and Workspace system modes.
trait GoModuleSystemMode {
    /// Returns the possible instance of a Go module or workspace system,
    /// from the content of a module or workspace file, respectively.
    /// This method should return `true` when there isn't any valid `go` versioning
    /// statements inside the module or workspace content.
    async fn try_new_from_content(flox: &Flox, module_content: &str) -> Result<Option<Self>>
    where
        Self: Sized;

    /// Detects and returns the possible instance of a Go module or workspace system
    /// from a given filesystem path. If the detected system inside is a directory,
    /// it must be rejected and return `None`.
    async fn try_new_from_path(flox: &Flox, path: &Path) -> Result<Option<Self>>
    where
        Self: Sized;

    /// Returns the filename of the module system mode. It can either be `go.mod`
    /// (for single module systems) or `go.work` (for multi-module workspace systems).
    fn get_filename(&self) -> &'static str;

    /// Returns the provided version obtained from the module system file.
    fn get_version(&self) -> ProvidedVersion;
}

/// Represents the single-module system from the content of `go.mod` files.
#[derive(Debug, Clone, PartialEq)]
struct GoModSystem {
    /// Represents the version obtained from the `go` statement inside the `go.mod` file.
    version: ProvidedVersion,
}

/// Represents the functionality for the single-module system mode.
impl GoModuleSystemMode for GoModSystem {
    /// Returns the possible instance of a Go module system, from the content
    /// of a module file.
    /// This method should return `true` when there isn't any valid `go` versioning
    /// statements inside the module content.
    async fn try_new_from_content(flox: &Flox, module_content: &str) -> Result<Option<Self>> {
        match GoVersion::from_content(flox, module_content).await? {
            Some(version) => Ok(Some(Self { version })),
            None => Ok(None),
        }
    }

    /// This method returns `None` if [GO_MOD_FILENAME] is a directory.
    async fn try_new_from_path(flox: &Flox, path: &Path) -> Result<Option<Self>> {
        let mod_path = path.join(GO_MOD_FILENAME);
        if !mod_path.exists() || mod_path.is_dir() {
            return Ok(None);
        }

        let mod_content = fs::read_to_string(mod_path)?;
        Self::try_new_from_content(flox, &mod_content).await
    }

    #[inline(always)]
    fn get_filename(&self) -> &'static str {
        GO_MOD_FILENAME
    }

    fn get_version(&self) -> ProvidedVersion {
        self.version.clone()
    }
}

/// Represents the multi-module workspace system from the content of `go.work` files.
#[derive(Debug, Clone, PartialEq)]
struct GoWorkSystem {
    /// Represents the version obtained from the `go` statement inside the `go.work` file.
    version: ProvidedVersion,
}

/// Represents the functionality for the multi-module workspace mode.
impl GoModuleSystemMode for GoWorkSystem {
    async fn try_new_from_content(flox: &Flox, workspace_content: &str) -> Result<Option<Self>> {
        match GoVersion::from_content(flox, workspace_content).await? {
            Some(version) => Ok(Some(Self { version })),
            None => Ok(None),
        }
    }

    /// This method returns `None` if [GO_WORK_FILENAME] is a directory.
    async fn try_new_from_path(flox: &Flox, path: &Path) -> Result<Option<Self>> {
        let work_path = path.join(GO_WORK_FILENAME);
        if !work_path.exists() || work_path.is_dir() {
            return Ok(None);
        }

        let work_content = fs::read_to_string(work_path)?;
        Self::try_new_from_content(flox, &work_content).await
    }

    #[inline(always)]
    fn get_filename(&self) -> &'static str {
        GO_WORK_FILENAME
    }

    fn get_version(&self) -> ProvidedVersion {
        self.version.clone()
    }
}

/// Represents a scoped implementation of version related functionality that
/// parses and encapsulate raw versions into [ProvidedVersion] objects.
struct GoVersion;

impl GoVersion {
    /// Returns the version contained in the content of a Go module system file
    /// as a [ProvidedVersion].
    async fn from_content(flox: &Flox, content: &str) -> Result<Option<ProvidedVersion>> {
        let Some(required_go_version) = Self::parse_content_version_string(content)? else {
            return Ok(None);
        };

        let provided_go_version = try_find_compatible_version(
            flox,
            "go",
            required_go_version.as_ref(),
            None::<Vec<String>>,
        )
        .await?;

        if let Some(found_go_version) = provided_go_version {
            let found_go_version = TryInto::<ProvidedPackage>::try_into(found_go_version)?;

            return Ok(Some(ProvidedVersion::Compatible {
                requested: Some(required_go_version),
                compatible: found_go_version,
            }));
        }

        // Returning this means that the version is incompatible
        Ok(None)
    }

    /// Parses the content of a Go module system file and returns the version as a [String].
    ///
    /// NOTE: future major releases of Go (e.g. go 2.x.y) are not contemplated in this code,
    /// but would be satisfied by a previous major release.
    /// See: https://github.com/flox/flox/pull/1227#discussion_r1548737251
    fn parse_content_version_string(content: &str) -> Result<Option<String>> {
        content
            .lines()
            .find(|line| line.trim_start().starts_with("go"))
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|version| {
                version
                    .parse::<semver::VersionReq>()
                    .map_err(|err| anyhow!(err))
                    .map(|semver| Some(semver.to_string()))
                    .into()
            })
            .unwrap_or(Err(anyhow!("Flox found an invalid Go module system file")))
    }
}

#[cfg(test)]
mod tests {
    use flox_rust_sdk::data::System;
    use flox_rust_sdk::flox::test_helpers::{
        flox_instance_with_global_lock,
        flox_instance_with_optional_floxhub_and_client,
    };
    use flox_rust_sdk::providers::catalog::Client;

    use super::*;
    use crate::commands::init::tests::resolved_pkg_group_with_dummy_package;
    use crate::commands::init::ProvidedPackage;

    #[tokio::test]
    async fn test_go_mod_system_returns_none_if_gomod_is_dir() {
        let (flox, temp_dir_handle) = flox_instance_with_global_lock();
        std::fs::create_dir_all(temp_dir_handle.path().join("go.mod/")).unwrap();

        let module_system = GoModSystem::try_new_from_path(&flox, temp_dir_handle.path())
            .await
            .unwrap();
        assert!(module_system.is_none());
    }

    #[tokio::test]
    async fn test_go_work_system_returns_none_if_gowork_is_dir() {
        let (flox, temp_dir_handle) = flox_instance_with_global_lock();
        std::fs::create_dir_all(temp_dir_handle.path().join("go.work/")).unwrap();

        let module_system = GoWorkSystem::try_new_from_path(&flox, temp_dir_handle.path())
            .await
            .unwrap();
        assert!(module_system.is_none());
    }

    #[tokio::test]
    async fn test_go_version_from_content_returns_compatible_version() {
        let (flox, _temp_dir_handle) = flox_instance_with_global_lock();
        let content = indoc! {r#"
                // valid go version
                go 1.21.4
            "#};

        let version = GoVersion::from_content(&flox, content)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(version, ProvidedVersion::Compatible {
            requested: Some("^1.21.4".to_string()),
            compatible: ProvidedPackage::new("go", vec!["go"], "1.21.4")
        });
    }

    #[tokio::test]
    async fn test_go_version_from_content_returns_none_on_incompatible_version() {
        let (flox, _temp_dir_handle) = flox_instance_with_global_lock();
        let content = indoc! {r#"
                // incompatible go version
                go 0.0.0
            "#};

        let version = GoVersion::from_content(&flox, content).await.unwrap();

        assert_eq!(version, None);
    }

    #[tokio::test]
    async fn test_go_version_from_content_returns_error_on_invalid_version() {
        let (flox, _temp_dir_handle) = flox_instance_with_global_lock();
        let content = indoc! {r#"
                // invalid go version
                go invalid
            "#};

        let version = GoVersion::from_content(&flox, content).await;

        assert!(version.is_err());
    }

    #[test]
    fn test_go_version_string_parsing_succeeds_with_valid_version() {
        let content = indoc! {r#"
                // valid go version
                go 1.21.0
            "#};

        let version = GoVersion::parse_content_version_string(content)
            .unwrap()
            .unwrap();

        assert_eq!(version, "^1.21.0");
    }

    #[test]
    fn test_go_version_string_parsing_fails_with_invalid_version() {
        let content = indoc! {r#"
                // invalid go version
                go invalid
            "#};

        let version = GoVersion::parse_content_version_string(content);

        assert!(version.is_err());
    }

    #[tokio::test]
    async fn test_go_mod_system_returns_none_if_gomod_is_dir_catalog() {
        let (flox, temp_dir_handle) = flox_instance_with_optional_floxhub_and_client(None, true);

        std::fs::create_dir_all(temp_dir_handle.path().join("go.mod/")).unwrap();

        let module_system = GoModSystem::try_new_from_path(&flox, temp_dir_handle.path())
            .await
            .unwrap();
        assert!(module_system.is_none());
    }

    #[tokio::test]
    async fn test_go_work_system_returns_none_if_gowork_is_dir_catalog() {
        let (flox, temp_dir_handle) = flox_instance_with_optional_floxhub_and_client(None, true);

        std::fs::create_dir_all(temp_dir_handle.path().join("go.work/")).unwrap();

        let module_system = GoWorkSystem::try_new_from_path(&flox, temp_dir_handle.path())
            .await
            .unwrap();
        assert!(module_system.is_none());
    }

    #[tokio::test]
    async fn test_go_version_from_content_returns_compatible_version_catalog() {
        let (mut flox, _temp_dir_handle) =
            flox_instance_with_optional_floxhub_and_client(None, true);

        if let Some(Client::Mock(ref mut client)) = flox.catalog_client {
            // Response for go 1.21.4
            client.push_resolve_response(vec![resolved_pkg_group_with_dummy_package(
                "go_group",
                &System::from("aarch64-darwin"),
                "go",
                "go",
                "1.21.4",
            )]);
        }

        let content = indoc! {r#"
                // valid go version
                go 1.21.4
            "#};

        let version = GoVersion::from_content(&flox, content)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(version, ProvidedVersion::Compatible {
            requested: Some("^1.21.4".to_string()),
            compatible: ProvidedPackage::new("go", vec!["go"], "1.21.4")
        });
    }

    #[tokio::test]
    async fn test_go_version_from_content_returns_none_on_incompatible_version_catalog() {
        let (mut flox, _temp_dir_handle) =
            flox_instance_with_optional_floxhub_and_client(None, true);

        if let Some(Client::Mock(ref mut client)) = flox.catalog_client {
            // Response for incompatible go version
            client.push_resolve_response(vec![]);
        }
        let content = indoc! {r#"
                // incompatible go version
                go 0.0.0
            "#};

        let version = GoVersion::from_content(&flox, content).await.unwrap();

        assert_eq!(version, None);
    }
}
