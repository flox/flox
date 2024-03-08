use std::borrow::Cow;
use std::fmt::{self, Debug, Display};
use std::path::Path;
use std::str::FromStr;

use anyhow::{anyhow, Context, Error, Result};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::path_environment::InitCustomization;
use flox_rust_sdk::models::manifest::PackageToInstall;
use indoc::{formatdoc, indoc};
use itertools::Itertools;
use log::debug;

use super::{format_customization, get_default_package_if_compatible, InitHook, AUTO_SETUP_HINT};
use crate::commands::init::get_default_package;
use crate::utils::dialog::{Dialog, Select};
use crate::utils::message;

pub(super) struct Python {
    providers: Vec<Provide<Box<dyn Provider>>>,
    selected_provider: Option<Box<dyn Provider>>,
}

impl Python {
    pub fn new(path: &Path, flox: &Flox) -> Self {
        let providers = vec![
            PoetryPyProject::detect(path, flox).into(),
            PyProject::detect(path, flox).into(),
            Requirements::detect(path, flox).into(),
        ];

        debug!("Detected Python providers: {:#?}", providers);

        Self {
            providers,
            selected_provider: None,
        }
    }
}

impl InitHook for Python {
    /// Returns `true` if any valid provider was found
    ///
    /// [Self::prompt_user] and [Self::get_init_customization]
    /// are expected to be called only if this method returns `true`!
    fn should_run(&mut self, _path: &Path) -> Result<bool> {
        Ok(self
            .providers
            .iter()
            .any(|provider| matches!(provider, Provide::Found(_))))
    }

    /// Empties the [Python::providers] and stores the selected provider in [Python::selected_provider]
    fn prompt_user(&mut self, _path: &Path, _flox: &Flox) -> Result<bool> {
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
    ///
    /// This method assumes that **it is only called if [Self::prompt_user] returned `true`**
    /// and will panic if no provider was selected or found.
    fn get_init_customization(&self) -> InitCustomization {
        let selected = self
            .selected_provider
            .as_ref()
            .map(|p| p.get_init_customization());
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

    fn describe_reason(&self) -> Cow<'static, str>;

    fn describe_customization(&self) -> Cow<'static, str>;

    fn compatible(&self) -> bool {
        true
    }

    fn get_init_customization(&self) -> InitCustomization;
}

/// Information gathered from a pyproject.toml file for poetry
/// <https://packaging.python.org/en/latest/guides/distributing-packages-using-setuptools/#configuring-setup-py>
#[derive(Debug)]
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
    fn detect(path: &Path, flox: &Flox) -> Result<Option<Self>> {
        debug!("Detecting poetry pyproject.toml at {:?}", path);

        let pyproject_toml = path.join("pyproject.toml");

        if !pyproject_toml.exists() {
            debug!("No pyproject.toml found at {:?}", path);
            return Ok(None);
        }

        let content = std::fs::read_to_string(&pyproject_toml)?;

        let toml = toml_edit::Document::from_str(&content)?;

        // poetry _requires_ `tool.poetry.dependencies.python` to be set [1],
        // so we do not resolve a default version here if the key is missing.
        //[1]: <https://python-poetry.org/docs/pyproject/#dependencies-and-dependency-groups>
        let Some(poetry) = toml.get("tool").and_then(|tool| tool.get("poetry")) else {
            return Ok(None);
        };

        let required_python_version = poetry
            .get("dependencies")
            .and_then(|dependencies| dependencies.get("python"))
            .map(|python| python.as_str().context("expected a string"))
            .transpose()?
            .ok_or_else(|| {
                anyhow!("No python version specified at 'tool.poetry.dependencies.python'")
            })?
            .to_string();

        let provided_python_version = 'version: {
            let compatible =
                get_default_package("python3", Some(required_python_version.clone()), flox)?;

            if let Some(found_version) = compatible {
                break 'version ProvidedVersion::Compatible {
                    compatible: found_version
                        .version
                        .unwrap_or_else(|| format!("comatible with {required_python_version}")),
                    requested: Some(required_python_version),
                };
            }

            log::debug!("poetry config requires python version {required_python_version}, but no compatible version found in the catalogs");

            let default =
                get_default_package_if_compatible(vec!["python3".to_string()], None, flox)?
                    .context("No python3 in the catalogs")?;

            ProvidedVersion::Incompatible {
                substitute: default.version.unwrap_or_else(|| "N/A".to_string()),
                requested: required_python_version,
            }
        };

        let poetry_version =
            get_default_package_if_compatible(vec!["poetry".to_string()], None, flox)?
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
        ", self.provided_python_version, self.poetry_version };

