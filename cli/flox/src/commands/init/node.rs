use std::fs;
use std::path::Path;

use anyhow::{anyhow, Result};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::path_environment::InitCustomization;
use flox_rust_sdk::models::environment::{global_manifest_lockfile_path, global_manifest_path};
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
    /// Node version as specified in package.json if it exists
    /// [PackageJSONVersion::None] if we found a compatible npm or yarn,
    package_json_node_version: PackageJSONVersion,
    /// Node version as specified in .nvmrc if it exists
    /// [None] if we found a version in package.json
    nvmrc_version: Option<NVMRCVersion>,
    action: NodeAction,
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
    /// We didn't check for package.json,
    /// or it does not exist or is invalid.
    None,
    /// package.json exists but doesn't specify a version
    Unspecified,
    Unavailable,
    Found(Box<SearchResult>),
}

#[derive(Clone, Debug, PartialEq)]
struct YarnInstall {
    yarn: SearchResult,
    node: SearchResult,
}

#[derive(Clone)]
struct NodeInstall {
    node: Option<SearchResult>,
    npm_hook: bool,
}

#[derive(Clone)]
enum NodeAction {
    InstallYarn(Box<YarnInstall>),
    InstallYarnOrNode(Box<YarnInstall>, Box<NodeInstall>),
    InstallNode(Box<NodeInstall>),
    Nothing,
}

struct PackageJSONVersions {
    yarn: Option<String>,
    node: Option<String>,
}

impl Node {
    pub fn new(path: &Path, flox: &Flox) -> Result<Self> {
        // Check for a viable yarn
        // TODO: we should check for npm version as well,
        // but since npm comes bundled with nodejs, we'll probably cover the most
        // cases by just giving the requested node and hoping for the bundled
        // npm to work.
        // We could check if our one version of npm with its harcoded nodejs
        // satisfies all constraints,
        // but that seems unlikely to be as commonly needed.
        let versions = Self::get_package_json_versions(path)?;
        let yarn_install = match versions {
            None => None,
            Some(ref versions) => {
                let yarn_lock_exists = path.join("yarn.lock").exists();
                if yarn_lock_exists {
                    Self::try_find_compatible_yarn(versions, flox)?
                } else {
                    None
                }
            },
        };

        let valid_package_json = versions.is_some();
        let package_json_and_package_lock =
            valid_package_json && path.join("package-lock.json").exists();

        // If there's not both a package.json and a package-lock.json, return
        // early with just yarn
        if let Some(yarn_install) = &yarn_install {
            if !package_json_and_package_lock {
                return Ok(Self {
                    action: NodeAction::InstallYarn(Box::new(yarn_install.clone())),
                    package_json_node_version: PackageJSONVersion::None,
                    nvmrc_version: None,
                });
            }
        }

        // Get value for self.package_json_node_version
        let package_json_node_version = match versions {
            Some(PackageJSONVersions {
                node: Some(ref node_version),
                ..
            }) => match Self::try_find_compatible_version("nodejs", node_version, None, flox)? {
                None => PackageJSONVersion::Unavailable,
                Some(result) => PackageJSONVersion::Found(Box::new(result)),
            },
            Some(_) => PackageJSONVersion::Unspecified,
            _ => PackageJSONVersion::None,
        };

        // Get value for self.nvmrc_version
        let nvmrc_version = match package_json_node_version {
            // package.json is higher priority than .nvmrc,
            // so don't check .nvmrc if we know we'll use the version in
            // package.json or we know we can't provide it
            PackageJSONVersion::Found(_) | PackageJSONVersion::Unavailable => None,
            _ => Self::get_nvmrc_version(path, flox)?,
        };

        let action = match yarn_install {
            Some(yarn_install) => {
                match Self::get_node_install(
                    &package_json_node_version,
                    &nvmrc_version,
                    valid_package_json,
                ) {
                    Some(node_install) => NodeAction::InstallYarnOrNode(
                        Box::new(yarn_install),
                        Box::new(node_install),
                    ),
                    None => NodeAction::InstallYarn(Box::new(yarn_install)),
                }
            },
            None => {
                match Self::get_node_install(
                    &package_json_node_version,
                    &nvmrc_version,
                    valid_package_json,
                ) {
                    Some(node_install) => NodeAction::InstallNode(Box::new(node_install)),
                    None => NodeAction::Nothing,
                }
            },
        };

        Ok(Self {
            package_json_node_version,
            nvmrc_version,
            action,
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
                let yarn = package_json_json["engines"]["yarn"]
                    .as_str()
                    .map(|s| s.to_string());
                Ok(Some(PackageJSONVersions { node, yarn }))
            },
        }
    }

