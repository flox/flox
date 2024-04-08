use std::borrow::Cow;
use std::fmt::Debug;
use std::path::Path;
use std::str::FromStr;

use anyhow::{anyhow, Context, Error, Result};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::path_environment::InitCustomization;
use flox_rust_sdk::models::manifest::PackageToInstall;
use indoc::{formatdoc, indoc};
use itertools::Itertools;
use log::debug;
use regex::Regex;

use super::{
    format_customization,
    get_default_package_if_compatible,
    try_find_compatible_version,
    InitHook,
    ProvidedVersion,
    AUTO_SETUP_HINT,
};
use crate::utils::dialog::{Dialog, Select};
use crate::utils::message;

pub(super) struct Python {
    providers: Vec<Provide<Box<dyn Provider>>>,
    selected_provider: Option<Box<dyn Provider>>,
}

impl Python {
    /// Creates and returns the [Python] hook with any detected
    /// [Provider] instances.
    /// If no providers are detected, returns [None].
    pub fn new(flox: &Flox, path: &Path) -> Option<Self> {
        let providers = vec![
            PoetryPyProject::detect(flox, path).into(),
            PyProject::detect(flox, path).into(),
            Requirements::detect(flox, path).into(),
        ];

        debug!("Detected Python providers: {:#?}", providers);

        // TODO: warn about errors (at least send to sentry)
        if !providers
            .iter()
            .any(|provider| matches!(provider, Provide::Found(_)))
        {
            return None;
        }

        Some(Self {
            providers,
            selected_provider: None,
        })
    }
}

impl InitHook for Python {
    /// Empties the [Python::providers] and stores the selected provider in [Python::selected_provider]
    fn prompt_user(&mut self, _flox: &Flox, _path: &Path) -> Result<bool> {
        let mut found_providers = std::mem::take(&mut self.providers)
            .into_iter()
            .filter_map(|provider| match provider {
                Provide::Found(provider) => Some(provider),
                _ => None,
            })
            .collect::<Vec<_>>();

        fn describe_provider(provider: &dyn Provider) -> String {
            format!(
                "* {} ({})\n\n{}",
                provider.describe_provider(),
                provider.describe_reason(),
                textwrap::indent(&provider.describe_customization(), "  ")
            )
        }

        message::plain(formatdoc! {"
            Flox detected a Python project with the following Python provider(s):

            {}
        ", found_providers.iter().map(|p| describe_provider(p.as_ref())).join("\n")});

        let message = formatdoc! {"
            Would you like Flox to set up a standard Python environment?
            You can always change the environment's manifest with 'flox edit'"};

        let accept_options = found_providers
            .iter()
            .map(|provider| format!("Yes - with {}", provider.describe_provider()))
            .collect::<Vec<_>>();

        let n_accept_options = accept_options.len();

        let show_modifications_options = found_providers
            .iter()
            .map(|provider| {
                format!(
                    "Show suggested modifications for {}",
                    provider.describe_provider()
                )
            })
            .collect::<Vec<_>>();

        let cancel_option = ["No".to_string()];

        let options = accept_options
            .iter()
            .chain(cancel_option.iter())
            .chain(show_modifications_options.iter())
            .collect::<Vec<_>>();

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
                choice if choice < n_accept_options => {
                    let _ = self
                        .selected_provider
                        .insert(found_providers.swap_remove(choice));
                    return Ok(true);
                },
                c if c == n_accept_options => {
                    return Ok(false);
                },
                choice_with_offset => {
                    let choice = choice_with_offset - (n_accept_options + 1);

                    let provider = &found_providers[choice];
                    message::plain(format_customization(&provider.get_init_customization())?);
                },
            }
        }
    }

    /// Returns the customization of the selected provider or the first found provider
    fn get_init_customization(&self) -> InitCustomization {
        let selected = self
            .selected_provider
            .as_ref()
            .map(|p| p.get_init_customization());
        // self.providers will be empty if prompt_user() was called
        let default = self.providers.iter().find_map(|provider| match provider {
            Provide::Found(provider) => Some(provider.get_init_customization()),
            _ => None,
        });

        selected
            .or(default)
            .expect("Should only be called if `prompt_user` returned `true`")
    }
}

/// Flattened result of a provider detection
///
/// Combines [Result] and [Option] into a single enum
#[derive(Debug)]
enum Provide<T> {
    /// Found a valid provider
    Found(T),
    /// Found a provider, but it's invalid
    /// e.g. found a pyproject.toml, but it's not a valid poetry file
    // We don't necessarily want to forget the error,
    // but currently we don't do anything with it either.
    #[allow(dead_code)]
    Invalid(Error),
    /// Provider not found
    NotFound,
}

impl<P: Provider + 'static> From<Result<Option<P>>> for Provide<Box<dyn Provider>> {
    fn from(result: Result<Option<P>>) -> Self {
        match result {
            Ok(Some(provider)) => Provide::Found(Box::new(provider)),
            Ok(None) => Provide::NotFound,
            Err(err) => Provide::Invalid(err),
        }
    }
}

