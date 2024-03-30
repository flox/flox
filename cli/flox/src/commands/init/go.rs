use std::borrow::Cow;
use std::fs;
use std::path::Path;

use anyhow::{anyhow, Error, Result};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::path_environment::InitCustomization;
use flox_rust_sdk::models::manifest::PackageToInstall;
use indoc::{formatdoc, indoc};

use super::{format_customization, InitHook, ProvidedPackage, ProvidedVersion, AUTO_SETUP_HINT};
use crate::utils::dialog::{Dialog, Select};
use crate::utils::message;

const GO_MOD_FILENAME: &str = "go.mod";
const GO_WORK_FILENAME: &str = "go.work";

const GO_HOOK: &str = indoc! {"
    # Install Go depedencies
    go get ."
};

/// The Go hook handles installation and configuration suggestions for projects using Go.
/// The general flow of the Go hook is:
///
/// - [Self::new]: Detects [GoModuleSystem] files in the current working directory.
/// - [Self::should_run]: Returns whether a valid module system was detected
///   in the current working directory, i.e. `false` if the [Self::module_system]
///   is [GoModuleSystemKind::None], else returns `true`.
/// - [Self::prompt_user]: Prints the customization from [Self::get_init_customization]
///   if user commands to do so. Else, return true or false based on whether
///   the user wants the customization.
/// - [Self::get_init_customization]: Returns a customization based on [Self::module_system].
pub(super) struct Go {
    /// Stores the customization values required to generate a customization with
    /// [Self::get_init_customization].
    /// Is initialized in [Self::new].
    module_system: GoModuleSystemKind,
}

impl Go {
    /// Creates and returns the Go hook with the detected module system.
    pub fn new(path: &Path, _flox: &Flox) -> Result<Self> {
        let module_system = Self::detect_module_system(path)?;

        Ok(Self { module_system })
    }

    /// Determines which [GoModuleSystemKind] is being used.
    /// Since the [GO_WORK_FILENAME] file declares a multiple module based workspace, it takes
    /// precedence over any other [GO_MOD_FILENAME] file that could possibly be found.
    fn detect_module_system(path: &Path) -> Result<GoModuleSystemKind> {
        if let Some(go_work) = GoWorkspaceSystem::try_new_from_path(path)? {
            return Ok(GoModuleSystemKind::Workspace(go_work));
        }

        if let Some(go_mod) = GoModuleSystem::try_new_from_path(path)? {
            return Ok(GoModuleSystemKind::Module(go_mod));
        }

        Ok(GoModuleSystemKind::None)
    }
}

impl InitHook for Go {
    /// Returns `true` if any valid module system file was found.
    ///
    /// [Self::prompt_user] and [Self::get_init_customization]
    /// are expected to be called only if this method returns `true`!
    fn should_run(&mut self, _path: &Path) -> Result<bool> {
        Ok(self.module_system == GoModuleSystemKind::None)
    }

    fn prompt_user(&mut self, _path: &Path, _flox: &Flox) -> Result<bool> {
        let Some(module_system) = self.module_system.get_system() else {
            return Ok(false);
        };

        message::plain(formatdoc! {"
            Flox detected a {} file in the current directory.

            Go projects typically need:
            * Go
            * A shell hook to apply environment variables\n
        ", module_system.get_filename()});

        let message = formatdoc! {"
        Would you like Flox to apply the standard Go environment? 
        You can always revisit the environment's declaration with 'flox edit'"};

        let accept_options = ["Yes".to_string()];
        let accept_options_offset = accept_options.len() - 1;
        let cancel_options = ["No".to_string()];
        let cancel_options_offset = accept_options_offset + cancel_options.len() - 1;

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
                accept if accept <= accept_options_offset => return Ok(true),
                cancel if cancel <= cancel_options_offset => return Ok(false),
                show_environment if show_environment < n_options => {
                    message::plain(format_customization(todo!(
                        "self.module_system.get_init_customization()"
                    ))?);
                },
                _ => unreachable!(),
            }
        }
    }

    fn get_init_customization(&self) -> InitCustomization {
        let package = PackageToInstall {
            id: "go".to_string(),
            pkg_path: "".to_string(),
            version: Some("".to_string()),
            input: None,
        };

        let profile = Some(GO_HOOK.to_string());

        InitCustomization {
            profile,
            packages: Some(vec![package]),
        }
    }
}

/// Represents Go module system files.
#[derive(PartialEq)]
enum GoModuleSystemKind {
    /// Not a Go module system, or just nothing at all.
    None,
    /// Single module based system [GoModuleSystem].
    Module(GoModuleSystem),
    /// Workspace system [GoWorkspaceSystem].
    Workspace(GoWorkspaceSystem),
}

impl GoModuleSystemKind {
    fn get_system(&self) -> Option<&dyn GoModuleSystemMode> {
        match self {
            GoModuleSystemKind::Workspace(workspace) => Some(workspace),
            GoModuleSystemKind::Module(module) => Some(module),
            GoModuleSystemKind::None => None,
        }
    }
}

trait GoModuleSystemMode {
    fn try_new_from_contents(module_contents: String) -> Option<Self>
    where
        Self: Sized;
    fn try_new_from_path(path: &Path) -> Result<Option<Self>>
    where
        Self: Sized;

