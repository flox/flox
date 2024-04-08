use std::collections::HashSet;
use std::num::NonZeroU8;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use anyhow::{anyhow, Context, Error, Result};
use bpaf::Bpaf;
use flox_rust_sdk::data::{AttrPath, CanonicalPath};
use flox_rust_sdk::flox::{EnvironmentName, Flox, DEFAULT_NAME};
use flox_rust_sdk::models::environment::path_environment::{InitCustomization, PathEnvironment};
use flox_rust_sdk::models::environment::{
    global_manifest_lockfile_path,
    global_manifest_path,
    Environment,
    PathPointer,
};
use flox_rust_sdk::models::lockfile::{
    LockedManifest,
    LockedManifestPkgdb,
    TypedLockedManifestPkgdb,
};
use flox_rust_sdk::models::manifest::{insert_packages, PackageToInstall};
use flox_rust_sdk::models::pkgdb::scrape_input;
use flox_rust_sdk::models::search::{do_search, PathOrJson, Query, SearchParams, SearchResult};
use flox_rust_sdk::providers::catalog::{
    ClientTrait,
    PackageDescriptor,
    PackageGroup,
    PackageResolutionInfo,
};
use indoc::formatdoc;
use log::debug;
use path_dedot::ParseDot;
use toml_edit::{DocumentMut, Formatted, Item, Table, Value};
use tracing::instrument;

use crate::commands::{environment_description, ConcreteEnvironment};
use crate::config::features::Features;
use crate::subcommand_metric;
use crate::utils::dialog::{Dialog, Spinner};
use crate::utils::message;

mod go;
mod node;
mod python;

use go::Go;
use node::Node;
use python::Python;

const AUTO_SETUP_HINT: &str = "Use '--auto-setup' to apply Flox recommendations in the future.";

/// The different types of init customizations
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
enum InitHookType {
    Go(Go),
    Node(Node),
    Python(Python),
}

impl InitHook for InitHookType {
    async fn prompt_user(&mut self, flox: &Flox, path: &Path) -> Result<bool> {
        match self {
            InitHookType::Go(hook) => hook.prompt_user(flox, path).await,
            InitHookType::Node(hook) => hook.prompt_user(flox, path).await,
            InitHookType::Python(hook) => hook.prompt_user(flox, path).await,
        }
    }

