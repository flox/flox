use std::fs;
use std::path::Path;

use anyhow::{anyhow, Result};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::path_environment::InitCustomization;
use flox_rust_sdk::models::environment::{global_manifest_lockfile_path, global_manifest_path};
use flox_rust_sdk::models::lockfile::LockedManifest;
use flox_rust_sdk::models::manifest::PackageToInstall;
use flox_rust_sdk::models::search::{do_search, PathOrJson, Query, SearchParams, SearchResult};
use indoc::formatdoc;
use log::debug;
use semver::VersionReq;

use super::{format_customization, InitHook, AUTO_SETUP_HINT};
use crate::config::features::Features;
use crate::utils::dialog::{Dialog, Select};

pub(super) struct Node {
    /// Node version as specified in package.json if it exists
    package_json_version: PackageJSONVersion,
    /// Node version as specified in .nvmrc if it exists
    /// [None] if we found a version in package.json
    nvmrc_version: Option<NVMRCVersion>,
    /// Whether a hook for `npm` or `yarn` should be generated
    /// This is initially set to whether package-lock.json and yarn.lock are
    /// present.
    /// If both are present and the user can be prompted, it will then be set to
    /// either [NodePackageManager::Npm] or [NodePackageManager::Yarn]
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
    Found(String),
}

enum NVMRCVersion {
    /// .nvmrc contains a version,
    /// but flox doesn't provide it.
    Unavailable,
    /// .nvmrc contains an alias or something we can't parse as a version.
    Unsure,
    Found(Box<SearchResult>),
}

enum PackageJSONVersion {
    /// package.json does not exist
    None,
    /// package.json exists but doesn't have an engines.node field
    Unspecified,
    Unavailable,
    Found(Box<SearchResult>),
}

enum NodeAction {
    Install(Box<SearchResult>),
    OfferFloxDefault,
    Nothing,
}

impl Node {
    pub fn new(path: &Path, flox: &Flox) -> Result<Self> {
        // Get value for self.package_json_version
        let package_json_version = Self::get_package_json_version(path, flox)?;

        // Get value for self.nvmrc_version
        let nvmrc_version = match package_json_version {
            PackageJSONVersion::Found(_) | PackageJSONVersion::Unavailable => None,
            _ => Self::get_nvmrc_version(path, flox)?,
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
                match Self::try_find_compatible_nodejs_version(&version, flox)? {
                    None => Some(NVMRCVersion::Unavailable),
                    Some(result) => Some(NVMRCVersion::Found(Box::new(result))),
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
                Some(version) => match Self::try_find_compatible_nodejs_version(version, flox)? {
                    None => Ok(PackageJSONVersion::Unavailable),
                    Some(result) => Ok(PackageJSONVersion::Found(Box::new(result))),
                },
                None => Ok(PackageJSONVersion::Unspecified),
            },
        }
    }

    fn try_find_compatible_nodejs_version(
        version: &str,
        flox: &Flox,
    ) -> Result<Option<SearchResult>> {
        let query = Query {
            pname: Some("nodejs".to_string()),
            semver: Some(version.to_string()),
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

    /// Return whether to skip the nodejs hook entirely, install a requested
    /// version of nodejs, or offer to install the Flox default version of
    /// nodejs.
    ///
    /// This is decided based on whether .nvmrc and package.json are present,
    /// and whether Flox can provide versions they request.
    fn get_action(&self) -> NodeAction {
        match (&self.package_json_version, self.nvmrc_version.as_ref()) {
            // package.json takes precedence over .nvmrc
            (PackageJSONVersion::Found(result), _) => NodeAction::Install(result.clone()),
            // Treat the version in package.json strictly; if we can't find it, don't suggest something else.
            (PackageJSONVersion::Unavailable, _) => NodeAction::Nothing,
            (_, Some(NVMRCVersion::Found(result))) => NodeAction::Install(result.clone()),
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
    /// 3. Whether the message says Flox detected package.json (to avoid
    ///    printing that message twice)
    ///
    /// Any case that get_action() would return NodeAction::Nothing for is unreachable
    fn nodejs_message_and_version(&self, flox: &Flox) -> Result<(String, Option<String>, bool)> {
        let mut mentions_package_json = false;
        let (message, version) = match (&self.package_json_version, self.nvmrc_version.as_ref()) {
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
                mentions_package_json = true;
                (message, result.version.clone())
            },
            // Treat the version in package.json strictly; if we can't find it, don't suggest something else.
            // get_action() returns NodeAction::Nothing for this case so it's unreachable
            (PackageJSONVersion::Unavailable, _) => unreachable!(),
            (_, Some(NVMRCVersion::Found(result))) => {
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
                let result = Self::get_default_node(flox)?;
                let message = format!("Flox detected an .nvmrc with a version specifier not understood by Flox, but Flox can provide {}",
                       result.version.as_ref().map(|version|format!("version {version}")).unwrap_or("another version".to_string()));
                (message, result.version)
            },
            (_, Some(NVMRCVersion::Unavailable)) => {
                let result = Self::get_default_node(flox)?;
                let message = format!("Flox detected an .nvmrc with a version of nodejs not provided by Flox, but Flox can provide {}",
                result.version.as_ref().map(|version|format!("version {version}")).unwrap_or("another version".to_string()));
                (message, result.version.clone())
            },
            (PackageJSONVersion::Unspecified, None) => {
                let result = Self::get_default_node(flox)?;
                mentions_package_json = true;
                ("Flox detected a package.json".to_string(), result.version)
            },
            // get_action() returns NodeAction::Nothing for this case so it's unreachable
            (PackageJSONVersion::None, None) => unreachable!(),
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

    fn prompt_for_hook(&mut self, flox: &Flox) -> Result<bool> {
        let (nodejs_detected, nodejs_version, _) = self.nodejs_message_and_version(flox)?;
        let message = formatdoc! {"
            {nodejs_detected}
            Flox detected both a package-lock.json and a yarn.lock

            Flox can add the following to your environment:
            * nodejs{}
            * Either an npm or yarn installation hook

            Would you like Flox to apply one of these modifications?
            You can always change the environment's manifest with 'flox edit'", nodejs_version.map(|version| format!(" {version}")).unwrap_or("".to_string())};
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

#[cfg(test)]
mod tests {
    use flox_rust_sdk::models::search::Subtree;
    use pretty_assertions::assert_eq;

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
    fn test_node_get_init_customization_install_action() {
        assert_eq!(
            Node {
                package_json_version: PackageJSONVersion::Found(Box::new(SearchResult {
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
                package_json_version: PackageJSONVersion::Unspecified,
                nvmrc_version: Some(NVMRCVersion::Unsure),
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