        if let ProvidedVersion::Incompatible {
            substitute,
            requested,
        } = &self.provided_python_version
        {
            message.push('\n');
            message.push_str(&format!(
                "Note: Flox could not provide requested version {requested}, but can provide {substitute} instead.",
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
            hook: Some(
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
#[derive(Debug)]
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

    /// Version of poetry found in the catalog
    pip_version: String,
}

impl PyProject {
    fn detect(path: &Path, flox: &Flox) -> Result<Option<Self>> {
        let pyproject_toml = path.join("pyproject.toml");

        if !pyproject_toml.exists() {
            return Ok(None);
        }

        let content = std::fs::read_to_string(&pyproject_toml)?;

        let toml = toml_edit::Document::from_str(&content)?;

        // unlike in poetry, `project.require-python` does not seem to be required
        //
        // TODO: check that this is _not (also)_ a poetry file?
        let required_python_version = toml
            .get("project")
            .and_then(|project| project.get("require-python"))
            .map(|constraint| constraint.as_str().context("expected a string"))
            .transpose()?
            .map(|s| s.to_string());

        let provided_python_version = 'version: {
            let search_default = || {
                let version =
                    get_default_package_if_compatible(vec!["python3".to_string()], None, flox)?
                        .context("No python3 in the catalogs")?
                        .version
                        .unwrap_or_else(|| "N/A".to_string());
                Ok::<_, Error>(version)
            };

            if required_python_version.is_none() {
                break 'version ProvidedVersion::Compatible {
                    compatible: search_default()?,
                    requested: None,
                };
            }

            let required_python_version_value = required_python_version.as_ref().unwrap();

            let compatible = get_default_package("python3", required_python_version.clone(), flox)?;

            if let Some(found_version) = compatible {
                break 'version ProvidedVersion::Compatible {
                    compatible: found_version.version.unwrap_or_else(|| "N/A".to_string()),
                    requested: required_python_version.clone(),
                };
            }

            log::debug!("poetry config requires python version {required_python_version_value}, but no compatible version found in the catalogs");

            ProvidedVersion::Incompatible {
                substitute: search_default()?,
                requested: required_python_version_value.clone(),
            }
        };

        let pip_version = get_default_package_if_compatible(
            vec!["python311Packages".to_string(), "pip".to_string()],
            None,
            flox,
        )?
        .context("Did not find pip in the catalogs")?
        .version
        .unwrap_or_else(|| "N/A".to_string());

        Ok(Some(PyProject {
            provided_python_version,
            pip_version,
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
            Installs python ({}) with pip ({})
            Adds a hook to setup a venv.
            Installs the dependencies from the pyproject.toml to the venv.
        ", self.provided_python_version, self.pip_version };

        if let ProvidedVersion::Incompatible {
            requested,
            substitute,
        } = &self.provided_python_version
        {
            message.push('\n');
            message.push_str(&format!(
                "Note: Flox could not provide requested version {requested}, but can provide {substitute} instead.",
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
            hook: Some(
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
            packages: Some(vec![
                PackageToInstall {
                    id: "python3".to_string(),
                    pkg_path: "python3".to_string(),
                    version: python_version,
                    input: None,
                },
                PackageToInstall {
                    id: "pip".to_string(),
                    pkg_path: "python311Packages.pip".to_string(),
                    version: None,
                    input: None,
                },
            ]),
        }
    }
}

#[derive(Debug)]
pub(super) struct Requirements {
    /// The latest version of python3 found in the catalogs
    python_version: String,

    /// The latest version of pip found in the catalogs
    pip_version: String,
}

impl Requirements {
    fn detect(path: &Path, flox: &Flox) -> Result<Option<Self>> {
        let requirements_txt = path.join("requirements.txt");

        if !requirements_txt.exists() {
            return Ok(None);
        }

        let result = get_default_package_if_compatible(vec!["python3".to_string()], None, flox)?
            .context("Did not find python3 in the catalogs")?;
        // given our catalog is based on nixpkgs,
        // we can assume that the version is always present.
        let python_version = result.version.unwrap_or_else(|| "N/A".to_string());

        let pip_version = get_default_package_if_compatible(
            vec!["python311Packages".to_string(), "pip".to_string()],
            None,
            flox,
        )?
        .context("Did not find pip in the catalogs")?
        .version
        .unwrap_or_else(|| "N/A".to_string());

        Ok(Some(Requirements {
            python_version,
            pip_version,
        }))
    }
}

impl Provider for Requirements {
    fn describe_provider(&self) -> Cow<'static, str> {
        "latest python".into()
    }

    fn describe_reason(&self) -> Cow<'static, str> {
        // Found ...
        "requirements.txt".into()
    }

    fn describe_customization(&self) -> Cow<'static, str> {
        formatdoc! {"
            Installs latest python ({}) with pip ({}).
            Adds hooks to setup and use a venv.
            Installs the dependencies from the requirements.txt to the venv.",
            self.python_version, self.pip_version,
        }
        .into()
    }

    fn get_init_customization(&self) -> InitCustomization {
        InitCustomization {
            hook: Some(
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

                pip install -r "$FLOX_ENV_PROJECT/requirements.txt" --quiet"#}
                .to_string(),
            ),
            packages: Some(vec![
                PackageToInstall {
                    id: "python3".to_string(),
                    pkg_path: "python3".to_string(),
                    version: None,
                    input: None,
                },
                PackageToInstall {
                    id: "pip".to_string(),
                    pkg_path: "python311Packages.pip".to_string(),
                    version: None,
                    input: None,
                },
            ]),
        }
    }
}

/// Distinguish compatible versions from default or incompatible versions
///
///
/// [ProvidedVersion::Compatible] if search yielded a compatible version to the requested version.
/// [ProvidedVersion::Incompatible::requested] may be [None] if no version was requested.
/// In that case any version found in the catalogs is considered compatible.
///
/// [ProvidedVersion::Incompatible] if no compatible version was found,
/// but another substitute was found.
///
/// [ProvidedVersion::Incompatible::requested] and [ProvidedVersion::Compatible::requested]
/// may be semver'ish, e.g. ">=3.6".
///
/// [ProvidedVersion::Incompatible::substitute] and [ProvidedVersion::Compatible::compatible]
/// are concrete versions, not semver!
#[derive(Debug)]
enum ProvidedVersion {
    Compatible {
        requested: Option<String>,
        compatible: String,
    },
    Incompatible {
        requested: String,
        substitute: String,
    },
}

impl Display for ProvidedVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Compatible { compatible, .. } => write!(f, "{compatible}"),
            Self::Incompatible { substitute, .. } => write!(f, "{substitute}"),
        }
    }
}
