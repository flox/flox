use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{anyhow, Context, Result};
use bpaf::Bpaf;
use flox_rust_sdk::flox::{EnvironmentName, Flox, DEFAULT_NAME};
use flox_rust_sdk::models::environment::path_environment::{InitCustomization, PathEnvironment};
use flox_rust_sdk::models::environment::{
    global_manifest_lockfile_path,
    global_manifest_path,
    Environment,
    PathPointer,
};
use flox_rust_sdk::models::lockfile::LockedManifest;
use flox_rust_sdk::models::manifest::{insert_packages, PackageToInstall};
use flox_rust_sdk::models::search::{do_search, PathOrJson, Query, SearchParams, SearchResult};
use indoc::{formatdoc, indoc};
use log::debug;
use semver::VersionReq;
use toml_edit::{Document, Formatted, Item, Table, Value};

use crate::commands::{environment_description, ConcreteEnvironment};
use crate::config::features::Features;
use crate::subcommand_metric;
use crate::utils::dialog::{Dialog, Select, Spinner};
use crate::utils::message;

const AUTO_SETUP_HINT: &str = "Use '--auto-setup' to apply Flox recommendations in the future.";

// Create an environment in the current directory
#[derive(Bpaf, Clone)]
pub struct Init {
    /// Directory to create the environment in (default: current directory)
    #[bpaf(long, short, argument("path"))]
    dir: Option<PathBuf>,

    /// Name of the environment
    ///
    /// "$(basename $PWD)" or "default" if in $HOME
    #[bpaf(long("name"), short('n'), argument("name"))]
    env_name: Option<String>,

    /// Apply Flox recommendations for the environment based on what languages
    /// are being used in the containing directory
    #[bpaf(long)]
    auto_setup: bool,
}

impl Init {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("init");

        let dir = self
            .dir
            .clone()
            .unwrap_or_else(|| std::env::current_dir().unwrap());

        let home_dir = dirs::home_dir().unwrap();

        let env_name = if let Some(ref name) = self.env_name {
            EnvironmentName::from_str(name)?
        } else if dir == home_dir {
            EnvironmentName::from_str(DEFAULT_NAME)?
        } else {
            let name = dir
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .context("Can't init in root")?;
            EnvironmentName::from_str(&name)?
        };

        // Don't run hooks in home dir
        let customization = (dir != home_dir)
            .then(|| self.run_hooks(&dir, &flox))
            .transpose()?
            .unwrap_or_default();

        let env = if customization.packages.is_some() {
            Dialog {
                message: "Installing Flox suggested packages...",
                help_message: None,
                typed: Spinner::new(|| {
                    PathEnvironment::init(
                        PathPointer::new(env_name),
                        &dir,
                        flox.temp_dir.clone(),
                        &flox.system,
                        &customization,
                        &flox,
                    )
                }),
            }
            .spin()?
        } else {
            PathEnvironment::init(
                PathPointer::new(env_name),
                &dir,
                flox.temp_dir.clone(),
                &flox.system,
                &customization,
                &flox,
            )?
        };