    fn get_filename(&self) -> Cow<'static, str>;
    fn get_version(&self) -> ProvidedVersion;
}

#[derive(PartialEq)]
struct GoModuleSystem {
    version: ProvidedVersion,
}

impl GoModuleSystemMode for GoModuleSystem {
    fn try_new_from_contents(module_contents: String) -> Option<Self> {
        let Some(version) = ProvidedVersion::from_module_system_contents(module_contents) else {
            return None;
        };

        Some(Self { version })
    }

    fn try_new_from_path(path: &Path) -> Result<Option<Self>> {
        let mod_path = path.join(GO_MOD_FILENAME);
        if !mod_path.exists() {
            return Ok(None);
        }

        let mod_contents = fs::read_to_string(mod_path)?;
        let go_module = Self::try_new_from_contents(mod_contents);
        Ok(go_module)
    }

    #[inline(always)]
    fn get_filename(&self) -> Cow<'static, str> {
        GO_MOD_FILENAME.into()
    }

    fn get_version(&self) -> ProvidedVersion {
        self.version.clone()
    }
}

#[derive(PartialEq)]
struct GoWorkspaceSystem {
    version: ProvidedVersion,
}

impl GoModuleSystemMode for GoWorkspaceSystem {
    fn try_new_from_contents(workspace_contents: String) -> Option<Self> {
        let Some(version) = ProvidedVersion::from_module_system_contents(workspace_contents) else {
            return None;
        };
        Some(Self { version })
    }

    /// Go commands ignore directories called [GO_WORK_FILENAME].
    fn try_new_from_path(path: &Path) -> Result<Option<Self>> {
        let work_path = path.join(GO_WORK_FILENAME);
        if !work_path.exists() || work_path.is_dir() {
            return Ok(None);
        }

        let work_contents = fs::read_to_string(work_path)?;
        let go_workspace = Self::try_new_from_contents(work_contents);
        Ok(go_workspace)
    }

    #[inline(always)]
    fn get_filename(&self) -> Cow<'static, str> {
        GO_WORK_FILENAME.into()
    }

    fn get_version(&self) -> ProvidedVersion {
        self.version.clone()
    }
}

impl ProvidedVersion {
    fn from_module_system_contents(contents: String) -> Option<Self> {
        let Some(version_str) = ProvidedVersion::get_package_from_contents(&contents) else {
            return None;
        };

        Some(Self::Compatible {
            requested: todo!(),
            compatible: todo!(),
        })
    }

