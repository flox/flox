use std::fs;
use std::path::Path;

use anyhow::{anyhow, Result};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::global_manifest_path;
use flox_rust_sdk::models::environment::path_environment::InitCustomization;
use flox_rust_sdk::models::lockfile::LockedManifest;
use flox_rust_sdk::models::manifest::PackageToInstall;
use flox_rust_sdk::models::search::{do_search, PathOrJson, Query, SearchParams, SearchResult};
use indoc::{formatdoc, indoc};
use log::debug;
use semver::VersionReq;

use super::{format_customization, InitHook, AUTO_SETUP_HINT};
use crate::config::features::Features;
use crate::utils::dialog::{Dialog, Select};

const NPM_HOOK: &str = indoc! {r#"
                # Install nodejs depedencies
                npm install"#};

const YARN_HOOK: &str = indoc! {r#"
                # Install nodejs depedencies
                yarn"#};

pub(super) struct Node {
    /// Whether npm or yarn should be installed.
    ///
    ///
    /// If initially set to [PackageManager::Npm],
    /// it will be set to either [PackageManager::Npm] or [PackageManager::Yarn]
    /// after prompting the user.
    package_manager: Option<PackageManager>,
    /// Node version as specified in package.json if it exists
    /// [PackageJSONVersion::None] if we found a compatible npm or yarn,
    package_json_node_version: PackageJSONVersion,
    /// Node version as specified in .nvmrc if it exists
    /// [None] if we found a version in package.json
    nvmrc_version: Option<NVMRCVersion>,
}

/// This stores which lockfiles are present and which of nixpkgs provided npm
/// and yarn are compatible with package.json
///
/// After prompting the user, it is set to either [PackageManager::Npm]
/// or [PackageManager::Yarn] even if either package manager is compatible.
#[derive(Debug, PartialEq)]
enum PackageManager {
    /// package.json is present,
    /// and nixpkgs provides npm and nodejs compatible with package.json
    ///
    /// Contains a [SearchResult] for npm and nodejs
    Npm(SearchResult, SearchResult),
    /// yarn.lock is present,
    /// and nixpkgs provides yarn and nodejs compatible with package.json
    ///
    /// Contains a [SearchResult] for yarn and nodejs
    Yarn(SearchResult, SearchResult),
    /// Both yarn.lock and package-lock.json are present,
    /// and nixpkgs provides npm, yarn, and nodejs compatible with package.json
    ///
    /// Contains a [SearchResult] for npm, yarn, and nodejs
    Both(SearchResult, SearchResult, Box<SearchResult>),
}

#[derive(Debug, PartialEq)]
enum RequestedNVMRCVersion {
    /// .nvmrc not present or empty
    None,
    /// .nvmrc contains an alias or something we can't parse as a version.
    Unsure,
    Found(String),
}

enum NVMRCVersion {
    /// .nvmrc contains a version,
    /// but flox doesn't provide it.
    Unavailable,
    /// .nvmrc contains an alias or something we can't parse as a version.
    Unsure,
    Some(Box<SearchResult>),
}

enum PackageJSONVersion {
    /// We didn't check for package.json,
    /// or it does not exist or is invalid.
    None,
    /// package.json exists but doesn't specify a version
    Unspecified,
    Unavailable,
    Found(Box<SearchResult>),
}

enum NodeAction {
    Install(Box<SearchResult>),
    OfferFloxDefault,
    Nothing,
}

struct PackageJSONVersions {
    npm: Option<String>,
    yarn: Option<String>,
    node: Option<String>,
}

impl Node {
    pub fn new(path: &Path, flox: &Flox) -> Result<Self> {
        // Get value for self.package_manager
        let versions = Self::get_package_json_versions(path)?;
        let package_manager = match versions {
            None => None,
            Some(ref versions) => {
                let package_lock_exists = path.join("package-lock.json").exists();
                let yarn_lock_exists = path.join("yarn.lock").exists();
                Self::get_package_manager(versions, package_lock_exists, yarn_lock_exists, flox)?
            },
        };

        // Get value for self.package_json_node_version if we didn't find a valid npm or yarn
        let package_json_node_version = if package_manager.is_some() {
            PackageJSONVersion::None
        } else {
            match versions {
                Some(PackageJSONVersions {
                    node: Some(node_version),
                    ..
                }) => match Self::try_find_compatible_version("nodejs", &node_version, None, flox)?
                {
                    None => PackageJSONVersion::Unavailable,
                    Some(result) => PackageJSONVersion::Found(Box::new(result)),
                },
                Some(_) => PackageJSONVersion::Unspecified,
                _ => PackageJSONVersion::None,
            }
        };

        // Get value for self.nvmrc_version if we didn't find a valid npm or yarn
        let nvmrc_version = if package_manager.is_some() {
            None
        } else {
            match package_json_node_version {
                // package.json is higher priority than .nvmrc,
                // so don't check .nvmrc if we know we'll use the version in
                // package.json or we know we can't provide it
                PackageJSONVersion::Found(_) | PackageJSONVersion::Unavailable => None,
                _ => Self::get_nvmrc_version(path, flox)?,
            }
        };

        Ok(Self {
            package_json_node_version,
            nvmrc_version,
            package_manager,
        })
    }

    /// Look for nodejs, npm, and yarn versions in a (possibly non-existent)
    /// `package.json` file
    fn get_package_json_versions(path: &Path) -> Result<Option<PackageJSONVersions>> {
        let package_json = path.join("package.json");
        if !package_json.exists() {
            return Ok(None);
        }
        let package_json_contents = fs::read_to_string(package_json)?;
        match serde_json::from_str::<serde_json::Value>(&package_json_contents) {
            // Treat a package.json that can't be parsed as JSON the same as it not existing
            Err(_) => Ok(None),
            Ok(package_json_json) => {
                let node = package_json_json["engines"]["node"]
                    .as_str()
                    .map(|s| s.to_string());
                let npm = package_json_json["engines"]["npm"]
                    .as_str()
                    .map(|s| s.to_string());
                let yarn = package_json_json["engines"]["yarn"]
                    .as_str()
                    .map(|s| s.to_string());
                Ok(Some(PackageJSONVersions { node, npm, yarn }))
            },
        }
    }

    /// Try to find node, npm, and yarn versions that satisfy constraints in
    /// package.json
    fn get_package_manager(
        versions: &PackageJSONVersions,
        package_lock_exists: bool,
        yarn_lock_exists: bool,
        flox: &Flox,
    ) -> Result<Option<PackageManager>> {
        let PackageJSONVersions { npm, yarn, node } = versions;

        let found_node = match node {
            Some(node_version) => {
                match Self::get_default_node_if_compatible(Some(node_version.clone()), flox)? {
                    // If the corresponding node isn't compatible, don't install a package manager
                    None => return Ok(None),
                    Some(found_node) => found_node,
                }
            },
            None => Self::get_default_node_if_compatible(None, flox)?
                .ok_or(anyhow!("Flox couldn't find nodejs in nixpkgs"))?,
        };

        // We assume that yarn and npm are built with found_node, which is
        // currently true in nixpkgs
        let found_npm = if !yarn_lock_exists || package_lock_exists {
            match npm {
                Some(npm_version) => Self::try_find_compatible_version(
                    "npm",
                    npm_version,
                    Some(vec!["nodePackages".to_string(), "npm".to_string()]),
                    flox,
                )?,
                _ => Some(Self::get_default_package("nodePackages.npm", flox)?),
            }
        } else {
            None
        };

        let found_yarn = if yarn_lock_exists {
            match yarn {
                Some(yarn_version) => {
                    Self::try_find_compatible_version("yarn", yarn_version, None, flox)?
                },
                _ => Some(Self::get_default_package("yarn", flox)?),
            }
        } else {
            None
        };

        let package_manager = {
            match (found_npm, found_yarn) {
                (None, None) => None,
                (Some(found_npm), None) => Some(PackageManager::Npm(found_npm, found_node)),
                (None, Some(found_yarn)) => Some(PackageManager::Yarn(found_yarn, found_node)),
                (Some(found_npm), Some(found_yarn)) => Some(PackageManager::Both(
                    found_npm,
                    found_yarn,
                    Box::new(found_node),
                )),
            }
        };
        Ok(package_manager)
    }

    /// Determine appropriate [NVMRCVersion] variant for a (possibly
    /// non-existent) `.nvmrc` file in `path`.
    ///
    /// This will perform a search to determine if a requested version is
    /// available.
    fn get_nvmrc_version(path: &Path, flox: &Flox) -> Result<Option<NVMRCVersion>> {
        let nvmrc = path.join(".nvmrc");
        if !nvmrc.exists() {
            return Ok(None);
        }

        let nvmrc_contents = fs::read_to_string(&nvmrc)?;
        let nvmrc_version = match Self::parse_nvmrc_version(&nvmrc_contents) {
            RequestedNVMRCVersion::None => None,
            RequestedNVMRCVersion::Unsure => Some(NVMRCVersion::Unsure),
            RequestedNVMRCVersion::Found(version) => {
                match Self::try_find_compatible_version("nodejs", &version, None, flox)? {
                    None => Some(NVMRCVersion::Unavailable),
                    Some(result) => Some(NVMRCVersion::Some(Box::new(result))),
                }
            },
        };
        Ok(nvmrc_version)
    }

    /// Translate the contents of a `.nvmrc` file into a [RequestedNVMRCVersion]
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
                        RequestedNVMRCVersion::Found(trimmed_first_line[1..].to_string())
                    },
                    _ if VersionReq::parse(trimmed_first_line).is_ok() => {
                        RequestedNVMRCVersion::Found(trimmed_first_line.to_string())
                    },
                    _ => RequestedNVMRCVersion::Unsure,
                }
            },
        }
    }

    /// Searches for a given pname and version, optionally restricting rel_path
    fn try_find_compatible_version(
        pname: &str,
        version: &str,
        rel_path: Option<Vec<String>>,
        flox: &Flox,
    ) -> Result<Option<SearchResult>> {
        let query = Query {
            pname: Some(pname.to_string()),
            semver: Some(version.to_string()),
            limit: Some(1),
            deduplicate: false,
            rel_path,
            ..Default::default()
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

    /// Get nixpkgs#nodejs,
    /// optionally verifying that it satisfies a version constraint.
    fn get_default_node_if_compatible(
        version: Option<String>,
        flox: &Flox,
    ) -> Result<Option<SearchResult>> {
        let query = Query {
            rel_path: Some(vec!["nodejs".to_string()]),
            semver: version,
            limit: Some(1),
            deduplicate: false,
            ..Default::default()
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

    /// Get a package as if installed with `flox install {package}`
    fn get_default_package(package: &str, flox: &Flox) -> Result<SearchResult> {
        let query = Query::new(package, Features::parse()?.search_strategy, Some(1), false)?;
        let params = SearchParams {
            manifest: None,
            global_manifest: PathOrJson::Path(global_manifest_path(flox)),
            lockfile: PathOrJson::Path(LockedManifest::ensure_global_lockfile(flox)?),
            query,
        };

        let (mut results, _) = do_search(&params)?;

        if results.results.is_empty() {
            Err(anyhow!("Flox couldn't find any versions of {package}"))?
        }
        Ok(results.results.swap_remove(0))
    }

    /// Return whether to skip the nodejs hook entirely, install a requested
    /// version of nodejs, or offer to install the Flox default version of
    /// nodejs.
    ///
    /// This is decided based on whether .nvmrc and package.json are present,
    /// and whether Flox can provide versions they request.
    fn get_node_action(&self) -> NodeAction {
        match (&self.package_json_node_version, self.nvmrc_version.as_ref()) {
            // package.json takes precedence over .nvmrc
            (PackageJSONVersion::Found(result), _) => NodeAction::Install(result.clone()),
            // Treat the version in package.json strictly; if we can't find it, don't suggest something else.
            (PackageJSONVersion::Unavailable, _) => NodeAction::Nothing,
            (_, Some(NVMRCVersion::Some(result))) => NodeAction::Install(result.clone()),
            (_, Some(NVMRCVersion::Unsure)) => NodeAction::OfferFloxDefault,
            (_, Some(NVMRCVersion::Unavailable)) => NodeAction::OfferFloxDefault,
            (PackageJSONVersion::Unspecified, None) => NodeAction::OfferFloxDefault,
            (PackageJSONVersion::None, None) => NodeAction::Nothing,
        }
    }

    /// Returns:
    /// 1. A message describing what version of nodejs Flox found requested to
    /// include in the prompt.
    /// 2. The version of nodejs Flox would install
    ///
    /// Any case that get_action() would return NodeAction::Nothing for is unreachable
    fn nodejs_message_and_version(&self, flox: &Flox) -> Result<(String, Option<String>)> {
        let (message, version) = match (
            &self.package_json_node_version,
            self.nvmrc_version.as_ref(),
        ) {
            // package.json takes precedence over .nvmrc
            (PackageJSONVersion::Found(result), _) => {
                let message = format!(
                    "Flox detected a package.json compatible with node{}",
                    result
                        .version
                        .as_ref()
                        .map(|version| format!(" {version}"))
                        .unwrap_or("".to_string())
                );
                (message, result.version.clone())
            },
            // Treat the version in package.json strictly; if we can't find it, don't suggest something else.
            // get_action() returns NodeAction::Nothing for this case so it's unreachable
            (PackageJSONVersion::Unavailable, _) => unreachable!(),
            (_, Some(NVMRCVersion::Some(result))) => {
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
            (_, Some(NVMRCVersion::Unsure)) => {
                let result = Self::get_default_package("nodejs", flox)?;
                let message = format!("Flox detected an .nvmrc with a version specifier not understood by Flox, but Flox can provide {}",
                       result.version.as_ref().map(|version|format!("version {version}")).unwrap_or("another version".to_string()));
                (message, result.version)
            },
            (_, Some(NVMRCVersion::Unavailable)) => {
                let result = Self::get_default_package("nodejs", flox)?;
                let message = format!("Flox detected an .nvmrc with a version of nodejs not provided by Flox, but Flox can provide {}",
                result.version.as_ref().map(|version|format!("version {version}")).unwrap_or("another version".to_string()));
                (message, result.version.clone())
            },
            (PackageJSONVersion::Unspecified, None) => {
                let result = Self::get_default_package("nodejs", flox)?;
                ("Flox detected a package.json".to_string(), result.version)
            },
            // get_action() returns NodeAction::Nothing for this case so it's unreachable
            (PackageJSONVersion::None, None) => unreachable!(),
        };
        Ok((message, version))
    }

    /// Prompt whether to install nodejs (but not npm or yarn)
    fn prompt_for_node(&self, flox: &Flox) -> Result<bool> {
        let (nodejs_detected, nodejs_version) = self.nodejs_message_and_version(flox)?;
        let mut message = format!("{nodejs_detected}\n");

        message.push_str(&formatdoc! {"

            Flox can add the following to your environment:
            * nodejs{}
        ",
            nodejs_version
                .map(|version| format!(" {version}"))
                .unwrap_or("".to_string()),
        });

        message.push_str(&formatdoc! {"

            Would you like Flox to apply this suggestion?
            You can always change the environment's manifest with 'flox edit'
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
                You can always change the environment's manifest with 'flox edit'
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

    /// Prompt whether to install npm or yarn when only one of them is viable
    fn prompt_with_known_package_manager(&self) -> Result<bool> {
        let mut message = match &self.package_manager {
            Some(PackageManager::Npm(found_npm, found_node)) => {
                let npm_version = found_npm
                    .version
                    .as_ref()
                    .map(|version| format!(" {version}"))
                    .unwrap_or("".to_string());
                let node_version = found_node
                    .version
                    .as_ref()
                    .map(|version| format!(" {version}"))
                    .unwrap_or("".to_string());
                formatdoc! {"
                    Flox detected a package.json

                    Flox can add the following to your environment:
                    * npm{npm_version} with nodejs{node_version} bundled
                    * An npm installation hook
                "}
            },

            Some(PackageManager::Yarn(found_yarn, found_node)) => {
                let yarn_version = found_yarn
                    .version
                    .as_ref()
                    .map(|version| format!(" {version}"))
                    .unwrap_or("".to_string());
                let node_version = found_node
                    .version
                    .as_ref()
                    .map(|version| format!(" {version}"))
                    .unwrap_or("".to_string());

                formatdoc! {"
                    Flox detected a package.json and a yarn.lock

                    Flox can add the following to your environment:
                    * yarn{yarn_version} with nodejs{node_version} bundled
                    * A yarn installation hook
                "}
            },
            None | Some(PackageManager::Both(..)) => unreachable!(),
        };

        message.push_str(&formatdoc! {"

            Would you like Flox to apply this suggestion?
            You can always change the environment's manifest with 'flox edit'
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
                You can always change the environment's manifest with 'flox edit'
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

    /// Prompt whether to install npm or yarn when either is viable
    fn prompt_for_package_manager(&mut self) -> Result<bool> {
        let (found_npm, found_yarn, found_node) = match &self.package_manager {
            Some(PackageManager::Both(found_npm, found_yarn, found_node)) => {
                (found_npm.clone(), found_yarn.clone(), found_node.clone())
            },
            _ => unreachable!(),
        };
        let npm_version = found_npm
            .version
            .as_ref()
            .map(|version| format!(" {version}"))
            .unwrap_or("".to_string());
        let yarn_version = found_yarn
            .version
            .as_ref()
            .map(|version| format!(" {version}"))
            .unwrap_or("".to_string());
        let node_version = found_node
            .version
            .as_ref()
            .map(|version| format!(" {version}"))
            .unwrap_or("".to_string());

        let message = formatdoc! {"
            Flox detected both a package-lock.json and a yarn.lock

            Flox can add the following to your environment:
            * Either npm{npm_version} or yarn{yarn_version} (both have nodejs{node_version} bundled)
            * Either an npm or yarn installation hook

            Would you like Flox to apply one of these modifications?
            You can always change the environment's manifest with 'flox edit'"};
        let options = [
            "Yes - with npm",
            "Yes - with yarn",
            "No",
            "Show modifications with npm",
            "Show modifications with yarn",
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
                self.package_manager =
                    Some(PackageManager::Npm(found_npm.clone(), *found_node.clone()))
            } else if choice == 4 {
                self.package_manager = Some(PackageManager::Yarn(
                    found_yarn.clone(),
                    *found_node.clone(),
                ))
            }
            let message = formatdoc! {"

                {}
                Would you like Flox to apply one of these modifications?
                You can always change the environment's manifest with 'flox edit'
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
            self.package_manager = Some(PackageManager::Npm(found_npm.clone(), *found_node.clone()))
        } else if choice == 1 {
            self.package_manager = Some(PackageManager::Yarn(
                found_yarn.clone(),
                *found_node.clone(),
            ))
        }
        Ok(choice == 0 || choice == 1)
    }
}

impl InitHook for Node {
    fn should_run(&mut self, _path: &Path) -> Result<bool> {
        match self.package_manager {
            Some(PackageManager::Both(..)) => {
                debug!("Should run node init hook. Both npm and yarn detected.");
                return Ok(true);
            },
            Some(PackageManager::Npm(..)) => {
                debug!("Should run node init hook and install npm.");
                return Ok(true);
            },
            Some(PackageManager::Yarn(..)) => {
                debug!("Should run node init hook and install yarn.");
                return Ok(true);
            },
            None => {},
        }

        match self.get_node_action() {
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
        match self.package_manager {
            Some(PackageManager::Both(..)) => self.prompt_for_package_manager(),
            Some(PackageManager::Npm(..)) | Some(PackageManager::Yarn(..)) => {
                self.prompt_with_known_package_manager()
            },
            None => self.prompt_for_node(flox),
        }
    }

    fn get_init_customization(&self) -> InitCustomization {
        let mut packages = vec![];

        let hook = match &self.package_manager {
            None => None,
            // Default to npm for Some(PackageManager::Both)
            // This is only reachable if --auto-setup is used.
            Some(PackageManager::Npm(found_npm, _)) | Some(PackageManager::Both(found_npm, ..)) => {
                packages.extend(vec![PackageToInstall {
                    id: "npm".to_string(),
                    pkg_path: found_npm.rel_path.join("."),
                    // TODO: we probably shouldn't pin this when we're just
                    // providing the default
                    version: found_npm.version.clone(),
                    input: None,
                }]);
                Some(NPM_HOOK.to_string())
            },
            Some(PackageManager::Yarn(found_yarn, _)) => {
                packages.extend(vec![PackageToInstall {
                    id: "yarn".to_string(),
                    pkg_path: found_yarn.rel_path.join("."),
                    // TODO: we probably shouldn't pin this when we're just
                    // providing the default
                    version: found_yarn.version.clone(),
                    input: None,
                }]);
                Some(YARN_HOOK.to_string())
            },
        };

        if self.package_manager.is_none() {
            let nodejs_to_install = match self.get_node_action() {
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
            packages.push(nodejs_to_install);
        }

        InitCustomization {
            hook,
            packages: Some(packages),
        }
    }
}

#[cfg(test)]
mod tests {
    use flox_rust_sdk::flox::test_flox_instance;
    use flox_rust_sdk::models::search::Subtree;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn test_parse_nvmrc_version_some() {
        assert_eq!(
            Node::parse_nvmrc_version("v0.1.14"),
            RequestedNVMRCVersion::Found("0.1.14".to_string())
        );
        assert_eq!(
            Node::parse_nvmrc_version("v20.11.1"),
            RequestedNVMRCVersion::Found("20.11.1".to_string())
        );
        assert_eq!(
            Node::parse_nvmrc_version("0.1.14"),
            RequestedNVMRCVersion::Found("0.1.14".to_string())
        );
        assert_eq!(
            Node::parse_nvmrc_version("0"),
            RequestedNVMRCVersion::Found("0".to_string())
        );
        assert_eq!(
            Node::parse_nvmrc_version("0.1"),
            RequestedNVMRCVersion::Found("0.1".to_string())
        );
        assert_eq!(
            Node::parse_nvmrc_version("0.1.14\n"),
            RequestedNVMRCVersion::Found("0.1.14".to_string())
        );
        assert_eq!(
            Node::parse_nvmrc_version("0.1.14   "),
            RequestedNVMRCVersion::Found("0.1.14".to_string())
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
    fn test_get_init_customization_install_action() {
        assert_eq!(
            Node {
                package_json_node_version: PackageJSONVersion::Found(Box::new(SearchResult {
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
                nvmrc_version: None,
                package_manager: None,
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
    fn test_get_init_customization_offer_flox_default_action() {
        assert_eq!(
            Node {
                package_json_node_version: PackageJSONVersion::Unspecified,
                nvmrc_version: Some(NVMRCVersion::Unsure),
                package_manager: None,
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

    #[test]
    fn test_get_init_customization_npm_hook() {
        assert_eq!(
            Node {
                package_json_node_version: PackageJSONVersion::None,
                nvmrc_version: None,
                package_manager: Some(PackageManager::Npm(
                    SearchResult {
                        input: "".to_string(),
                        abs_path: vec![],
                        subtree: Subtree::LegacyPackages,
                        system: "".to_string(),
                        rel_path: vec!["npm".to_string(), "path".to_string()],
                        pname: None,
                        version: Some("1".to_string()),
                        description: None,
                        broken: None,
                        unfree: None,
                        license: None,
                        id: 0,
                    },
                    SearchResult {
                        input: "".to_string(),
                        abs_path: vec![],
                        subtree: Subtree::LegacyPackages,
                        system: "".to_string(),
                        rel_path: vec!["nodejs".to_string(), "path".to_string()], // should be disregarded
                        pname: None,
                        version: Some("0.1.14".to_string()), // should be disregarded
                        description: None,
                        broken: None,
                        unfree: None,
                        license: None,
                        id: 0,
                    },
                )),
            }
            .get_init_customization(),
            InitCustomization {
                packages: Some(vec![
                    PackageToInstall {
                        id: "npm".to_string(),
                        pkg_path: "npm.path".to_string(),
                        version: Some("1".to_string()),
                        input: None,
                    }]),
                hook: Some(NPM_HOOK.to_string()),
            }
        );
    }

    // TODO: all the get_package_manager() tests actually hit the database,
    // and it might be better to mock out do_search().
    // But I'm only seeing 11 tests take 1-1.5 seconds,
    // so at this point I think there are bigger testing efficiency fish to fry.

    fn flox_instance_with_locked_global_manifest() -> (Flox, TempDir) {
        let (flox, _temp_dir_handle) = test_flox_instance();
        let pkgdb_nixpkgs_rev_new = "ab5fd150146dcfe41fda501134e6503932cc8dfd";
        std::env::set_var("_PKGDB_GA_REGISTRY_REF_OR_REV", pkgdb_nixpkgs_rev_new);
        LockedManifest::update_global_manifest(&flox, vec![]).unwrap();
        (flox, _temp_dir_handle)
    }

    #[test]
    fn test_get_package_manager_none() {
        let (flox, _temp_dir_handle) = flox_instance_with_locked_global_manifest();
        let package_manager = Node::get_package_manager(
            &PackageJSONVersions {
                npm: None,
                yarn: None,
                node: None,
            },
            false,
            false,
            &flox,
        )
        .unwrap()
        .unwrap();
        let (found_npm, found_node) = match package_manager {
            PackageManager::Npm(found_npm, found_node) => (found_npm, found_node),
            _ => panic!(),
        };
        assert_eq!(found_node.rel_path, vec!["nodejs".to_string()]);
        assert_eq!(found_npm.rel_path, vec![
            "nodePackages".to_string(),
            "npm".to_string()
        ]);
    }

    /// Test get_package_manager() when node has a version specified
    #[test]
    fn test_get_package_manager_node() {
        let (flox, _temp_dir_handle) = flox_instance_with_locked_global_manifest();
        let package_manager = Node::get_package_manager(
            &PackageJSONVersions {
                npm: None,
                yarn: None,
                node: Some("18".to_string()),
            },
            false,
            false,
            &flox,
        )
        .unwrap()
        .unwrap();
        let (found_npm, found_node) = match package_manager {
            PackageManager::Npm(found_npm, found_node) => (found_npm, found_node),
            _ => panic!(),
        };
        assert_eq!(found_node.rel_path, vec!["nodejs".to_string()]);
        assert!(found_node.version.unwrap().starts_with("18"));
        assert_eq!(found_npm.rel_path, vec![
            "nodePackages".to_string(),
            "npm".to_string()
        ]);
    }

    /// Test get_package_manager() when node and npm have versions specified
    #[test]
    fn test_get_package_manager_node_and_npm() {
        let (flox, _temp_dir_handle) = flox_instance_with_locked_global_manifest();
        let package_manager = Node::get_package_manager(
            &PackageJSONVersions {
                npm: Some("10".to_string()),
                yarn: None,
                node: Some("18".to_string()),
            },
            false,
            false,
            &flox,
        )
        .unwrap()
        .unwrap();
        let (found_npm, found_node) = match package_manager {
            PackageManager::Npm(found_npm, found_node) => (found_npm, found_node),
            _ => panic!(),
        };
        assert_eq!(found_node.rel_path, vec!["nodejs".to_string()]);
        assert!(found_node.version.unwrap().starts_with("18"));
        assert_eq!(found_npm.rel_path, vec![
            "nodePackages".to_string(),
            "npm".to_string()
        ]);
        assert!(found_npm.version.unwrap().starts_with("10"));
    }

    /// Test get_package_manager() when node has a version specified not
    /// compatible with the nixpkgs npm
    #[test]
    fn test_get_package_manager_node_and_npm_unavailable() {
        let (flox, _temp_dir_handle) = flox_instance_with_locked_global_manifest();
        let package_manager = Node::get_package_manager(
            &PackageJSONVersions {
                npm: Some("10".to_string()),
                yarn: None,
                node: Some("20".to_string()),
            },
            false,
            false,
            &flox,
        )
        .unwrap();
        assert_eq!(package_manager, None);
    }

    /// Test get_package_manager() when node has a version specified not
    /// compatible with the nixpkgs npm, even if the version of npm is not
    /// specified.
    #[test]
    fn test_get_package_manager_node_not_compatible() {
        let (flox, _temp_dir_handle) = flox_instance_with_locked_global_manifest();
        let package_manager = Node::get_package_manager(
            &PackageJSONVersions {
                npm: None,
                yarn: None,
                node: Some("20".to_string()),
            },
            false,
            false,
            &flox,
        )
        .unwrap();
        assert_eq!(package_manager, None);
    }

    /// Test get_package_manager() when a yarn.lock is present
    #[test]
    fn test_get_package_manager_yarn_lock() {
        let (flox, _temp_dir_handle) = flox_instance_with_locked_global_manifest();
        let package_manager = Node::get_package_manager(
            &PackageJSONVersions {
                npm: None,
                yarn: None,
                node: Some("18".to_string()),
            },
            false,
            true,
            &flox,
        )
        .unwrap()
        .unwrap();
        let (found_yarn, found_node) = match package_manager {
            PackageManager::Yarn(found_yarn, found_node) => (found_yarn, found_node),
            _ => panic!(),
        };
        assert_eq!(found_node.rel_path, vec!["nodejs".to_string()]);
        assert!(found_node.version.unwrap().starts_with("18"));
        assert_eq!(found_yarn.rel_path, vec!["yarn".to_string()]);
        assert_eq!(found_yarn.version.unwrap(), "1.22.19");
    }

    /// Test get_package_manager() when node has a version specified not
    /// compatible with the nixpkgs yarn, even if the version of yarn is not
    /// specified.
    #[test]
    fn test_get_package_manager_node_not_compatible_with_yarn() {
        let (flox, _temp_dir_handle) = flox_instance_with_locked_global_manifest();
        let package_manager = Node::get_package_manager(
            &PackageJSONVersions {
                npm: None,
                yarn: None,
                node: Some("20".to_string()),
            },
            false,
            true,
            &flox,
        )
        .unwrap();
        assert_eq!(package_manager, None);
    }

    /// Test get_package_manager() when the yarn version requested does not exist
    #[test]
    fn test_get_package_manager_yarn_incompatible() {
        let (flox, _temp_dir_handle) = flox_instance_with_locked_global_manifest();
        let package_manager = Node::get_package_manager(
            &PackageJSONVersions {
                npm: None,
                yarn: Some("2".to_string()),
                node: None,
            },
            false,
            true,
            &flox,
        )
        .unwrap();
        assert_eq!(package_manager, None);
    }

    /// Test get_package_manager() when both locks are present
    #[test]
    fn test_get_package_manager_both_locks() {
        let (flox, _temp_dir_handle) = flox_instance_with_locked_global_manifest();
        let package_manager = Node::get_package_manager(
            &PackageJSONVersions {
                npm: None,
                yarn: None,
                node: None,
            },
            true,
            true,
            &flox,
        )
        .unwrap();
        let (found_npm, found_yarn, found_node) = match package_manager {
            Some(PackageManager::Both(found_npm, found_yarn, found_node)) => {
                (found_npm, found_yarn, found_node)
            },
            _ => panic!(),
        };
        assert_eq!(found_npm.rel_path, vec![
            "nodePackages".to_string(),
            "npm".to_string()
        ]);
        assert!(found_npm.version.unwrap().starts_with("10"));
        assert_eq!(found_node.rel_path, vec!["nodejs".to_string()]);
        assert!(found_node.version.unwrap().starts_with("18"));
        assert_eq!(found_yarn.rel_path, vec!["yarn".to_string()]);
        assert_eq!(found_yarn.version.unwrap(), "1.22.19");
    }

    /// Test get_package_manager() when both locks are present and the version of npm cannot be provided
    #[test]
    fn test_get_package_manager_both_locks_unavailable_npm() {
        let (flox, _temp_dir_handle) = flox_instance_with_locked_global_manifest();
        let package_manager = Node::get_package_manager(
            &PackageJSONVersions {
                npm: Some("11".to_string()),
                yarn: None,
                node: None,
            },
            true,
            true,
            &flox,
        )
        .unwrap();
        let (found_yarn, found_node) = match package_manager {
            Some(PackageManager::Yarn(found_yarn, found_node)) => (found_yarn, found_node),
            _ => panic!(),
        };
        assert_eq!(found_node.rel_path, vec!["nodejs".to_string()]);
        assert!(found_node.version.unwrap().starts_with("18"));
        assert_eq!(found_yarn.rel_path, vec!["yarn".to_string()]);
        assert_eq!(found_yarn.version.unwrap(), "1.22.19");
    }

    /// Test get_package_manager() when both locks are present but requested
    /// versions of both npm and yarn are unavailable
    #[test]
    fn test_get_package_manager_both_locks_both_unavailable() {
        let (flox, _temp_dir_handle) = flox_instance_with_locked_global_manifest();
        let package_manager = Node::get_package_manager(
            &PackageJSONVersions {
                npm: Some("11".to_string()),
                yarn: Some("2".to_string()),
                node: None,
            },
            true,
            true,
            &flox,
        )
        .unwrap();
        assert_eq!(package_manager, None);
    }
}