        message::created(format!(
            "Created environment {name} ({system})",
            name = env.name(),
            system = flox.system
        ));
        if let Some(packages) = customization.packages {
            let description = environment_description(&ConcreteEnvironment::Path(env))?;
            for package in packages {
                message::package_installed(&package, &description);
            }
        }
        message::plain(formatdoc! {"

            Next:
              $ flox search <package>    <- Search for a package
              $ flox install <package>   <- Install a package into an environment
              $ flox activate            <- Enter the environment
            "
        });
        Ok(())
    }

    /// Run all hooks and return a single combined customization
    fn run_hooks(&self, dir: &Path, flox: &Flox) -> Result<InitCustomization> {
        let hooks: Vec<Box<dyn InitHook>> = if std::env::var("_FLOX_NODE_HOOK").is_ok() {
            vec![Box::new(Requirements), Box::new(Node::new(dir, flox)?)]
        } else {
            vec![Box::new(Requirements)]
        };

        let mut customizations = vec![];

        for mut hook in hooks {
            // Run hooks if we can't prompt
            if hook.should_run(dir)?
                && (self.auto_setup || !Dialog::can_prompt() || hook.prompt_user(dir, flox)?)
            {
                customizations.push(hook.get_init_customization())
            }
        }

        Ok(Self::combine_customizations(customizations))
    }

    /// Deduplicate packages and concatenate hooks into a single string
    fn combine_customizations(customizations: Vec<InitCustomization>) -> InitCustomization {
        let mut hooks: Vec<String> = vec![];
        // Deduplicate packages with a set
        let mut packages_set = HashSet::<PackageToInstall>::new();
        for customization in customizations {
            if let Some(packages) = customization.packages {
                packages_set.extend(packages)
            }
            if let Some(hook) = customization.hook {
                hooks.push(hook)
            }
        }

        let hook = (!hooks.is_empty()).then(|| {
            formatdoc! {"
                # Autogenerated by flox

                {}

                # End autogenerated by flox", hooks.join("\n\n")}
        });

        let packages = (!packages_set.is_empty())
            .then(|| packages_set.into_iter().collect::<Vec<PackageToInstall>>());

        InitCustomization { hook, packages }
    }
}

// TODO: clean up how we pass around path and flox
trait InitHook {
    fn should_run(&mut self, path: &Path) -> Result<bool>;

    fn prompt_user(&mut self, path: &Path, flox: &Flox) -> Result<bool>;

    fn get_init_customization(&self) -> InitCustomization;
}

struct Requirements;

impl InitHook for Requirements {
    fn should_run(&mut self, path: &Path) -> Result<bool> {
        Ok(path.join("requirements.txt").exists())
    }