trait Provider: Debug {
    fn describe_provider(&self) -> Cow<'static, str>;

    fn describe_reason(&self) -> Cow<'_, str>;

    fn describe_customization(&self) -> Cow<'_, str>;

    fn compatible(&self) -> bool {
        true
    }

    fn get_init_customization(&self) -> InitCustomization;
}

/// Information gathered from a pyproject.toml file for poetry
/// <https://packaging.python.org/en/latest/guides/distributing-packages-using-setuptools/#configuring-setup-py>
#[derive(Debug, PartialEq)]
struct PoetryPyProject {
    /// Provided python version
    ///
    /// [ProvidedVersion::Compatible] if a version compatible with the requirement
    /// `tools.poetry.dependencies.python` in the pyproject.toml was found in the catalogs.
    ///
    ///  <https://python-poetry.org/docs/pyproject/#dependencies-and-dependency-groups>
    ///
    /// [ProvidedVersion::Default] if no compatible version was found, but a default version was found.
    provided_python_version: ProvidedVersion,

    /// Version of poetry found in the catalog
    poetry_version: String,
}

impl PoetryPyProject {
    fn detect(flox: &Flox, path: &Path) -> Result<Option<Self>> {
        debug!("Detecting poetry pyproject.toml at {:?}", path);

        let pyproject_toml = path.join("pyproject.toml");

        if !pyproject_toml.exists() {
            debug!("No pyproject.toml found at {:?}", path);
            return Ok(None);
        }

        let content = std::fs::read_to_string(&pyproject_toml)?;

        Self::from_pyproject_content(flox, &content)
    }

    fn from_pyproject_content(flox: &Flox, content: &str) -> Result<Option<PoetryPyProject>> {
        let toml = toml_edit::DocumentMut::from_str(content)?;

        // poetry _requires_ `tool.poetry.dependencies.python` to be set [1],
        // so we do not resolve a default version here if the key is missing.
        // [1]: <https://python-poetry.org/docs/pyproject/#dependencies-and-dependency-groups>
        let Some(poetry) = toml.get("tool").and_then(|tool| tool.get("poetry")) else {
            return Ok(None);
        };

        // python version constraints may use a looser semver syntax than
        // pkgdb. We'll parse and convert them to canonical form.
        let required_python_version = poetry
            .get("dependencies")
            .and_then(|dependencies| dependencies.get("python"))
            .map(|python| python.as_str().context("expected a string"))
            .transpose()?
            .ok_or_else(|| {
                anyhow!("No python version specified at 'tool.poetry.dependencies.python'")
            })?
            .parse::<semver::VersionReq>()?
            .to_string();

        let provided_python_version = 'version: {
            let compatible = try_find_compatible_version(
                flox,
                "python3",
                Some(&required_python_version),
                None::<Vec<&str>>,
            )?;

            if let Some(found_version) = compatible {
                break 'version ProvidedVersion::Compatible {
                    compatible: found_version.try_into()?,
                    requested: Some(required_python_version),
                };
            }

            log::debug!("poetry config requires python version {required_python_version}, but no compatible version found in the catalogs");

            let substitute = get_default_package_if_compatible(flox, ["python3"], None)?
                .context("No python3 in the catalogs")?
                .try_into()?;

            ProvidedVersion::Incompatible {
                substitute,
                requested: required_python_version,
            }
        };

        let poetry_version = get_default_package_if_compatible(flox, ["poetry"], None)?
            .context("Did not find poetry in the catalogs")?
            .version
            .unwrap_or_else(|| "N/A".to_string());

        Ok(Some(PoetryPyProject {
            provided_python_version,
            poetry_version,
        }))
    }
}