    /// Try to find node, npm, and yarn versions that satisfy constraints in
    /// package.json
    fn try_find_compatible_yarn(
        versions: &PackageJSONVersions,
        flox: &Flox,
    ) -> Result<Option<YarnInstall>> {
        let PackageJSONVersions { yarn, node, .. } = versions;

        let found_node = match node {
            Some(node_version) => {
                match Self::get_default_node_if_compatible(Some(node_version.clone()), flox)? {
                    // If the corresponding node isn't compatible, don't install yarn
                    None => return Ok(None),
                    Some(found_node) => found_node,
                }
            },
            None => Self::get_default_node_if_compatible(None, flox)?
                .ok_or(anyhow!("Flox couldn't find nodejs in nixpkgs"))?,
        };

        // We assume that yarn is built with found_node, which is currently true
        // in nixpkgs
        let found_yarn = match yarn {
            Some(yarn_version) => {
                Self::try_find_compatible_version("yarn", yarn_version, None, flox)?
            },
            _ => Some(Self::get_default_package("yarn", flox)?),
        };

        Ok(found_yarn.map(|found_yarn| YarnInstall {
            yarn: found_yarn,
            node: found_node,
        }))
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
            lockfile: PathOrJson::Path(global_manifest_lockfile_path(flox)),
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
            lockfile: PathOrJson::Path(global_manifest_lockfile_path(flox)),
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
            lockfile: PathOrJson::Path(global_manifest_lockfile_path(flox)),
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
    fn get_node_install(
        package_json_node_version: &PackageJSONVersion,
        nvmrc_version: &Option<NVMRCVersion>,
        npm_hook: bool,
    ) -> Option<NodeInstall> {
        match (package_json_node_version, nvmrc_version) {
            // package.json takes precedence over .nvmrc
            (PackageJSONVersion::Found(result), _) => Some(NodeInstall {
                node: Some(*result.clone()),
                npm_hook,
            }),
            // Treat the version in package.json strictly; if we can't find it, don't suggest something else.
            (PackageJSONVersion::Unavailable, _) => None,
            (_, Some(NVMRCVersion::Found(result))) => Some(NodeInstall {
                node: Some(*result.clone()),
                npm_hook,
            }),
            (_, Some(NVMRCVersion::Unsure)) => Some(NodeInstall {
                node: None,
                npm_hook,
            }),
            (_, Some(NVMRCVersion::Unavailable)) => Some(NodeInstall {
                node: None,
                npm_hook,
            }),
            (PackageJSONVersion::Unspecified, None) => Some(NodeInstall {
                node: None,
                npm_hook,
            }),
            (PackageJSONVersion::None, None) => None,
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
                mentions_package_json = true;
                ("Flox detected a package.json".to_string(), result.version)
            },
            // get_action() returns NodeAction::Nothing for this case so it's unreachable
            (PackageJSONVersion::None, None) => unreachable!(),
        };
        Ok((message, version, mentions_package_json))
    }