    fn prompt_user(&mut self, _path: &Path, _flox: &Flox) -> Result<bool> {
        let message = formatdoc! {"
            Flox detected a requirements.txt

            Python projects typically need:
            * python, pip
            * A Python virtual environment to install dependencies into

            Would you like Flox to set up a standard Python environment?
            You can always revisit the environment's declaration with 'flox edit'
        "};

        let dialog = Dialog {
            message: &message,
            help_message: Some(AUTO_SETUP_HINT),
            typed: Select {
                options: ["Yes (python 3.11)", "No", "Show suggested modifications"].to_vec(),
            },
        };

        let (mut choice, _) = dialog.raw_prompt()?;

        if choice == 2 {
            let message = formatdoc! {"

                {}
                Would you like Flox to apply these modifications?
                You can always revisit the environment's declaration with 'flox edit'
            ", format_customization(&self.get_init_customization())?};

            let dialog = Dialog {
                message: &message,
                help_message: Some(AUTO_SETUP_HINT),
                typed: Select {
                    options: ["Yes (python 3.11)", "No"].to_vec(),
                },
            };

            (choice, _) = dialog.raw_prompt()?;
        }

        Ok(choice == 0)
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
                    pkg_path: "python311Packages.python".to_string(),
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

struct Node {
    nvmrc_version: NVMRCVersion,
    /// None if we found a version in .nvmrc, otherwise contains whether
    /// package.json requests a version
    package_json_version: Option<PackageJSONVersion>,
    hook: Option<NodePackageManager>,
}

enum NodePackageManager {
    Npm,
    Yarn,
    Both,
}

#[derive(Debug, PartialEq)]
enum RequestedNVMRCVersion {
    /// .nvmrc not present or empty
    None,
    /// .nvmrc contains an alias or something we can't parse as a version.
    Unsure,
    Some(String),
}

enum NVMRCVersion {
    /// .nvmrc not present or empty
    None,
    /// .nvmrc or package.json contains a version,
    /// but flox doesn't provide it.
    Unavailable,
    /// .nvmrc contains an alias or something we can't parse as a version.
    Unsure,
    Some(Box<SearchResult>),
}

enum PackageJSONVersion {
    /// package.json does not exist
    None,
    /// package.json exists but doesn't have an engines.node field
    Unspecified,
    Unavailable,
    Some(Box<SearchResult>),
}

enum NodeAction {
    Install(Box<SearchResult>),
    OfferFloxDefault,
    Nothing,
}

impl Node {
    pub fn new(path: &Path, flox: &Flox) -> Result<Self> {
        // Get value for self.nvmrc_version
        let nvmrc_version = Self::get_nvmrc_version(path, flox)?;

        // Get value for self.package_json_version
        let package_json_version = match nvmrc_version {
            NVMRCVersion::Some(_) => None,
            _ => Some(Self::get_package_json_version(path, flox)?),
        };

        // Get value for self.hook
        let hook = if path.join("package.json").exists() {
            let package_lock_exists = path.join("package-lock.json").exists();
            let yarn_lock_exists = path.join("yarn.lock").exists();
            match (package_lock_exists, yarn_lock_exists) {
                (false, false) => None,
                (true, false) => Some(NodePackageManager::Npm),
                (false, true) => Some(NodePackageManager::Yarn),
                (true, true) => Some(NodePackageManager::Both),
            }
        } else {
            None
        };

        Ok(Self {
            nvmrc_version,
            package_json_version,
            hook,
        })
    }

    /// Determine appropriate [NVMRCVersion] variant for a (possibly
    /// non-existent) `.nvmrc` file in `path`.
    ///
    /// This will perform a search to determine if a requested version is
    /// available.
    fn get_nvmrc_version(path: &Path, flox: &Flox) -> Result<NVMRCVersion> {
        let nvmrc = path.join(".nvmrc");
        if !nvmrc.exists() {
            return Ok(NVMRCVersion::None);
        }

        let nvmrc_contents = fs::read_to_string(&nvmrc)?;
        let nvmrc_version = match Self::parse_nvmrc_version(&nvmrc_contents) {
            RequestedNVMRCVersion::None => NVMRCVersion::None,
            RequestedNVMRCVersion::Unsure => NVMRCVersion::Unsure,
            RequestedNVMRCVersion::Some(version) => {
                match Self::check_for_node_version(&version, flox)? {
                    None => NVMRCVersion::Unavailable,
                    Some(result) => NVMRCVersion::Some(Box::new(result)),
                }
            },
        };
        Ok(nvmrc_version)
    }

    fn parse_nvmrc_version(nvmrc_contents: &str) -> RequestedNVMRCVersion {
        // When reading from a file, nvm runs:
        // "$(command head -n 1 "${NVMRC_PATH}" | command tr -d '\r')" || command printf ''
        // https://github.com/nvm-sh/nvm/blob/294ff9e3aa8ce02bbf8d83fa235a363d9560a179/nvm.sh#L481
        let first_line = nvmrc_contents.lines().next();
        // From nvm --help:
        // <version> refers to any version-like string nvm understands. This includes:
        //   - full or partial version numbers, starting with an optional "v" (0.10, v0.1.2, v1)
        //   - default (built-in) aliases: node, stable, unstable, iojs, system
        //   - custom aliases you define with `nvm alias foo`
        match first_line {
            None => RequestedNVMRCVersion::None,
            Some(first_line) => {
                // nvm will fail if there's trailing whitespace,
                // so trimming whitespace is technically inconsistent,
                // but it's still probably a good recommendation from flox.
                let trimmed_first_line = first_line.trim();
                match trimmed_first_line {
                    "" => RequestedNVMRCVersion::None,
                    "node" | "stable" | "unstable" | "iojs" | "system" => {
                        RequestedNVMRCVersion::Unsure
                    },
                    _ if trimmed_first_line.starts_with('v')
                        && VersionReq::parse(&trimmed_first_line[1..]).is_ok() =>
                    {
                        RequestedNVMRCVersion::Some(trimmed_first_line[1..].to_string())
                    },
                    _ if VersionReq::parse(trimmed_first_line).is_ok() => {
                        RequestedNVMRCVersion::Some(trimmed_first_line.to_string())
                    },
                    _ => RequestedNVMRCVersion::Unsure,
                }
            },
        }
    }

    /// Determine appropriate [PackageJSONVersion] variant for a (possibly
    /// non-existent) `package.json` file in `path`
    ///
    /// This will perform a search to determine if a requested version is
    /// available.
    fn get_package_json_version(path: &Path, flox: &Flox) -> Result<PackageJSONVersion> {
        let package_json = path.join("package.json");
        if !package_json.exists() {
            return Ok(PackageJSONVersion::None);
        }
        let package_json_contents = fs::read_to_string(package_json)?;
        match serde_json::from_str::<serde_json::Value>(&package_json_contents) {
            // Treat a package.json that can't be parsed as JSON the same as it not existing
            Err(_) => Ok(PackageJSONVersion::None),
            Ok(package_json_json) => match package_json_json["engines"]["node"].as_str() {
                Some(version) => match Self::check_for_node_version(version, flox)? {
                    None => Ok(PackageJSONVersion::Unavailable),
                    Some(result) => Ok(PackageJSONVersion::Some(Box::new(result))),
                },
                None => Ok(PackageJSONVersion::Unspecified),
            },
        }
    }

    fn check_for_node_version(version: &str, flox: &Flox) -> Result<Option<SearchResult>> {
        let query = Query {
            name: None,
            pname: Some("nodejs".to_string()),
            version: None,
            semver: Some(version.to_string()),
            r#match: None,
            match_name: None,
            match_name_or_rel_path: None,
            limit: Some(1),
            deduplicate: false,
        };
        let params = SearchParams {
            manifest: None,
            global_manifest: PathOrJson::Path(global_manifest_path(flox)),
            lockfile: PathOrJson::Path(LockedManifest::ensure_global_lockfile(flox)?),
            query,
        };

        let (mut results, _) = do_search(&params)?;

        if results.results.is_empty() {
            return Ok(None);
        }
        Ok(Some(results.results.swap_remove(0)))
    }

    fn get_default_node(flox: &Flox) -> Result<SearchResult> {
        let query = Query::new("nodejs", Features::parse()?.search_strategy, Some(1), false)?;
        let params = SearchParams {
            manifest: None,
            global_manifest: PathOrJson::Path(global_manifest_path(flox)),
            lockfile: PathOrJson::Path(global_manifest_lockfile_path(flox)),
            query,
        };

        let (mut results, _) = do_search(&params)?;

        if results.results.is_empty() {
            Err(anyhow!("Flox couldn't find any versions of nodejs"))?
        }
        Ok(results.results.swap_remove(0))
    }

    fn get_action(&self) -> NodeAction {
        match (&self.nvmrc_version, self.package_json_version.as_ref()) {
            (NVMRCVersion::Some(result), _) => NodeAction::Install(result.clone()),
            // Maybe the project works with a different node version,
            // and there's just something else in .nvmrc
            (NVMRCVersion::Unavailable, Some(PackageJSONVersion::None))
            | (NVMRCVersion::Unavailable, Some(PackageJSONVersion::Unspecified)) => {
                NodeAction::OfferFloxDefault
            },
            // If package.json asks for a version we don't have, don't offer a
            // version that would cause warnings.
            (_, Some(PackageJSONVersion::Unavailable)) => NodeAction::Nothing,
            // The project will work with a flox provided version, even though
            // it's not the one in .nvmrc.
            (NVMRCVersion::Unavailable, Some(PackageJSONVersion::Some(result))) => {
                NodeAction::Install(result.clone())
            },
            (NVMRCVersion::None, Some(PackageJSONVersion::None)) => NodeAction::Nothing,
            (NVMRCVersion::None, Some(PackageJSONVersion::Some(result))) => {
                NodeAction::Install(result.clone())
            },
            (NVMRCVersion::None, Some(PackageJSONVersion::Unspecified)) => {
                NodeAction::OfferFloxDefault
            },
            (NVMRCVersion::Unsure, Some(PackageJSONVersion::None))
            | (NVMRCVersion::Unsure, Some(PackageJSONVersion::Unspecified)) => {
                NodeAction::OfferFloxDefault
            },
            (NVMRCVersion::Unsure, Some(PackageJSONVersion::Some(result))) => {
                NodeAction::Install(result.clone())
            },
            (_, None) => unreachable!(),
        }
    }

    /// Returns:
    /// 1. A message describing what version of nodejs Flox found requested to
    /// include in the prompt.
    /// 2. The version of nodejs Flox would install
    /// 3. Whether the message says Flox detected package.json (to avoid
    ///    printing that message twice)
    fn nodejs_message_and_version(&self, flox: &Flox) -> Result<(String, Option<String>, bool)> {
        let mut mentions_package_json = false;
        let (message, version) = match (&self.nvmrc_version, self.package_json_version.as_ref()) {
            (NVMRCVersion::Some(result), _) => {
                let message = format!(
                    "Flox detected an .nvmrc{}",
                    result
                        .version
                        .as_ref()
                        .map(|version| format!(" compatible with node {version}"))
                        .unwrap_or("".to_string())
                );
                (message, result.version.clone())
            },

            (NVMRCVersion::Unavailable, Some(PackageJSONVersion::None))
            | (NVMRCVersion::Unavailable, Some(PackageJSONVersion::Unspecified)) =>
            // Maybe the project works with a different node version,
            // and there's just something else in .nvmrc
            {
                let result = Self::get_default_node(flox)?;
                let message = format!("Flox detected an .nvmrc with a version of nodejs not provided by Flox, but Flox can provide {}",
                       result.version.as_ref().map(|version|format!("version {version}")).unwrap_or("another version".to_string()));
                (message, result.version)
            },
            (_, Some(PackageJSONVersion::Unavailable)) =>
            // If package.json asks for a version we don't have, don't offer a
            // version that would cause warnings.
            {
                unreachable!()
            },
            (NVMRCVersion::Unavailable, Some(PackageJSONVersion::Some(result))) =>
            // The project will work with a flox provided version, even though
            // it's not the one in .nvmrc.
            {
                let message = format!("Flox detected an .nvmrc with a version of nodejs not provided by Flox, but Flox can provide {}",
                result.version.as_ref().map(|version|format!("version {version}")).unwrap_or("another version".to_string()));
                (message, result.version.clone())
            },
            (NVMRCVersion::None, Some(PackageJSONVersion::None)) => unreachable!(),
            (NVMRCVersion::None, Some(PackageJSONVersion::Some(result))) => {
                let message = format!(
                    "Flox detected a package.json compatible with node{}",
                    result
                        .version
                        .as_ref()
                        .map(|version| format!(" {version}"))
                        .unwrap_or("".to_string())
                );
                mentions_package_json = true;
                (message, result.version.clone())
            },

            (NVMRCVersion::None, Some(PackageJSONVersion::Unspecified)) => {
                let result = Self::get_default_node(flox)?;
                mentions_package_json = true;
                ("Flox detected a package.json".to_string(), result.version)
            },
            (NVMRCVersion::Unsure, Some(PackageJSONVersion::None))
            | (NVMRCVersion::Unsure, Some(PackageJSONVersion::Unspecified)) => {
                let result = Self::get_default_node(flox)?;
                let message = format!("Flox detected an .nvmrc with a version specifier not understood by Flox, but Flox can provide {}",
                       result.version.as_ref().map(|version|format!("version {version}")).unwrap_or("another version".to_string()));
                (message, result.version)
            },
            (NVMRCVersion::Unsure, Some(PackageJSONVersion::Some(result))) => {
                let message = format!("Flox detected an .nvmrc with a version specifier not understood by Flox, but Flox can provide {}",
                       result.version.as_ref().map(|version|format!("version {version}")).unwrap_or("another version".to_string()));
                (message, result.version.clone())
            },
            (_, None) => unreachable!(),
        };
        Ok((message, version, mentions_package_json))
    }

    fn prompt_with_known_hook(&self, flox: &Flox) -> Result<bool> {
        let (nodejs_detected, nodejs_version, mentions_package_json) =
            self.nodejs_message_and_version(flox)?;
        let mut message = format!("{nodejs_detected}\n");
        match self.hook {
            None => {},
            Some(NodePackageManager::Npm) if !mentions_package_json => {
                message.push_str("Flox detected a package.json\n")
            },
            Some(NodePackageManager::Npm) => {},
            Some(NodePackageManager::Yarn) => message.push_str("Flox detected a yarn.lock\n"),
            Some(NodePackageManager::Both) => unreachable!(),
        }
        message.push_str(&formatdoc! {"

            Flox can add the following to your environment:
            * nodejs{}
        ",
            nodejs_version
                .map(|version| format!(" {version}"))
                .unwrap_or("".to_string()),
        });

        match self.hook {
            None => {},
            Some(NodePackageManager::Npm) => message.push_str("* An npm installation hook\n"),
            Some(NodePackageManager::Yarn) => message.push_str("* A yarn installation hook\n"),
            Some(NodePackageManager::Both) => unreachable!(),
        }

        message.push_str(&formatdoc! {"

            Would you like Flox to apply this suggestion?
            You can always revisit the environment's declaration with 'flox edit'
            "});

        let dialog = Dialog {
            message: &message,
            help_message: Some(AUTO_SETUP_HINT),
            typed: Select {
                options: ["Yes", "No", "Show suggested modifications"].to_vec(),
            },
        };
        let (mut choice, _) = dialog.raw_prompt()?;

        if choice == 2 {
            let message = formatdoc! {"

                {}
                Would you like Flox to apply these modifications?
                You can always revisit the environment's declaration with 'flox edit'
            ", format_customization(&self.get_init_customization())?};

            let dialog = Dialog {
                message: &message,
                help_message: Some(AUTO_SETUP_HINT),
                typed: Select {
                    options: ["Yes", "No"].to_vec(),
                },
            };

            (choice, _) = dialog.raw_prompt()?;
        }
        Ok(choice == 0)
    }

    fn prompt_for_hook(&mut self, flox: &Flox) -> Result<bool> {
        let (nodejs_detected, nodejs_version, _) = self.nodejs_message_and_version(flox)?;
        let message = formatdoc! {"
            {nodejs_detected}
            Flox detected both a package-lock.json and a yarn.lock

            Flox can add the following to your environment:
            * nodejs{}
            * Either an npm or yarn installation hook

            Would you like Flox to apply one of these modifications?
            You can always revisit the environment's declaration with 'flox edit'", nodejs_version.map(|version| format!(" {version}")).unwrap_or("".to_string())};
        let options = [
            "Yes - with npm hook",
            "Yes - with yarn hook",
            "No",
            "Show modifications with npm hook",
            "Show modifications with yarn hook",
        ]
        .to_vec();

        let dialog = Dialog {
            message: &message,
            help_message: Some(AUTO_SETUP_HINT),
            typed: Select {
                options: options.clone(),
            },
        };

        let (mut choice, _) = dialog.raw_prompt()?;

        while choice == 3 || choice == 4 {
            // Temporarily set choice so self.get_init_customization() returns
            // the correct hook
            if choice == 3 {
                self.hook = Some(NodePackageManager::Npm)
            } else if choice == 4 {
                self.hook = Some(NodePackageManager::Yarn)
            }
            let message = formatdoc! {"

                {}
                Would you like Flox to apply one of these modifications?
                You can always revisit the environment's declaration with 'flox edit'
            ", format_customization(&self.get_init_customization())?};

            let dialog = Dialog {
                message: &message,
                help_message: Some(AUTO_SETUP_HINT),
                typed: Select {
                    options: options.clone(),
                },
            };

            (choice, _) = dialog.raw_prompt()?;
        }

        if choice == 0 {
            self.hook = Some(NodePackageManager::Npm)
        } else if choice == 1 {
            self.hook = Some(NodePackageManager::Yarn)
        }
        Ok(choice == 0 || choice == 1)
    }
}

impl InitHook for Node {
    fn should_run(&mut self, _path: &Path) -> Result<bool> {
        match self.get_action() {
            NodeAction::Install(_) => {
                debug!("Should run node init hook with requested nodejs version");
                Ok(true)
            },
            NodeAction::OfferFloxDefault => {
                debug!("Should run node init hook with default nodejs version");
                Ok(true)
            },
            NodeAction::Nothing => {
                debug!("Should not run node init hook");
                Ok(false)
            },
        }
    }

    fn prompt_user(&mut self, _path: &Path, flox: &Flox) -> Result<bool> {
        if let Some(NodePackageManager::Both) = self.hook {
            self.prompt_for_hook(flox)
        } else {
            self.prompt_with_known_hook(flox)
        }
    }

    fn get_init_customization(&self) -> InitCustomization {
        let nodejs_to_install = match self.get_action() {
            NodeAction::Install(result) => PackageToInstall {
                id: "nodejs".to_string(),
                pkg_path: result.rel_path.join("."),
                version: result.version,
                input: None,
            },
            NodeAction::OfferFloxDefault => PackageToInstall {
                id: "nodejs".to_string(),
                pkg_path: "nodejs".to_string(),
                version: None,
                input: None,
            },
            NodeAction::Nothing => unreachable!(),
        };

        let hook = match self.hook {
            None => None,
            Some(NodePackageManager::Npm) => Some("# TODO: npm hook".to_string()),
            Some(NodePackageManager::Yarn) => Some("# TODO: yarn hook".to_string()),
            // Default to npm
            Some(NodePackageManager::Both) => Some("# TODO: npm hook".to_string()),
        };

        InitCustomization {
            hook,
            packages: Some(vec![nodejs_to_install]),
        }
    }
}

/// Create a temporary TOML document containing just the contents of the passed
/// [InitCustomization],
/// and return it as a string.
fn format_customization(customization: &InitCustomization) -> Result<String> {
    let mut toml = if let Some(packages) = &customization.packages {
        let with_packages = insert_packages("", packages)?;
        with_packages.new_toml.unwrap_or(Document::new())
    } else {
        Document::new()
    };

    if let Some(hook) = &customization.hook {
        let hook_table = {
            let hook_field = toml
                .entry("hook")
                .or_insert_with(|| Item::Table(Table::new()));
            let hook_field_type = hook_field.type_name();
            hook_field.as_table_mut().context(format!(
                "'install' must be a table, but found {hook_field_type} instead"
            ))?
        };
        hook_table.insert(
            "script",
            Item::Value(Value::String(Formatted::new(formatdoc! {r#"
                {}
            "#, indent::indent_all_by(2, hook)}))),
        );
    }

    Ok(toml.to_string())
}

#[cfg(test)]
mod tests {
    use flox_rust_sdk::models::search::Subtree;
    use pretty_assertions::assert_eq;

    use super::*;

    /// combine_customizations() deduplicates a package and corretly concatenates hooks
    #[test]
    fn test_combine_customizations() {
        let customizations = vec![
            Requirements {}.get_init_customization(),
            InitCustomization {
                hook: Some("hook2".to_string()),
                packages: Some(vec![
                    PackageToInstall {
                        id: "pip".to_string(),
                        pkg_path: "python311Packages.pip".to_string(),
                        version: None,
                        input: None,
                    },
                    PackageToInstall {
                        id: "package1".to_string(),
                        pkg_path: "path1".to_string(),
                        version: None,
                        input: None,
                    },
                ]),
            },
        ];

        let mut combined = Init::combine_customizations(customizations);
        combined.packages.as_mut().unwrap().sort();
        assert_eq!(combined, InitCustomization {
            // Yes, this is incredibly brittle, but it's to make sure we get the newlines right
            hook: Some(
                indoc! {r#"
                        # Autogenerated by flox

                        # Setup a Python virtual environment

                        PYTHON_DIR="$FLOX_ENV_CACHE/python"
                        if [ ! -d "$PYTHON_DIR" ]; then
                          echo "Creating python virtual environment in $PYTHON_DIR"
                          python -m venv "$PYTHON_DIR"
                        fi

                        echo "Activating python virtual environment"
                        source "$PYTHON_DIR/bin/activate"

                        pip install -r "$FLOX_ENV_PROJECT/requirements.txt" --quiet

                        hook2

                        # End autogenerated by flox"#}
                .to_string()
            ),
            packages: Some(vec![
                PackageToInstall {
                    id: "package1".to_string(),
                    pkg_path: "path1".to_string(),
                    version: None,
                    input: None,
                },
                PackageToInstall {
                    id: "pip".to_string(),
                    pkg_path: "python311Packages.pip".to_string(),
                    version: None,
                    input: None,
                },
                PackageToInstall {
                    id: "python3".to_string(),
                    pkg_path: "python311Packages.python".to_string(),
                    version: None,
                    input: None,
                },
            ]),
        });
    }

    #[test]
    fn test_parse_nvmrc_version_some() {
        assert_eq!(
            Node::parse_nvmrc_version("v0.1.14"),
            RequestedNVMRCVersion::Some("0.1.14".to_string())
        );
        assert_eq!(
            Node::parse_nvmrc_version("v20.11.1"),
            RequestedNVMRCVersion::Some("20.11.1".to_string())
        );
        assert_eq!(
            Node::parse_nvmrc_version("0.1.14"),
            RequestedNVMRCVersion::Some("0.1.14".to_string())
        );
        assert_eq!(
            Node::parse_nvmrc_version("0"),
            RequestedNVMRCVersion::Some("0".to_string())
        );
        assert_eq!(
            Node::parse_nvmrc_version("0.1"),
            RequestedNVMRCVersion::Some("0.1".to_string())
        );
        assert_eq!(
            Node::parse_nvmrc_version("0.1.14\n"),
            RequestedNVMRCVersion::Some("0.1.14".to_string())
        );
        assert_eq!(
            Node::parse_nvmrc_version("0.1.14   "),
            RequestedNVMRCVersion::Some("0.1.14".to_string())
        );
    }

    #[test]
    fn test_parse_nvmrc_version_unsure() {
        assert_eq!(
            Node::parse_nvmrc_version("node"),
            RequestedNVMRCVersion::Unsure
        );
        assert_eq!(
            Node::parse_nvmrc_version("0.1.14 blah blah"),
            RequestedNVMRCVersion::Unsure
        );
    }

    #[test]
    fn test_parse_nvmrc_version_none() {
        assert_eq!(Node::parse_nvmrc_version(""), RequestedNVMRCVersion::None);
        assert_eq!(Node::parse_nvmrc_version("\n"), RequestedNVMRCVersion::None);
    }

    #[test]
    fn test_node_get_init_customization_install_action() {
        assert_eq!(
            Node {
                nvmrc_version: NVMRCVersion::Some(Box::new(SearchResult {
                    input: "".to_string(),
                    abs_path: vec![],
                    subtree: Subtree::LegacyPackages,
                    system: "".to_string(),
                    rel_path: vec!["pkg".to_string(), "path".to_string()],
                    pname: None,
                    version: Some("0.1.14".to_string()),
                    description: None,
                    broken: None,
                    unfree: None,
                    license: None,
                    id: 0,
                })),
                package_json_version: None,
                hook: None,
            }
            .get_init_customization(),
            InitCustomization {
                packages: Some(vec![PackageToInstall {
                    id: "nodejs".to_string(),
                    pkg_path: "pkg.path".to_string(),
                    version: Some("0.1.14".to_string()),
                    input: None,
                }]),
                hook: None,
            }
        );
    }

    #[test]
    fn test_node_get_init_customization_offer_flox_default_action() {
        assert_eq!(
            Node {
                nvmrc_version: NVMRCVersion::Unsure,
                package_json_version: Some(PackageJSONVersion::Unspecified),
                hook: None,
            }
            .get_init_customization(),
            InitCustomization {
                packages: Some(vec![PackageToInstall {
                    id: "nodejs".to_string(),
                    pkg_path: "nodejs".to_string(),
                    version: None,
                    input: None,
                }]),
                hook: None,
            }
        );
    }
}