impl Provider for PoetryPyProject {
    fn describe_provider(&self) -> Cow<'static, str> {
        "poetry".into()
    }

    fn describe_reason(&self) -> Cow<'static, str> {
        "pyproject.toml for poetry".into()
    }

    fn describe_customization(&self) -> Cow<'static, str> {
        let mut message = formatdoc! {"
            Installs python ({}) with poetry ({})
            Adds a hook to lock the poetry project and load the poetry environment
        ", self.provided_python_version.display_version(), self.poetry_version };

        if let ProvidedVersion::Incompatible {
            substitute,
            requested,
        } = &self.provided_python_version
        {
            message.push('\n');
            message.push_str(&format!(
                "Note: Flox could not provide requested version {requested}, but can provide {sub_version} instead.",
                sub_version = substitute.display_version,
            ));
            message.push('\n');
        }

        message.into()
    }

    fn get_init_customization(&self) -> InitCustomization {
        let python_version = match &self.provided_python_version {
            ProvidedVersion::Incompatible { .. } => None, /* do not lock if no compatible version was found */
            ProvidedVersion::Compatible { requested, .. } => requested.clone(),
        };

        InitCustomization {
            script: Some(
                // TODO: when we support fish, we'll need to source activate.fish
                indoc! {r#"
                # Setup a Python virtual environment

                export POETRY_VIRTUALENVS_PATH="$FLOX_ENV_CACHE/poetry/virtualenvs"

                if [ -z "$(poetry env info --path)" ]; then
                  echo "Creating poetry virtual environment in $POETRY_VIRTUALENVS_PATH"
                  poetry lock --quiet
                fi

                echo "Activating poetry virtual environment"
                source "$(poetry env info --path)/bin/activate"

                poetry install --quiet"#}
                .to_string(),
            ),
            packages: Some(vec![
                PackageToInstall {
                    id: "python3".to_string(),
                    pkg_path: "python3".to_string(),
                    version: python_version,
                    input: None,
                },
                PackageToInstall {
                    id: "poetry".to_string(),
                    pkg_path: "poetry".to_string(),
                    version: None,
                    input: None,
                },
            ]),
        }
    }
}

/// Information gathered from a pyproject.toml file
/// <https://packaging.python.org/en/latest/guides/distributing-packages-using-setuptools/#configuring-setup-py>
#[derive(Debug, PartialEq)]
struct PyProject {
    /// Provided python version
    ///
    /// [ProvidedVersion::Compatible] if a version compatible with the requirement
    /// `project.require-python` in the pyproject.toml was found in the catalogs.
    ///
    ///
    /// [ProvidedVersion::Default] if no compatible version was found, but a default version was found.
    ///
    /// [ProvidedVersion::Default::requested] is the version requested in the pyproject.toml
    ///
    /// May be semver'ish, e.g. ">=3.6"
    ///
    /// <https://packaging.python.org/en/latest/guides/writing-pyproject-toml/#python-requires>
    ///
    /// [ProvidedVersion::Default::substitute] is the version found in the catalogs instead
    ///
    /// Concrete version, not semver!
    provided_python_version: ProvidedVersion,
}

impl PyProject {
    fn detect(flox: &Flox, path: &Path) -> Result<Option<Self>> {
        let pyproject_toml = path.join("pyproject.toml");

        if !pyproject_toml.exists() {
            return Ok(None);
        }

        let content = std::fs::read_to_string(&pyproject_toml)?;

        Self::from_pyproject_content(flox, &content)
    }

    fn from_pyproject_content(flox: &Flox, content: &str) -> Result<Option<PyProject>> {
        let toml = toml_edit::DocumentMut::from_str(content)?;

        // unlike in poetry, `project.require-python` does not seem to be required
        //
        // TODO: check that this is _not (also)_ a poetry file?
        //
        // python docs have a space in the version (>= 3.8)
        // https://packaging.python.org/en/latest/guides/writing-pyproject-toml/#python-requires
        // pkgdb currently throws an exception when passed that specifier
        let required_python_version = toml
            .get("project")
            .and_then(|project| project.get("requires-python"))
            .map(|constraint| constraint.as_str().context("expected a string"))
            .transpose()?
            .map(|s| s.parse::<semver::VersionReq>())
            .transpose()?
            .map(|req| req.to_string());

        let provided_python_version = 'version: {
            let search_default = || {
                let default = get_default_package_if_compatible(flox, ["python3"], None)?
                    .context("No python3 in the catalogs")?
                    .try_into()?;
                Ok::<_, Error>(default)
            };

            let Some(required_python_version) = required_python_version else {
                break 'version ProvidedVersion::Compatible {
                    compatible: search_default()?,
                    requested: None,
                };
            };

