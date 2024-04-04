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

    # Install Go depedencies
    go get ."
};

/// The Go hook handles installation and configuration suggestions for projects using Go.
/// The general flow of the Go hook is:
///
/// - [Self::new]: Detects files of type [GoModuleSystemKind] in the current working directory.
/// - [Self::should_run]: Returns whether a valid module system containing a compatible version
///   was detected in the current working directory, i.e. `false` if the [Self::module_system]
///   is [None], else returns `true`.
/// - [Self::prompt_user]: Prints the customization from [Self::get_init_customization]
///   if user commands to do so. Else, return `true` or `false` based on whether
///   the user desires or not the presented customization.
/// - [Self::get_init_customization]: Returns a Go specific customization based on [Self::module_system].
pub(super) struct Go {
    /// Stores the version required to generate a customization with [Self::get_init_customization].
    /// Becomes initialized in [Self::new].
    module_system: Option<GoModuleSystemKind>,
}

impl Go {
    /// Creates and returns the Go hook with the detected module system.
    pub fn new(path: &Path, flox: &Flox) -> Result<Self> {
        let module_system = Self::detect_module_system(flox, path)?;

        Ok(Self { module_system })
    }

    /// Determines which [GoModuleSystemKind] is being used.
    /// Since the [GO_WORK_FILENAME] file declares a multiple module based workspace, it takes
    /// precedence over any other [GO_MOD_FILENAME] file that could possibly be found.
    fn detect_module_system(flox: &Flox, path: &Path) -> Result<Option<GoModuleSystemKind>> {
        if let Some(go_work) = GoWorkSystem::try_new_from_path(flox, path)? {
            return Ok(Some(GoModuleSystemKind::Workspace(go_work)));
        }

        if let Some(go_mod) = GoModSystem::try_new_from_path(flox, path)? {
            return Ok(Some(GoModuleSystemKind::Module(go_mod)));
        }

        Ok(None)
    }
}

impl InitHook for Go {
    /// Returns `true` if any valid module system file was found.
    ///
    /// [Self::prompt_user] and [Self::get_init_customization]
    /// are expected to be called only if this method returns `true`!
    fn should_run(&mut self, _path: &Path) -> Result<bool> {
        Ok(self.module_system.is_some())
    }

    /// Returns `true` if the user accepts the prompt. In that case,
    /// the hook customizes the manifest with the default Go environment.
    fn prompt_user(&mut self, _path: &Path, _flox: &Flox) -> Result<bool> {
        let module_system = &mut self
            .module_system
            .as_ref()
            .map(|module_system| module_system.get_system())
            .unwrap_or_else(|| {
                unreachable!(
                    "called `prompt_user` without `should_run` called or \
                        having returned `false`"
                )
            });

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
        let go_version = self
            .module_system
            .as_ref()
            .map(|sys| sys.get_system())
            .and_then(|system| match system.get_version() {
                ProvidedVersion::Compatible { requested, .. } => requested,
                ProvidedVersion::Incompatible { .. } => unreachable!(
                    "The Go hook should not be running if the requested module system \
                    version is incompatible"
                ),
            });

        InitCustomization {
            profile: Some(GO_HOOK.to_string()),
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
#[derive(PartialEq)]
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
    fn try_new_from_content(flox: &Flox, module_content: &str) -> Result<Option<Self>>
    where
        Self: Sized;

    /// Detects and returns the possible instance of a Go module or workspace system
    /// from a given filesystem path. If the detected system inside is a directory,
    /// it must be rejected and return `None`.
    fn try_new_from_path(flox: &Flox, path: &Path) -> Result<Option<Self>>
    where
        Self: Sized;

    /// Returns the filename of the module system mode. It can either be `go.mod`
    /// (for single module systems) or `go.work` (for multi-module workspace systems).
    fn get_filename(&self) -> &'static str;

    /// Returns the provided version obtained from the module system file.
    fn get_version(&self) -> ProvidedVersion;
}

/// Represents the single-module system from the content of `go.mod` files.
#[derive(PartialEq)]
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
    fn try_new_from_content(flox: &Flox, module_content: &str) -> Result<Option<Self>> {
        match GoVersion::from_content(flox, module_content)? {
            Some(version) => Ok(Some(Self { version })),
            None => Ok(None),
        }
    }

    /// This method returns `None` if [GO_MOD_FILENAME] is a directory.
    fn try_new_from_path(flox: &Flox, path: &Path) -> Result<Option<Self>> {
        let mod_path = path.join(GO_MOD_FILENAME);
        if !mod_path.exists() {
            return Ok(None);
        }

        let mod_content = fs::read_to_string(mod_path)?;
        Self::try_new_from_content(flox, &mod_content)
    }

    #[inline(always)]
    fn get_filename(&self) -> &'static str {
        GO_MOD_FILENAME.into()
    }