    fn get_init_customization(&self) -> InitCustomization {
        match self {
            InitHookType::Go(hook) => hook.get_init_customization(),
            InitHookType::Node(hook) => hook.get_init_customization(),
            InitHookType::Python(hook) => hook.get_init_customization(),
        }
    }
}

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
    #[instrument(name = "init", skip_all)]
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
                .parse_dot()?
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .context("Can't init in root")?;
            EnvironmentName::from_str(&name)?
        };

        // Don't run language hooks in home dir
        let customization = if dir != home_dir || self.auto_setup {
            // Some language hooks run searches, so scrape with pkgdb if necessary
            if flox.catalog_client.is_none() {
                tracing::debug!("using pkgdb for init");
                Dialog {
                    message: "Generating database for flox packages...",
                    help_message: None,
                    typed: Spinner::new(|| {
                        let global_lockfile = LockedManifestPkgdb::ensure_global_lockfile(&flox)?;

                        let lockfile: LockedManifest =
                            LockedManifest::read_from_file(&CanonicalPath::new(global_lockfile)?)?;

                        let LockedManifest::Pkgdb(lockfile) = lockfile else {
                            return Err(anyhow!("Expected a Pkgdb lockfile"));
                        };

                        let lockfile = TypedLockedManifestPkgdb::try_from(lockfile)?;

                        // --ga-registry forces a single input
                        if let Some((_, input)) = lockfile.registry().inputs.iter().next() {
                            scrape_input(&input.from)?;
                        };
                        Ok::<(), Error>(())
                    }),
                }
                .spin_with_delay(Duration::from_secs_f32(0.25))?;
            };

            self.run_language_hooks(&flox, &dir)
                .await
                .unwrap_or_else(|e| {
                    message::warning(format!("Failed to generate init suggestions: {}", e));
                    InitCustomization::default()
                })
        } else {
            debug!("Skipping language hooks in home directory");
            InitCustomization::default()
        };

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
            "Created environment '{name}' ({system})",
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

    /// Run all language hooks and return a single combined customization
    async fn run_language_hooks(&self, flox: &Flox, path: &Path) -> Result<InitCustomization> {
        let mut hooks: Vec<InitHookType> = vec![];

        if let Some(node) = Node::new(flox, path).await? {
            hooks.push(InitHookType::Node(node));
        }

        if let Some(python) = Python::new(flox, path).await {
            hooks.push(InitHookType::Python(python));
        }

        if let Some(go) = Go::new(flox, path).await? {
            hooks.push(InitHookType::Go(go));
        }

        let mut customizations = vec![];

        for mut hook in hooks {
            // Run hooks if we can't prompt
            if self.auto_setup || (Dialog::can_prompt() && hook.prompt_user(flox, path).await?) {
                customizations.push(hook.get_init_customization())
            }
        }

        Ok(Self::combine_customizations(customizations))
    }

    /// Deduplicate packages and concatenate customization scripts into a single string
    fn combine_customizations(customizations: Vec<InitCustomization>) -> InitCustomization {
        let mut custom_hook_on_activate_scripts: Vec<String> = vec![];
        let mut custom_profile_common_scripts: Vec<String> = vec![];
        let mut custom_profile_bash_scripts: Vec<String> = vec![];
        let mut custom_profile_fish_scripts: Vec<String> = vec![];
        let mut custom_profile_tcsh_scripts: Vec<String> = vec![];
        let mut custom_profile_zsh_scripts: Vec<String> = vec![];
        // Deduplicate packages with a set
        let mut packages_set = HashSet::<PackageToInstall>::new();
        for customization in customizations {
            if let Some(packages) = customization.packages {
                packages_set.extend(packages)
            }
            if let Some(hook_on_activate_script) = customization.hook_on_activate {
                custom_hook_on_activate_scripts.push(hook_on_activate_script)
            }
            if let Some(profile_common_script) = customization.profile_common {
                custom_profile_common_scripts.push(profile_common_script)
            }
            if let Some(profile_bash_script) = customization.profile_bash {
                custom_profile_bash_scripts.push(profile_bash_script)
            }
            if let Some(profile_fish_script) = customization.profile_fish {
                custom_profile_fish_scripts.push(profile_fish_script)
            }
            if let Some(profile_tcsh_script) = customization.profile_tcsh {
                custom_profile_tcsh_scripts.push(profile_tcsh_script)
            }
            if let Some(profile_zsh_script) = customization.profile_zsh {
                custom_profile_zsh_scripts.push(profile_zsh_script)
            }
        }

        let custom_hook_on_activate = (!custom_hook_on_activate_scripts.is_empty()).then(|| {
            formatdoc! {"
                # Autogenerated by Flox

                {}

                # End autogenerated by Flox
                ", custom_hook_on_activate_scripts.join("\n\n")}
        });

        let custom_profile_common = (!custom_profile_common_scripts.is_empty()).then(|| {
            formatdoc! {"
                # Autogenerated by Flox

                {}

                # End autogenerated by Flox
                ", custom_profile_common_scripts.join("\n\n")}
        });
        let custom_profile_bash = (!custom_profile_bash_scripts.is_empty()).then(|| {
            formatdoc! {"
                # Autogenerated by Flox

                {}

                # End autogenerated by Flox
                ", custom_profile_bash_scripts.join("\n\n")}
        });
        let custom_profile_fish = (!custom_profile_fish_scripts.is_empty()).then(|| {
            formatdoc! {"
                # Autogenerated by Flox

                {}

                # End autogenerated by Flox", custom_profile_fish_scripts.join("\n\n")}
        });
        let custom_profile_tcsh = (!custom_profile_tcsh_scripts.is_empty()).then(|| {
            formatdoc! {"
                # Autogenerated by Flox

                {}

                # End autogenerated by Flox", custom_profile_tcsh_scripts.join("\n\n")}
        });
        let custom_profile_zsh = (!custom_profile_zsh_scripts.is_empty()).then(|| {
            formatdoc! {"
                # Autogenerated by Flox

                {}

                # End autogenerated by Flox
                ", custom_profile_zsh_scripts.join("\n\n")}
        });

        let packages = (!packages_set.is_empty())
            .then(|| packages_set.into_iter().collect::<Vec<PackageToInstall>>());

        InitCustomization {
            hook_on_activate: custom_hook_on_activate,
            profile_common: custom_profile_common,
            profile_bash: custom_profile_bash,
            profile_fish: custom_profile_fish,
            profile_tcsh: custom_profile_tcsh,
            profile_zsh: custom_profile_zsh,
            packages,
        }
    }
}

// TODO: clean up how we pass around path and flox
trait InitHook {
    async fn prompt_user(&mut self, flox: &Flox, path: &Path) -> Result<bool>;

    fn get_init_customization(&self) -> InitCustomization;
}

/// Create a temporary TOML document containing just the contents of the passed
/// [InitCustomization], and return it as a string.
fn format_customization(customization: &InitCustomization) -> Result<String> {
    let mut toml = if let Some(packages) = &customization.packages {
        let with_packages = insert_packages("", packages)?;
        with_packages.new_toml.unwrap_or(DocumentMut::new())
    } else {
        DocumentMut::new()
    };

    // Add the "hook" section to the toml document.
    let hook_table = {
        let hook_field = toml
            .entry("hook")
            .or_insert_with(|| Item::Table(Table::new()));
        let hook_field_type = hook_field.type_name();
        hook_field.as_table_mut().context(format!(
            "'hook' must be a table, but found {hook_field_type} instead"
        ))?
    };
    if let Some(hook_on_activate_script) = &customization.hook_on_activate {
        hook_table.insert(
            "on-activate",
            Item::Value(Value::String(Formatted::new(formatdoc! {r#"
                {}
            "#, indent::indent_all_by(2, hook_on_activate_script)}))),
        );
    };

    // Add the "profile" section to the toml document.
    let profile_table = {
        let profile_field = toml
            .entry("profile")
            .or_insert_with(|| Item::Table(Table::new()));
        let profile_field_type = profile_field.type_name();
        profile_field.as_table_mut().context(format!(
            "'profile' must be a table, but found {profile_field_type} instead"
        ))?
    };
    if let Some(profile_common_script) = &customization.profile_common {
        profile_table.insert(
            "common",
            Item::Value(Value::String(Formatted::new(formatdoc! {r#"
                {}
            "#, indent::indent_all_by(2, profile_common_script)}))),
        );
    };
    if let Some(profile_bash_script) = &customization.profile_bash {
        profile_table.insert(
            "bash",
            Item::Value(Value::String(Formatted::new(formatdoc! {r#"
                {}
            "#, indent::indent_all_by(2, profile_bash_script)}))),
        );
    };
    if let Some(profile_fish_script) = &customization.profile_fish {
        profile_table.insert(
            "bash",
            Item::Value(Value::String(Formatted::new(formatdoc! {r#"
                {}
            "#, indent::indent_all_by(2, profile_fish_script)}))),
        );
    };
    if let Some(profile_tcsh_script) = &customization.profile_tcsh {
        profile_table.insert(
            "bash",
            Item::Value(Value::String(Formatted::new(formatdoc! {r#"
                {}
            "#, indent::indent_all_by(2, profile_tcsh_script)}))),
        );
    };
    if let Some(profile_zsh_script) = &customization.profile_zsh {
        profile_table.insert(
            "zsh",
            Item::Value(Value::String(Formatted::new(formatdoc! {r#"
                {}
            "#, indent::indent_all_by(2, profile_zsh_script)}))),
        );
    };

    Ok(toml.to_string())
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
#[derive(Debug, PartialEq, Clone)]
pub(crate) enum ProvidedVersion {
    Compatible {
        requested: Option<String>,
        compatible: ProvidedPackage,
    },
    Incompatible {
        requested: String,
        substitute: ProvidedPackage,
    },
}

impl ProvidedVersion {
    pub(crate) fn display_version(&self) -> &str {
        match self {
            Self::Compatible { compatible, .. } => &compatible.display_version,
            Self::Incompatible { substitute, .. } => &substitute.display_version,
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct ProvidedPackage {
    /// Name of the provided package
    /// pname or the last component of [Self::rel_path]
    pub name: String,
    /// Path to the package in the catalog
    /// Checked to be non-empty
    pub rel_path: AttrPath,
    /// Version of the package in the catalog
    /// "N/A" if not found
    ///
    /// Used for display purposes only,
    /// version constraints should be added based on the original query.
    pub display_version: String,
    /// The actual version of the package
    pub version: Option<String>,
}

impl TryFrom<SearchResult> for ProvidedPackage {
    type Error = Error;

    fn try_from(value: SearchResult) -> Result<Self, Self::Error> {
        let path_name = value
            .rel_path
            .last()
            .ok_or_else(|| anyhow!("invalid search result: 'rel_path' empty in {value:?}"))?;

        let name = value.pname.unwrap_or_else(|| path_name.to_string());

        Ok(ProvidedPackage {
            name,
            rel_path: value.rel_path.into(),
            display_version: value.version.clone().unwrap_or("N/A".to_string()),
            version: value.version,
        })
    }
}

impl From<ProvidedPackage> for PackageToInstall {
    fn from(value: ProvidedPackage) -> Self {
        PackageToInstall {
            id: value.name,
            pkg_path: value.rel_path.into(),
            input: None,
            version: value.version,
        }
    }
}

impl From<PackageResolutionInfo> for ProvidedPackage {
    fn from(value: PackageResolutionInfo) -> Self {
        Self {
            name: value.install_id,
            rel_path: value.attr_path.into(),
            display_version: value.version.clone(),
            version: Some(value.version),
        }
    }
}

impl From<&PackageResolutionInfo> for ProvidedPackage {
    fn from(value: &PackageResolutionInfo) -> Self {
        Self {
            name: value.install_id.clone(),
            rel_path: value.attr_path.clone().into(),
            display_version: value.version.clone(),
            version: Some(value.version.clone()),
        }
    }
}

/// Get nixpkgs#rel_path,
/// optionally verifying that it satisfies a version constraint.
async fn get_default_package_if_compatible(
    flox: &Flox,
    rel_path: Vec<String>,
    version: Option<String>,
) -> Result<Option<ProvidedPackage>> {
    let pkg = if let Some(client) = flox.catalog_client.as_ref() {
        tracing::debug!("using catalog client to find default compatible package");
        let resolved_groups = client
            .resolve(vec![PackageGroup {
                descriptors: vec![PackageDescriptor {
                    attr_path: rel_path.join("."),
                    install_id: "default".to_string(),
                    version,
                    allow_pre_releases: None,
                    derivation: None,
                }],
                name: "default".to_string(),
                system: flox.system.clone(),
            }])
            .await?;
        let pkg: Option<ProvidedPackage> = resolved_groups
            .first()
            .and_then(|pkg_group| pkg_group.pages.first())
            .and_then(|page| page.packages.as_ref())
            .and_then(|pkgs| pkgs.first().cloned())
            .map(|pkg| {
                // Type-inference fails without the fully-qualified method call
                <PackageResolutionInfo as Into<ProvidedPackage>>::into(pkg)
            });
        let Some(pkg) = pkg else {
            tracing::debug!("no compatible default package");
            return Ok(None);
        };
        pkg
    } else {
        tracing::debug!("using pkgdb to find default compatible package");
        let query = Query {
            rel_path: Some(rel_path),
            semver: version,
            limit: NonZeroU8::new(1),
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
            tracing::debug!("no compatible default package version found");
            return Ok(None);
        }
        let pkg = results.results.swap_remove(0);
        pkg.try_into()?
    };
    tracing::debug!(
        version = pkg.version.as_ref().unwrap_or(&"null".to_string()),
        "found default package version"
    );
    Ok(Some(pkg))
}

/// Get a package as if installed with `flox install {package}`
async fn get_default_package(flox: &Flox, package: &AttrPath) -> Result<ProvidedPackage> {
    let pkg = if let Some(client) = flox.catalog_client.as_ref() {
        tracing::debug!(
            package = package.to_string(),
            "using catalog client to find default package"
        );
        let resolved_groups = client
            .resolve(vec![PackageGroup {
                descriptors: vec![PackageDescriptor {
                    attr_path: package.to_string(),
                    install_id: package.to_string(),
                    version: None,
                    allow_pre_releases: None,
                    derivation: None,
                }],
                name: package.to_string(),
                system: flox.system.clone(),
            }])
            .await?;
        let pkg: Option<ProvidedPackage> = resolved_groups
            .first()
            .and_then(|pkg_group| pkg_group.pages.first())
            .and_then(|page| page.packages.as_ref())
            .and_then(|pkgs| pkgs.first().cloned())
            .map(|pkg| {
                // Type-inference fails without the fully-qualified method call
                <PackageResolutionInfo as Into<ProvidedPackage>>::into(pkg)
            });
        let Some(pkg) = pkg else {
            tracing::debug!("no default package found");
            return Err(anyhow!("Flox couldn't find any versions of {package}"))?;
        };
        pkg
    } else {
        tracing::debug!(
            package = package.to_string(),
            "using pkgdb to find default package"
        );
        let query = Query::new(
            package.to_string().as_ref(),
            Features::parse()?.search_strategy,
            NonZeroU8::new(1),
            false,
        )?;
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
        results.results.swap_remove(0).try_into()?
    };

    tracing::debug!(
        version = pkg.version.as_ref().unwrap_or(&"null".to_string()),
        package = package.to_string(),
        "found default package"
    );
    Ok(pkg)
}

/// Searches for a given pname and version, optionally restricting rel_path
async fn try_find_compatible_version(
    flox: &Flox,
    pname: &str,
    version: &str,
    rel_path: Option<Vec<String>>,
) -> Result<Option<ProvidedPackage>> {
    let pkg = if let Some(client) = flox.catalog_client.as_ref() {
        tracing::debug!(
            pname,
            version,
            "using catalog client to find compatible package version"
        );
        let resolved_groups = client
            .resolve(vec![PackageGroup {
                descriptors: vec![PackageDescriptor {
                    attr_path: pname.to_string(),
                    install_id: pname.to_string(),
                    version: Some(version.to_string()),
                    allow_pre_releases: None,
                    derivation: None,
                }],
                name: pname.to_string(),
                system: flox.system.clone(),
            }])
            .await?;
        let pkg: Option<ProvidedPackage> = resolved_groups
            .first()
            .and_then(|pkg_group| pkg_group.pages.first())
            .and_then(|page| page.packages.as_ref())
            .and_then(|pkgs| pkgs.first().cloned())
            .map(|pkg| {
                // Type-inference fails without the fully-qualified method call
                <PackageResolutionInfo as Into<ProvidedPackage>>::into(pkg)
            });
        let Some(pkg) = pkg else {
            tracing::debug!(pname, version, "no compatible package version found");
            return Ok(None);
        };
        pkg
    } else {
        tracing::debug!("using pkgdb to find default node version");
        let query = Query {
            pname: Some(pname.to_string()),
            semver: Some(version.to_string()),
            limit: NonZeroU8::new(1),
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
        let matching_package = results.results.swap_remove(0);
        matching_package.try_into()?
    };

    tracing::debug!(
        pname,
        version = pkg.version.as_ref().unwrap_or(&"null".to_string()),
        "found matching package version"
    );
    Ok(Some(pkg))
}

#[cfg(test)]
mod tests {

    use flox_rust_sdk::data::System;
    use flox_rust_sdk::providers::catalog::{CatalogPage, ResolvedPackageGroup};
    use indoc::indoc;
    use pretty_assertions::assert_eq;

    use super::*;

    impl ProvidedPackage {
        pub(crate) fn new(
            name: impl ToString,
            rel_path: impl IntoIterator<Item = impl ToString>,
            version: &str,
        ) -> Self {
            Self {
                name: name.to_string(),
                rel_path: rel_path
                    .into_iter()
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
                    .into(),
                display_version: version.to_string(),
                version: if version != "N/A" {
                    Some(version.to_string())
                } else {
                    None
                },
            }
        }
    }

    // This function should really be a #[cfg(test)] method on ResolvedPackageGroup,
    // but you can't import test features across crates
    pub fn resolved_pkg_group_with_dummy_package(
        group_name: &str,
        system: &System,
        install_id: &str,
        pkg_path: &str,
        version: &str,
    ) -> ResolvedPackageGroup {
        let pkg = PackageResolutionInfo {
            attr_path: pkg_path.to_string(),
            broken: false,
            derivation: String::new(),
            description: None,
            install_id: install_id.to_string(),
            license: None,
            locked_url: String::new(),
            name: String::new(),
            outputs: None,
            outputs_to_install: None,
            pname: String::new(),
            rev: String::new(),
            rev_count: 0,
            rev_date: chrono::offset::Utc::now(),
            scrape_date: chrono::offset::Utc::now(),
            stabilities: None,
            unfree: None,
            version: version.to_string(),
        };
        let page = CatalogPage {
            packages: Some(vec![pkg]),
            page: 0,
            url: String::new(),
        };
        ResolvedPackageGroup {
            name: group_name.to_string(),
            pages: vec![page],
            system: system.to_string(),
        }
    }

    // This function should really be a #[cfg(test)] method on ResolvedPackageGroup,
    // but you can't import test features across crates
    #[allow(dead_code)]
    pub fn push_dummy_package_to_first_page_of_pkg_group(
        resolved_group: &mut ResolvedPackageGroup,
        install_id: &str,
        pkg_path: &str,
        version: &str,
    ) {
        let pkg = PackageResolutionInfo {
            attr_path: pkg_path.to_string(),
            broken: false,
            derivation: String::new(),
            description: None,
            install_id: install_id.to_string(),
            license: None,
            locked_url: String::new(),
            name: String::new(),
            outputs: None,
            outputs_to_install: None,
            pname: String::new(),
            rev: String::new(),
            rev_count: 0,
            rev_date: chrono::offset::Utc::now(),
            scrape_date: chrono::offset::Utc::now(),
            stabilities: None,
            unfree: None,
            version: version.to_string(),
        };
        if let Some(page) = resolved_group.pages.first_mut() {
            let mut pkgs = page.packages.take().unwrap();
            pkgs.push(pkg);
            page.packages = Some(pkgs);
        } else {
            let page = CatalogPage {
                packages: Some(vec![pkg]),
                page: 0,
                url: String::new(),
            };
            resolved_group.pages = vec![page];
        }
    }

    /// combine_customizations() deduplicates a package and correctly concatenates customization scripts
    #[test]
    fn test_combine_customizations() {
        let customizations = vec![
            InitCustomization {
                hook_on_activate: Some("hook_on_activate1".to_string()),
                profile_common: Some("profile_common1".to_string()),
                profile_bash: Some("profile_bash1".to_string()),
                profile_fish: Some("profile_fish1".to_string()),
                profile_tcsh: Some("profile_tcsh1".to_string()),
                profile_zsh: Some("profile_zsh1".to_string()),
                packages: Some(vec![
                    PackageToInstall {
                        id: "pip".to_string(),
                        pkg_path: "python311Packages.pip".to_string(),
                        version: None,
                        input: None,
                    },
                    PackageToInstall {
                        id: "package2".to_string(),
                        pkg_path: "path2".to_string(),
                        version: None,
                        input: None,
                    },
                ]),
            },
            InitCustomization {
                hook_on_activate: Some("hook_on_activate2".to_string()),
                profile_common: Some("profile_common2".to_string()),
                profile_bash: Some("profile_bash2".to_string()),
                profile_fish: Some("profile_fish2".to_string()),
                profile_tcsh: Some("profile_tcsh2".to_string()),
                profile_zsh: Some("profile_zsh2".to_string()),
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
            hook_on_activate: Some(
                indoc! {r#"
                        # Autogenerated by Flox

                        hook_on_activate1

                        hook_on_activate2

                        # End autogenerated by Flox
                "#}
                .to_string()
            ),
            profile_common: Some(
                indoc! {r#"
                        # Autogenerated by Flox

                        profile_common1

                        profile_common2

                        # End autogenerated by Flox
                "#}
                .to_string()
            ),
            profile_bash: Some(
                indoc! {r#"
                        # Autogenerated by Flox

                        profile_bash1

                        profile_bash2

                        # End autogenerated by Flox
                "#}
                .to_string()
            ),
            profile_fish: Some(
                indoc! {r#"
                        # Autogenerated by Flox

                        profile_fish1

                        profile_fish2

                        # End autogenerated by Flox"#}
                .to_string()
            ),
            profile_tcsh: Some(
                indoc! {r#"
                        # Autogenerated by Flox

                        profile_tcsh1

                        profile_tcsh2

                        # End autogenerated by Flox"#}
                .to_string()
            ),
            profile_zsh: Some(
                indoc! {r#"
                        # Autogenerated by Flox

                        profile_zsh1

                        profile_zsh2

                        # End autogenerated by Flox
                "#}
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
                    id: "package2".to_string(),
                    pkg_path: "path2".to_string(),
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
        });
    }
}