            let compatible = try_find_compatible_version(
                flox,
                "python3",
                Some(required_python_version.clone()),
                None::<Vec<&str>>,
            )?;

            if let Some(found_version) = compatible {
                break 'version ProvidedVersion::Compatible {
                    compatible: found_version.try_into()?,
                    requested: Some(required_python_version),
                };
            }

            log::debug!("pyproject.toml requires python version {required_python_version}, but no compatible version found in the catalogs");

            ProvidedVersion::Incompatible {
                substitute: search_default()?,
                requested: required_python_version.clone(),
            }
        };

        Ok(Some(PyProject {
            provided_python_version,
        }))
    }
}

impl Provider for PyProject {
    fn describe_provider(&self) -> Cow<'static, str> {
        "pyproject".into()
    }

    fn describe_reason(&self) -> Cow<'static, str> {
        "generic pyproject.toml".into()
    }

    fn describe_customization(&self) -> Cow<'static, str> {
        let mut message = formatdoc! {"
            Installs python ({}) with pip bundled.
            Adds a hook to setup a venv.
            Installs the dependencies from the pyproject.toml to the venv.
        ", self.provided_python_version.display_version() };

        if let ProvidedVersion::Incompatible {
            requested,
            substitute,
        } = &self.provided_python_version
        {
            message.push('\n');
            message.push_str(&format!(
                "Note: Flox could not provide requested version {requested}, but can provide {sub_version} instead.",
                sub_version = substitute.display_version,
            ));
            message.push('\n');
        }

        message.into()
    }

    fn get_init_customization(&self) -> InitCustomization {
        let python_version = match &self.provided_python_version {
            ProvidedVersion::Incompatible { .. } => None, /* do not lock if no compatible version was found */
            ProvidedVersion::Compatible { requested, .. } => requested.clone(),
        };

        InitCustomization {
            script: Some(
                // TODO: when we support fish, we'll need to source activate.fish
                indoc! {r#"
                # Setup a Python virtual environment

                PYTHON_DIR="$FLOX_ENV_CACHE/python"
                if [ ! -d "$PYTHON_DIR" ]; then
                  echo "Creating python virtual environment in $PYTHON_DIR"
                  python -m venv "$PYTHON_DIR"
                fi

                echo "Activating python virtual environment"
                source "$PYTHON_DIR/bin/activate"

                # install the dependencies for this project based on pyproject.toml
                # <https://pip.pypa.io/en/stable/cli/pip_install/>

                pip install -e . --quiet"#}
                .to_string(),
            ),
            packages: Some(vec![PackageToInstall {
                id: "python3".to_string(),
                pkg_path: "python3".to_string(),
                version: python_version,
                input: None,
            }]),
        }
    }
}

#[derive(Debug)]
pub(super) struct Requirements {
    /// The latest version of python3 found in the catalogs
    python_version: String,
    filenames: Vec<String>,
}

impl Requirements {
    /// Gets the filenames of all the requirements.txt files in the given directory
    fn get_matches(path: &Path) -> Result<Vec<String>> {
        // NOTE: Does not match requirements files that have a prefix like `example_requirements.txt`
        // See https://github.com/flox/flox/issues/1323
        let pat = Regex::new(r"^requirements\S*\.txt")?;
        let dir_it = std::fs::read_dir(path)?;
        let matches: Vec<String> = dir_it
            .filter_map(|entry_res| match entry_res {
                Ok(entry) => {
                    let path = entry.path();

                    if path.is_file() {
                        // Files are considered valid requirements files if they:
                        // Have a name (should always be the case)
                        if let Some(file_name_osstr) = path.file_name() {
                            // The name is valid unicode
                            if let Some(file_name) = file_name_osstr.to_str() {
                                // The name matches the requirements*.txt pattern
                                if pat.is_match(file_name) {
                                    // NOTE: Does not currently check the contents of the file
                                    return Some(Ok(file_name.to_string()));
                                }
                            }
                        }
                    }
                    None
                },
                // Convert from std::io::Error to anyhow::Error
                Err(e) => Some(Err(e.into())),
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(matches)
    }

    fn detect(flox: &Flox, path: &Path) -> Result<Option<Self>> {
        debug!("Detecting python requirements.txt at {:?}", path);
        let matches = Self::get_matches(path)?;

        if !matches.is_empty() {
            let result = get_default_package_if_compatible(flox, ["python3"], None)?
                .context("Did not find python3 in the catalogs")?;
            // given our catalog is based on nixpkgs,
            // we can assume that the version is always present.
            let python_version = result.version.unwrap_or_else(|| "N/A".to_string());

            Ok(Some(Requirements {
                python_version,
                filenames: matches,
            }))
        } else {
            debug!("Did not find a python requirements.txt at {:?}", path);
            Ok(None)
        }
    }
}

impl Provider for Requirements {
    fn describe_provider(&self) -> Cow<'static, str> {
        "latest python".into()
    }

    fn describe_reason(&self) -> Cow<'_, str> {
        // Found ...
        self.filenames.join(", ").into()
    }