    fn get_package_from_contents<'a>(contents: &'a String) -> Option<&'a str> {
        contents
            .lines()
            .skip_while(|line| (**line).trim_start().starts_with("go"))
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::init::ProvidedPackage;

    #[test]
    fn test_should_run_returns_true_on_valid_module() {
        let mut go = Go {
            module_system: GoModuleSystemKind::Module(GoModuleSystem {
                version: ProvidedVersion::Compatible {
                    requested: None,
                    compatible: ProvidedPackage::new("go", vec!["go"], "1.21.0"),
                },
            }),
        };
        assert!(go.should_run(Path::new("")).unwrap());
    }

    #[test]
    fn test_should_run_returns_true_on_valid_workspace() {
        let mut go = Go {
            module_system: GoModuleSystemKind::Workspace(GoWorkspaceSystem { version: todo!() }),
        };
        assert!(go.should_run(Path::new("")).unwrap());
    }

    #[test]
    fn test_should_run_returns_false_on_none_system() {
        let mut go = Go {
            module_system: GoModuleSystemKind::None,
        };
        assert!(!go.should_run(Path::new("")).unwrap());
    }

    #[test]
    fn test_should_run_returns_false_on_invalid_system() {
        let mut go = Go {
            module_system: GoModuleSystemKind::Module(GoModuleSystem { version: todo!() }),
        };
        assert!(!go.should_run(Path::new("")).unwrap());
    }

    /*
    #[test]
    fn test_pyproject_invalid() {
        let (flox, _) = &*FLOX_INSTANCE;

        let content = indoc! {r#"
        ,
        "#};

        let pyproject = PyProject::from_pyproject_content(content, flox);

        assert!(pyproject.is_err());
    }
    #[test]
    #[serial]
    fn test_pyproject_empty() {
        let (flox, _) = &*FLOX_INSTANCE;

        let pyproject = PyProject::from_pyproject_content("", flox).unwrap();

        assert_eq!(pyproject.unwrap(), PyProject {
            provided_python_version: ProvidedVersion::Compatible {
                requested: None,
                compatible: ProvidedPackage::new("python3", vec!["python3"], "3.11.6"),
            },
        });
    }

    /// ProvidedVersion::Compatible should be returned for requires-python = ">=3.8"
    #[test]
    #[serial]
    fn test_pyproject_available_version() {
        let (flox, _) = &*FLOX_INSTANCE;

        let content = indoc! {r#"
        [project]
        requires-python = ">= 3.8"
        "#};

        let pyproject = PyProject::from_pyproject_content(content, flox).unwrap();

        assert_eq!(pyproject.unwrap(), PyProject {
            provided_python_version: ProvidedVersion::Compatible {
                requested: Some(">=3.8".to_string()),
                compatible: ProvidedPackage::new("python3", vec!["python39"], "3.9.18"),
            },
        });
    }

    /// ProvidedVersion::Incompatible should be returned for requires-python = "1"
    #[test]
    #[serial]
    fn test_pyproject_unavailable_version() {
        let (flox, _) = &*FLOX_INSTANCE;

        let content = indoc! {r#"
        [project]
        requires-python = "1"
        "#};

        let pyproject = PyProject::from_pyproject_content(content, flox).unwrap();

        assert_eq!(pyproject.unwrap(), PyProject {
            provided_python_version: ProvidedVersion::Incompatible {
                requested: "^1".to_string(),
                substitute: ProvidedPackage::new("python3", vec!["python3"], "3.11.6"),
            }
        });
    }

    /// ProvidedVersion::Incompatible should be returned for requires-python = "1"
    #[test]
    #[serial]
    fn test_pyproject_parse_version() {
        let (flox, _) = &*FLOX_INSTANCE;

        // python docs have a space in the version (>= 3.8):
        // https://packaging.python.org/en/latest/guides/writing-pyproject-toml/#python-requires
        // Expect that version requirement to be parsed and passed on to pkgdb in canonical form.
        let content = indoc! {r#"
        [project]
        requires-python = ">= 3.8" # < with space
        "#};

        let pyproject = PyProject::from_pyproject_content(content, flox).unwrap();

        assert_eq!(pyproject.unwrap(), PyProject {
            provided_python_version: ProvidedVersion::Compatible {
                requested: Some(">=3.8".to_string()), // without space
                compatible: ProvidedPackage::new("python3", vec!["python39"], "3.9.18"),
            }
        });
    }

    /// An invalid pyproject.toml should return an error
    #[test]
    fn test_poetry_pyproject_invalid() {
        let (flox, _) = &*FLOX_INSTANCE;

        let content = indoc! {r#"
        ,
        "#};

        let pyproject = PoetryPyProject::from_pyproject_content(content, flox);

        assert!(pyproject.is_err());
    }
    */
}
