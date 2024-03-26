use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use anyhow::{Context, Error, Result};
use bpaf::Bpaf;
use flox_rust_sdk::flox::{EnvironmentName, Flox, DEFAULT_NAME};
use flox_rust_sdk::models::environment::path_environment::{InitCustomization, PathEnvironment};
use flox_rust_sdk::models::environment::{
    global_manifest_lockfile_path,
    global_manifest_path,
    CanonicalPath,
    Environment,
    PathPointer,
};
use flox_rust_sdk::models::lockfile::{LockedManifest, TypedLockedManifest};
use flox_rust_sdk::models::manifest::{insert_packages, PackageToInstall};
use flox_rust_sdk::models::pkgdb::scrape_input;
use flox_rust_sdk::models::search::{do_search, PathOrJson, Query, SearchParams, SearchResult};
use indoc::formatdoc;
use log::debug;
use toml_edit::{Document, Formatted, Item, Table, Value};
use tracing::instrument;

use crate::commands::{environment_description, ConcreteEnvironment};
use crate::subcommand_metric;
use crate::utils::dialog::{Dialog, Spinner};
use crate::utils::message;

mod node;
mod python;

use node::Node;
use python::Python;

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
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .context("Can't init in root")?;
            EnvironmentName::from_str(&name)?
        };

        // Don't run language hooks in home dir
        let customization = if dir != home_dir || self.auto_setup {
            Dialog {
                message: "Generating database for flox packages...",
                help_message: None,
                typed: Spinner::new(|| {
                    // Some language hooks run searches,
                    // so run a scrape first
                    let global_lockfile = LockedManifest::ensure_global_lockfile(&flox)?;
                    let lockfile: TypedLockedManifest =
                        LockedManifest::read_from_file(&CanonicalPath::new(global_lockfile)?)?
                            .try_into()?;
                    // --ga-registry forces a single input
                    if let Some((_, input)) = lockfile.registry().inputs.iter().next() {
                        scrape_input(&input.from)?;
                    };
                    Ok::<(), Error>(())
                }),
            }
            .spin_with_delay(Duration::from_secs_f32(0.25))?;

            self.run_language_hooks(&dir, &flox).unwrap_or_else(|e| {
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
    fn run_language_hooks(&self, dir: &Path, flox: &Flox) -> Result<InitCustomization> {
        let mut hooks: Vec<Box<dyn InitHook>> = vec![];

        let node = Node::new(dir, flox)?;
        hooks.push(Box::new(node));

        let python = Python::new(dir, flox);
        hooks.push(Box::new(python));

        let mut customizations = vec![];

        for mut hook in hooks {
            // Run hooks if we can't prompt
            if hook.should_run(dir)?
                && (self.auto_setup || (Dialog::can_prompt() && hook.prompt_user(dir, flox)?))
            {
                customizations.push(hook.get_init_customization())
            }
        }

        Ok(Self::combine_customizations(customizations))
    }

    /// Deduplicate packages and concatenate profiles into a single string
    fn combine_customizations(customizations: Vec<InitCustomization>) -> InitCustomization {
        let mut custom_profiles: Vec<String> = vec![];
        // Deduplicate packages with a set
        let mut packages_set = HashSet::<PackageToInstall>::new();
        for customization in customizations {
            if let Some(packages) = customization.packages {
                packages_set.extend(packages)
            }
            if let Some(hook) = customization.profile {
                custom_profiles.push(hook)
            }
        }

        let custom_profile = (!custom_profiles.is_empty()).then(|| {
            formatdoc! {"
                # Autogenerated by flox

                {}

                # End autogenerated by flox", custom_profiles.join("\n\n")}
        });

        let packages = (!packages_set.is_empty())
            .then(|| packages_set.into_iter().collect::<Vec<PackageToInstall>>());

        InitCustomization {
            profile: custom_profile,
            packages,
        }
    }
}

// TODO: clean up how we pass around path and flox
trait InitHook {
    fn should_run(&mut self, path: &Path) -> Result<bool>;

    fn prompt_user(&mut self, path: &Path, flox: &Flox) -> Result<bool>;

    fn get_init_customization(&self) -> InitCustomization;
}

/// Create a temporary TOML document containing just the contents of the passed
/// [InitCustomization], and return it as a string.
fn format_customization(customization: &InitCustomization) -> Result<String> {
    let mut toml = if let Some(packages) = &customization.packages {
        let with_packages = insert_packages("", packages)?;
        with_packages.new_toml.unwrap_or(Document::new())
    } else {
        Document::new()
    };

    if let Some(hook) = &customization.profile {
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

/// Get nixpkgs#rel_path optionally verifying that it satisfies a version constraint.
fn get_default_package_if_compatible(
    rel_path: impl IntoIterator<Item = impl ToString>,
    version: Option<String>,
    flox: &Flox,
) -> Result<Option<SearchResult>> {
    let rel_path = rel_path
        .into_iter()
        .map(|s| s.to_string())
        .collect::<Vec<String>>();

    let query = Query {
        rel_path: Some(rel_path),
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

/// Searches for a given pname and version, optionally restricting rel_path
fn try_find_compatible_version(
    pname: impl Into<String>,
    version: Option<impl Into<String>>,
    rel_path: Option<impl IntoIterator<Item = impl Into<String>>>,
    flox: &Flox,
) -> Result<Option<SearchResult>> {
    let rel_path = rel_path.map(|r| r.into_iter().map(|s| s.into()).collect::<Vec<String>>());

    let version = version.map(|v| v.into());

    let query = Query {
        pname: Some(pname.into()),
        semver: version,
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

#[cfg(test)]
mod tests {
    use flox_rust_sdk::flox::test_flox_instance;
    use indoc::indoc;
    use once_cell::sync::Lazy;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    use super::*;

    /// combine_customizations() deduplicates a package and corretly concatenates profiles
    #[test]
    fn test_combine_customizations() {
        let customizations = vec![
            InitCustomization {
                profile: Some("profile1".to_string()),
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
                profile: Some("profile2".to_string()),
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
            profile: Some(
                indoc! {r#"
                        # Autogenerated by flox

                        profile1

                        profile2

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

    pub static FLOX_INSTANCE: Lazy<(Flox, TempDir)> = Lazy::new(|| {
        let (flox, _temp_dir_handle) = test_flox_instance();
        let pkgdb_nixpkgs_rev_new = "ab5fd150146dcfe41fda501134e6503932cc8dfd";
        std::env::set_var("_PKGDB_GA_REGISTRY_REF_OR_REV", pkgdb_nixpkgs_rev_new);
        LockedManifest::update_global_manifest(&flox, vec![]).unwrap();
        (flox, _temp_dir_handle)
    });
}
