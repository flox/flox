use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result};
use bpaf::Bpaf;
use flox_rust_sdk::flox::{EnvironmentName, Flox};
use flox_rust_sdk::models::environment::path_environment::{InitCustomization, PathEnvironment};
use flox_rust_sdk::models::environment::{Environment, PathPointer};
use flox_rust_sdk::models::manifest::{insert_packages, PackageToInstall};
use indoc::formatdoc;
use toml_edit::{Document, Formatted, Item, Table, Value};

use crate::commands::{environment_description, ConcreteEnvironment};
use crate::subcommand_metric;
use crate::utils::dialog::{Dialog, Select, Spinner};
use crate::utils::message;

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
            EnvironmentName::from_str("default")?
        } else {
            let name = dir
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .context("Can't init in root")?;
            EnvironmentName::from_str(&name)?
        };

        // Don't run hooks in home dir
        let customization = (dir != home_dir)
            .then(|| self.run_hooks(&dir))
            .transpose()?;

        let env = if let Some(InitCustomization {
            packages: Some(_), ..
        }) = customization
        {
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
        if let Some(InitCustomization {
            packages: Some(packages),
            ..
        }) = customization
        {
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
    fn run_hooks(&self, dir: &Path) -> Result<InitCustomization> {
        let hooks: [Box<dyn InitHook>; 1] = [Box::new(Requirements)];

        let mut customizations = vec![];

        for hook in hooks {
            // Run hooks if we can't prompt
            if hook.should_run(dir)
                && (self.auto_setup || !Dialog::can_prompt() || hook.prompt_user()?)
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

trait InitHook {
    fn should_run(&self, path: &Path) -> bool;

    fn prompt_user(&self) -> Result<bool>;

    fn get_init_customization(&self) -> InitCustomization;
}

struct Requirements;

impl InitHook for Requirements {
    fn should_run(&self, path: &Path) -> bool {
        path.join("requirements.txt").exists()
    }

    fn prompt_user(&self) -> Result<bool> {
        let message = formatdoc! {"
            Flox detected a requirements.txt

            Python projects typically need:
            * python, pip
            * A Python virtual environment to install dependencies into

            Would you like Flox to apply the standard Python environment?
            You can always revisit the environment's declaration with 'flox edit'
        "};

        let help = "Use '--auto-setup' to apply Flox recommendations in the future.";

        let dialog = Dialog {
            message: &message,
            help_message: Some(help),
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

            let help = "Use '--auto-setup' to apply Flox recommendations in the future.";

            let dialog = Dialog {
                message: &message,
                help_message: Some(help),
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
            hook: Some("# TODO".to_string()),
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

        assert_eq!(
            Init::combine_customizations(customizations),
            InitCustomization {
                hook: Some(
                    formatdoc! {"
                        # Autogenerated by flox

                        # TODO

                        hook2

                        # End autogenerated by flox"}
                    .to_string()
                ),
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
                    PackageToInstall {
                        id: "python3".to_string(),
                        pkg_path: "python311Packages.python".to_string(),
                        version: None,
                        input: None,
                    },
                ]),
            }
        );
    }
}