    fn describe_customization(&self) -> Cow<'_, str> {
        formatdoc! {"
            Installs latest python ({}) with pip bundled.
            Adds hooks to setup and use a venv.
            Installs dependencies to the venv from: {}",
            self.python_version,
            self.filenames.join(", ")
        }
        .into()
    }

    fn get_init_customization(&self) -> InitCustomization {
        let pip_cmds = self
            .filenames
            .iter()
            .map(|file_name| {
                formatdoc! {r#"
                pip install -r "$FLOX_ENV_PROJECT/{}" --quiet"#,
                file_name
                }
            })
            .join("\n");
        InitCustomization {
            script: Some(
                // TODO: when we support fish, we'll need to source activate.fish
                formatdoc! {r#"
                # Setup a Python virtual environment

                PYTHON_DIR="$FLOX_ENV_CACHE/python"
                if [ ! -d "$PYTHON_DIR" ]; then
                  echo "Creating python virtual environment in $PYTHON_DIR"
                  python -m venv "$PYTHON_DIR"
                fi

                echo "Activating python virtual environment"
                source "$PYTHON_DIR/bin/activate"

                {pip_cmds}"#}
                .to_string(),
            ),
            packages: Some(vec![PackageToInstall {
                id: "python3".to_string(),
                pkg_path: "python3".to_string(),
                version: None,
                input: None,
            }]),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs::File;

    use flox_rust_sdk::flox::test_helpers::flox_instance_with_global_lock;
    use pretty_assertions::assert_eq;
    use serial_test::serial;

    use super::*;
    use crate::commands::init::ProvidedPackage;

    /// An invalid pyproject.toml should return an error
    #[test]
    fn test_pyproject_invalid() {
        let (flox, _temp_dir_handle) = flox_instance_with_global_lock();

        let content = indoc! {r#"
        ,
        "#};

        let pyproject = PyProject::from_pyproject_content(&flox, content);

        assert!(pyproject.is_err());
    }

    /// ProvidedVersion::Compatible should be returned for an empty pyproject.toml
    #[test]
    #[serial]
    fn test_pyproject_empty() {
        let (flox, _temp_dir_handle) = flox_instance_with_global_lock();

        let pyproject = PyProject::from_pyproject_content(&flox, "").unwrap();

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
        let (flox, _temp_dir_handle) = flox_instance_with_global_lock();

        let content = indoc! {r#"
        [project]
        requires-python = ">= 3.8"
        "#};

        let pyproject = PyProject::from_pyproject_content(&flox, content).unwrap();

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
        let (flox, _temp_dir_handle) = flox_instance_with_global_lock();

        let content = indoc! {r#"
        [project]
        requires-python = "1"
        "#};

        let pyproject = PyProject::from_pyproject_content(&flox, content).unwrap();

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
        let (flox, _temp_dir_handle) = flox_instance_with_global_lock();

        // python docs have a space in the version (>= 3.8):
        // https://packaging.python.org/en/latest/guides/writing-pyproject-toml/#python-requires
        // Expect that version requirement to be parsed and passed on to pkgdb in canonical form.
        let content = indoc! {r#"
        [project]
        requires-python = ">= 3.8" # < with space
        "#};

        let pyproject = PyProject::from_pyproject_content(&flox, content).unwrap();

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
        let (flox, _temp_dir_handle) = flox_instance_with_global_lock();

        let content = indoc! {r#"
        ,
        "#};

        let pyproject = PoetryPyProject::from_pyproject_content(&flox, content);

        assert!(pyproject.is_err());
    }

    /// None should be returned for an empty pyproject.toml
    #[test]
    fn test_poetry_pyproject_empty() {
        let (flox, _temp_dir_handle) = flox_instance_with_global_lock();

        let pyproject = PoetryPyProject::from_pyproject_content(&flox, "").unwrap();

        assert_eq!(pyproject, None);
    }

    /// Err should be returned for a pyproject.toml with `tool.poetry` but not
    /// `tool.poetry.dependencies.python`
    #[test]
    fn test_poetry_pyproject_no_python() {
        let (flox, _temp_dir_handle) = flox_instance_with_global_lock();

        let content = indoc! {r#"
        [tool.poetry]
        "#};

        let pyproject = PoetryPyProject::from_pyproject_content(&flox, content);

        assert!(pyproject.is_err());
    }

    /// ProvidedVersion::Compatible should be returned for python = "^3.7"
    #[test]
    #[serial]
    fn test_poetry_pyproject_available_version() {
        let (flox, _temp_dir_handle) = flox_instance_with_global_lock();

        let content = indoc! {r#"
        [tool.poetry.dependencies]
        python = "^3.7"
        "#};

        let pyproject = PoetryPyProject::from_pyproject_content(&flox, content).unwrap();

        assert_eq!(pyproject.unwrap(), PoetryPyProject {
            provided_python_version: ProvidedVersion::Compatible {
                requested: Some("^3.7".to_string()),
                compatible: ProvidedPackage::new("python3", vec!["python39"], "3.9.18"),
            },
            poetry_version: "1.7.1".to_string(),
        });
    }

    /// ProvidedVersion::Incompatible should be returned for python = "1"
    #[test]
    #[serial]
    fn test_poetry_pyproject_unavailable_version() {
        let (flox, _temp_dir_handle) = flox_instance_with_global_lock();

        let content = indoc! {r#"
        [tool.poetry.dependencies]
        python = "1"
        "#};

        let pyproject = PoetryPyProject::from_pyproject_content(&flox, content).unwrap();

        assert_eq!(pyproject.unwrap(), PoetryPyProject {
            provided_python_version: ProvidedVersion::Incompatible {
                requested: "^1".to_string(),
                substitute: ProvidedPackage::new("python3", vec!["python3"], "3.11.6"),
            },
            poetry_version: "1.7.1".to_string(),
        });
    }

    /// Requirements::get_matches should return an empty Vec if no requirements files are found
    #[test]
    fn test_requirements_no_match() {
        let (flox, _temp_dir_handle) = flox_instance_with_global_lock();
        let temp_dir = flox.temp_dir;
        let no_match = temp_dir.join("not_a_requirements.txt");
        let no_match2 = temp_dir.join("random_file.txt");

        File::create(no_match).unwrap();
        File::create(no_match2).unwrap();
        let matches = Requirements::get_matches(&temp_dir).unwrap();
        assert!(matches.is_empty());
    }

    /// Requirements::detect should match requirements.txt
    #[test]
    fn test_requirements_matches_conventional() {
        let (flox, _temp_dir_handle) = flox_instance_with_global_lock();
        let temp_dir = flox.temp_dir;
        let requirements_file = temp_dir.join("requirements.txt");
        File::create(requirements_file).unwrap();
        let matches = Requirements::get_matches(&temp_dir).unwrap();
        assert!(matches.len() == 1);
        assert_eq!(matches[0], "requirements.txt");
    }

    /// Requirements::detect should match requirements_versioned.txt
    #[test]
    fn test_requirements_matches_unconventional() {
        let (flox, _temp_dir_handle) = flox_instance_with_global_lock();
        let temp_dir = flox.temp_dir;
        let requirements_file_unconventional = temp_dir.join("requirements_versioned.txt");
        File::create(requirements_file_unconventional).unwrap();
        let matches = Requirements::get_matches(&temp_dir).unwrap();
        assert!(matches.len() == 1);
        assert_eq!(matches[0], "requirements_versioned.txt");
    }

    /// Requirements::detect should return all matches
    #[test]
    fn test_requirements_matches_all() {
        let (flox, _temp_dir_handle) = flox_instance_with_global_lock();
        let temp_dir = flox.temp_dir;
        let long_name = temp_dir.join("requirements_versioned_dev.txt");
        let short_name = temp_dir.join("requirements_versioned.txt");
        File::create(long_name).unwrap();
        File::create(short_name).unwrap();
        let matches = Requirements::get_matches(&temp_dir).unwrap();
        assert!(matches.len() == 2);
        // std::fs::read_dir does not guarantee order
        assert!(matches
            .iter()
            .any(|s| s == "requirements_versioned_dev.txt"));
        assert!(matches.iter().any(|s| s == "requirements_versioned.txt"));
    }
}