    /// Prompt whether to install nodejs (but not npm or yarn)
    fn prompt_for_node(&self, node_install: &NodeInstall, flox: &Flox) -> Result<bool> {
        let (nodejs_detected, nodejs_version, mentions_package_json) =
            self.nodejs_message_and_version(flox)?;
        let mut message = format!("{nodejs_detected}\n");
        if !mentions_package_json {
            message.push_str("Flox detected a package.json\n");
        }

        if node_install.npm_hook {
            message.push_str(&formatdoc! {"

                Flox can add the following to your environment:
                * nodejs{} with npm bundled
                * An npm installation hook
            ",
                nodejs_version
                    .map(|version| format!(" {version}"))
                    .unwrap_or("".to_string()),
            });
        } else {
            message.push_str(&formatdoc! {"

                Flox can add the following to your environment:
                * nodejs{}
            ",
                nodejs_version
                    .map(|version| format!(" {version}"))
                    .unwrap_or("".to_string()),
            });
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

    /// Prompt whether to install npm or yarn when only one of them is viable
    fn prompt_with_yarn(&self, yarn_install: &YarnInstall) -> Result<bool> {
        let message = {
            let yarn_version = yarn_install
                .yarn
                .version
                .as_ref()
                .map(|version| format!(" {version}"))
                .unwrap_or("".to_string());
            let node_version = yarn_install
                .node
                .version
                .as_ref()
                .map(|version| format!(" {version}"))
                .unwrap_or("".to_string());

            formatdoc! {"
                    Flox detected a package.json and a yarn.lock

                    Flox can add the following to your environment:
                    * yarn{yarn_version} with nodejs{node_version} bundled
                    * A yarn installation hook

                    Would you like Flox to apply this suggestion?
                    You can always change the environment's manifest with 'flox edit'
                "}
        };

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
    fn prompt_for_package_manager(
        &mut self,
        yarn_install: YarnInstall,
        node_install: NodeInstall,
        flox: &Flox,
    ) -> Result<bool> {
        let yarn_version = yarn_install
            .yarn
            .version
            .as_ref()
            .map(|version| format!(" {version}"))
            .unwrap_or("".to_string());
        let yarn_node_version = yarn_install
            .node
            .version
            .as_ref()
            .map(|version| format!(" {version}"))
            .unwrap_or("".to_string());
        let node_version = match &node_install.node {
            Some(found_node) => found_node.clone(),
            None => Self::get_default_package("nodejs", flox)?,
        }
        .version
        .as_ref()
        .map(|version| format!(" {version}"))
        .unwrap_or("".to_string());

        let message = formatdoc! {"
            Flox detected both a package-lock.json and a yarn.lock

            Flox can add the following to your environment:
            * Either nodejs{node_version} with npm bundled, or yarn{yarn_version} with nodejs{yarn_node_version} bundled
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
                self.action = NodeAction::InstallNode(Box::new(node_install.clone()))
            } else if choice == 4 {
                self.action = NodeAction::InstallYarn(Box::new(yarn_install.clone()))
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
            self.action = NodeAction::InstallNode(Box::new(node_install.clone()))
        } else if choice == 1 {
            self.action = NodeAction::InstallYarn(Box::new(yarn_install.clone()))
        }
        Ok(choice == 0 || choice == 1)
    }
}

impl InitHook for Node {
    fn should_run(&mut self, _path: &Path) -> Result<bool> {
        match self.action {
            NodeAction::InstallYarn(_) => {
                debug!("Should run node init hook and install yarn.");
                Ok(true)
            },
            NodeAction::InstallYarnOrNode(_, _) => {
                debug!("Should run node init hook and install npm or yarn.");
                Ok(true)
            },
            NodeAction::InstallNode(_) => {
                debug!("Should run node init hook and install nodejs");
                Ok(true)
            },
            NodeAction::Nothing => {
                debug!("Should not run node init hook");
                Ok(false)
            },
        }
    }

    fn prompt_user(&mut self, _path: &Path, flox: &Flox) -> Result<bool> {
        match &self.action {
            NodeAction::InstallYarn(yarn_install) => self.prompt_with_yarn(yarn_install),
            NodeAction::InstallYarnOrNode(yarn_install, node_install) => {
                self.prompt_for_package_manager(*yarn_install.clone(), *node_install.clone(), flox)
            },
            NodeAction::InstallNode(node_install) => self.prompt_for_node(node_install, flox),
            NodeAction::Nothing => unreachable!(),
        }
    }

    fn get_init_customization(&self) -> InitCustomization {
        let mut packages = vec![];

        let hook = match &self.action {
            NodeAction::InstallYarn(yarn_install) => {
                packages.push(PackageToInstall {
                    id: "yarn".to_string(),
                    pkg_path: yarn_install.yarn.rel_path.join("."),
                    // TODO: we probably shouldn't pin this when we're just
                    // providing the default
                    version: yarn_install.yarn.version.clone(),
                    input: None,
                });
                Some(YARN_HOOK.to_string())
            },
            // Default to node for InstallYarnOrNode
            // This is only reachable if --auto-setup is used.
            NodeAction::InstallYarnOrNode(_, node_install)
            | NodeAction::InstallNode(node_install) => {
                let nodejs_to_install = match &node_install.node {
                    Some(result) => PackageToInstall {
                        id: "nodejs".to_string(),
                        pkg_path: result.rel_path.join("."),
                        version: result.version.clone(),
                        input: None,
                    },
                    None => PackageToInstall {
                        id: "nodejs".to_string(),
                        pkg_path: "nodejs".to_string(),
                        version: None,
                        input: None,
                    },
                };
                packages.push(nodejs_to_install);
                if node_install.npm_hook {
                    Some(NPM_HOOK.to_string())
                } else {
                    None
                }
            },
            // get_init_customization() should only be called when should_run() returns true
            NodeAction::Nothing => unreachable!(),
        };

        InitCustomization {
            hook,
            packages: Some(packages),
        }
    }
}

#[cfg(test)]
mod tests {
    use flox_rust_sdk::flox::test_flox_instance;
    use flox_rust_sdk::models::lockfile::LockedManifest;
    use once_cell::sync::Lazy;
    use pretty_assertions::assert_eq;
    use serial_test::serial;
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

    /// Test get_init_customization() for action InstallYarn
    #[test]
    fn test_get_init_customization_yarn() {
        assert_eq!(
            Node {
                package_json_node_version: PackageJSONVersion::None,
                nvmrc_version: None,
                action: NodeAction::InstallYarn(Box::new(YarnInstall {
                    yarn: SearchResult {
                        rel_path: vec!["yarn".to_string(), "path".to_string()],
                        version: Some("1".to_string()),
                        ..Default::default()
                    },
                    node: SearchResult::default(),
                })),
            }
            .get_init_customization(),
            InitCustomization {
                packages: Some(vec![PackageToInstall {
                    id: "yarn".to_string(),
                    pkg_path: "yarn.path".to_string(),
                    version: Some("1".to_string()),
                    input: None,
                }]),
                hook: Some(YARN_HOOK.to_string()),
            }
        );
    }

    /// Test get_init_customization() for action InstallYarnOrNode and npm_hook
    /// true
    #[test]
    fn test_get_init_customization_yarn_or_node() {
        assert_eq!(
            Node {
                package_json_node_version: PackageJSONVersion::None,
                nvmrc_version: None,
                action: NodeAction::InstallYarnOrNode(
                    Box::new(YarnInstall {
                        yarn: SearchResult::default(),
                        node: SearchResult::default(),
                    }),
                    Box::new(NodeInstall {
                        node: Some(SearchResult {
                            rel_path: vec!["nodejs".to_string(), "path".to_string()],
                            version: Some("1".to_string()),
                            ..Default::default()
                        }),
                        npm_hook: true,
                    })
                ),
            }
            .get_init_customization(),
            InitCustomization {
                packages: Some(vec![PackageToInstall {
                    id: "nodejs".to_string(),
                    pkg_path: "nodejs.path".to_string(),
                    version: Some("1".to_string()),
                    input: None,
                }]),
                hook: Some(NPM_HOOK.to_string()),
            }
        );
    }
    /// Test get_init_customization() for action InstallNode and npm_hook false
    #[test]
    fn test_get_init_customization_node() {
        assert_eq!(
            Node {
                package_json_node_version: PackageJSONVersion::None,
                nvmrc_version: None,
                action: NodeAction::InstallNode(Box::new(NodeInstall {
                    node: Some(SearchResult {
                        rel_path: vec!["nodejs".to_string(), "path".to_string()],
                        version: Some("1".to_string()),
                        ..Default::default()
                    }),
                    npm_hook: false,
                })),
            }
            .get_init_customization(),
            InitCustomization {
                packages: Some(vec![PackageToInstall {
                    id: "nodejs".to_string(),
                    pkg_path: "nodejs.path".to_string(),
                    version: Some("1".to_string()),
                    input: None,
                }]),
                hook: None,
            }
        );
    }

    // TODO: all the try_find_compatible_yarn() tests actually hit the database,
    // and it might be better to mock out do_search().
    // But I'm only seeing 6 tests take ~3 seconds,
    // so at this point I think there are bigger testing efficiency fish to fry.

    static FLOX_INSTANCE: Lazy<(Flox, TempDir)> = Lazy::new(|| {
        let (flox, _temp_dir_handle) = test_flox_instance();
        let pkgdb_nixpkgs_rev_new = "ab5fd150146dcfe41fda501134e6503932cc8dfd";
        std::env::set_var("_PKGDB_GA_REGISTRY_REF_OR_REV", pkgdb_nixpkgs_rev_new);
        LockedManifest::update_global_manifest(&flox, vec![]).unwrap();
        (flox, _temp_dir_handle)
    });

    /// Test finding yarn with no constraints succeeds
    #[test]
    #[serial]
    fn test_try_find_compatible_yarn_no_constraints() {
        let flox = &FLOX_INSTANCE.0;
        let yarn_install = Node::try_find_compatible_yarn(
            &PackageJSONVersions {
                yarn: None,
                node: None,
            },
            flox,
        )
        .unwrap()
        .unwrap();

        assert_eq!(yarn_install.node.rel_path, vec!["nodejs".to_string()]);
        assert_eq!(yarn_install.yarn.rel_path, vec!["yarn".to_string()]);
    }

    /// Test finding yarn with the version of nixpkgs#nodejs specified succeeds
    #[test]
    #[serial]
    fn test_try_find_compatible_yarn_node_available() {
        let flox = &FLOX_INSTANCE.0;
        let yarn_install = Node::try_find_compatible_yarn(
            &PackageJSONVersions {
                yarn: None,
                node: Some("18".to_string()),
            },
            flox,
        )
        .unwrap()
        .unwrap();

        assert_eq!(yarn_install.node.rel_path, vec!["nodejs".to_string()]);
        assert!(yarn_install.node.version.unwrap().starts_with("18"));
        assert_eq!(yarn_install.yarn.rel_path, vec!["yarn".to_string()]);
    }

    /// Test finding yarn with a version of node other than that of
    /// nixpkgs#nodejs fails
    #[test]
    #[serial]
    fn test_try_find_compatible_yarn_node_unavailable() {
        let flox = &FLOX_INSTANCE.0;
        let yarn_install = Node::try_find_compatible_yarn(
            &PackageJSONVersions {
                yarn: None,
                node: Some("20".to_string()),
            },
            flox,
        )
        .unwrap();

        assert_eq!(yarn_install, None);
    }

    /// Test finding yarn with the version nixpkgs#yarn specified succeeds
    #[test]
    #[serial]
    fn test_try_find_compatible_yarn_yarn_available() {
        let flox = &FLOX_INSTANCE.0;
        let yarn_install = Node::try_find_compatible_yarn(
            &PackageJSONVersions {
                yarn: Some("1".to_string()),
                node: None,
            },
            flox,
        )
        .unwrap()
        .unwrap();

        assert_eq!(yarn_install.node.rel_path, vec!["nodejs".to_string()]);
        assert_eq!(yarn_install.yarn.rel_path, vec!["yarn".to_string()]);
        assert!(yarn_install.yarn.version.unwrap().starts_with('1'));
    }

    /// Test finding yarn with a version of yarn other than that of
    /// nixpkgs#yarn fails
    #[test]
    #[serial]
    fn test_try_find_compatible_yarn_yarn_unavailable() {
        let flox = &FLOX_INSTANCE.0;
        let yarn_install = Node::try_find_compatible_yarn(
            &PackageJSONVersions {
                yarn: Some("2".to_string()),
                node: None,
            },
            flox,
        )
        .unwrap();

        assert_eq!(yarn_install, None);
    }

    /// Test finding yarn with versions of nixpkgs#yarn and nixpkgs#nodejs
    /// specified succeeds
    #[test]
    #[serial]
    fn test_try_find_compatible_yarn_both_available() {
        let flox = &FLOX_INSTANCE.0;
        let yarn_install = Node::try_find_compatible_yarn(
            &PackageJSONVersions {
                yarn: Some("1".to_string()),
                node: Some("18".to_string()),
            },
            flox,
        )
        .unwrap()
        .unwrap();

        assert_eq!(yarn_install.node.rel_path, vec!["nodejs".to_string()]);
        assert!(yarn_install.node.version.unwrap().starts_with("18"));
        assert_eq!(yarn_install.yarn.rel_path, vec!["yarn".to_string()]);
        assert!(yarn_install.yarn.version.unwrap().starts_with('1'));
    }
}