    fn get_version(&self) -> ProvidedVersion {
        self.version.clone()
    }
}

/// Represents the multi-module workspace system from the content of `go.work` files.
#[derive(PartialEq)]
struct GoWorkSystem {
    /// Represents the version obtained from the `go` statement inside the `go.work` file.
    version: ProvidedVersion,
}

/// Represents the functionality for the multi-module workspace mode.
impl GoModuleSystemMode for GoWorkSystem {
    fn try_new_from_content(flox: &Flox, workspace_content: &str) -> Result<Option<Self>> {
        match GoVersion::from_content(flox, workspace_content)? {
            Some(version) => Ok(Some(Self { version })),
            None => Ok(None),
        }
    }

    /// This method returns `None` if [GO_WORK_FILENAME] is a directory.
    fn try_new_from_path(flox: &Flox, path: &Path) -> Result<Option<Self>> {
        let work_path = path.join(GO_WORK_FILENAME);
        if !work_path.exists() || work_path.is_dir() {
            return Ok(None);
        }

        let work_content = fs::read_to_string(work_path)?;
        Self::try_new_from_content(flox, &work_content)
    }

    #[inline(always)]
    fn get_filename(&self) -> &'static str {
        GO_WORK_FILENAME.into()
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
    fn from_content(flox: &Flox, content: &str) -> Result<Option<ProvidedVersion>> {
        let Some(required_go_version) = Self::parse_content_version_string(content)? else {
            return Ok(None);
        };

        let provided_go_version =
            try_find_compatible_version("go", Some(&required_go_version), None::<Vec<&str>>, flox)?;

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
    fn parse_content_version_string(content: &str) -> Result<Option<String>> {
        content
            .lines()
            .skip_while(|line| !line.trim_start().starts_with("go"))
            .next()
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
    use flox_rust_sdk::flox::test_helpers::flox_instance_with_global_lock;

    use super::*;
    use crate::commands::init::ProvidedPackage;

    #[test]
    fn test_should_run_returns_true_on_valid_module() {
        let mut go = Go {
            module_system: Some(GoModuleSystemKind::Module(GoModSystem {
                version: ProvidedVersion::Compatible {
                    requested: None,
                    compatible: ProvidedPackage::new("go", vec!["go"], "1.22.1"),
                },
            })),
        };
        assert!(go.should_run(Path::new("")).unwrap());
    }

    #[test]
    fn test_should_run_returns_true_on_valid_workspace() {
        let mut go = Go {
            module_system: Some(GoModuleSystemKind::Workspace(GoWorkSystem {
                version: ProvidedVersion::Compatible {
                    requested: None,
                    compatible: ProvidedPackage::new("go", vec!["go"], "1.22.1"),
                },
            })),
        };
        assert!(go.should_run(Path::new("")).unwrap());
    }

    #[test]
    fn test_should_run_returns_false_on_none_system() {
        let mut go = Go {
            module_system: None,
        };
        assert!(!go.should_run(Path::new("")).unwrap());
    }

    #[test]
    fn test_go_version_from_content_returns_compatible_version() {
        let (flox, _temp_dir_handle) = flox_instance_with_global_lock();
        let content = indoc! {r#"
                // valid go version
                go 1.21.4
            "#};

        let version = GoVersion::from_content(&flox, content).unwrap().unwrap();

        assert_eq!(version, ProvidedVersion::Compatible {
            requested: Some("^1.21.4".to_string()),
            compatible: ProvidedPackage::new("go", vec!["go"], "1.21.4")
        });
    }

    #[test]
    fn test_go_version_from_content_returns_none_on_incompatible_version() {
        let (flox, _temp_dir_handle) = flox_instance_with_global_lock();
        let content = indoc! {r#"
                // incompatible go version
                go 0.0.0
            "#};

        let version = GoVersion::from_content(&flox, content).unwrap();

        assert_eq!(version, None);
    }

    #[test]
    fn test_go_version_from_content_returns_error_on_invalid_version() {
        let (flox, _temp_dir_handle) = flox_instance_with_global_lock();
        let content = indoc! {r#"
                // invalid go version
                go invalid
            "#};

        let version = GoVersion::from_content(&flox, content);

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
}
