use std::collections::BTreeMap;
use std::fmt::Display;
use std::io::Write;

use crossterm::style::Stylize;
use flox_rust_sdk::data::System;
use flox_rust_sdk::models::lockfile::{LockedPackage, Lockfile, PackageOutputs};
use flox_rust_sdk::models::manifest::composite::{COMPOSER_MANIFEST_ID, Warning};
use flox_rust_sdk::models::manifest::raw::PackageToInstall;
use indoc::formatdoc;
use minus::{ExitStrategy, Pager, page_all};
use tracing::info;

/// Write a message to stderr.
///
/// This is printed via the message_layer tracing subscriber
fn print_message(v: impl Display) {
    info!("{v}");
}

fn print_message_to_buffer(out: &mut impl Write, v: impl Display) {
    writeln!(out, "{v}").unwrap();
}

/// alias for [print_message]
pub(crate) fn plain(v: impl Display) {
    print_message(v);
}
pub(crate) fn error(v: impl Display) {
    let icon = if stderr_supports_color() {
        "✘".red().to_string()
    } else {
        "✘".to_string()
    };
    print_message(std::format_args!("{icon} ERROR: {v}"));
}
pub(crate) fn created(v: impl Display) {
    let icon = if stderr_supports_color() {
        "⚡︎".yellow().to_string()
    } else {
        "⚡︎".to_string()
    };
    print_message(std::format_args!("{icon} {v}"));
}
/// double width character, add an additional space for alignment
pub(crate) fn deleted(v: impl Display) {
    let icon = if stderr_supports_color() {
        "━".red().to_string()
    } else {
        "━".to_string()
    };
    print_message(std::format_args!("{icon} {v}"));
}
pub(crate) fn updated(v: impl Display) {
    let icon = if stderr_supports_color() {
        "✔".green().to_string()
    } else {
        "✔".to_string()
    };
    print_message(std::format_args!("{icon} {v}"));
}
/// double width character, add an additional space for alignment
pub(crate) fn info(v: impl Display) {
    let icon = if stderr_supports_color() {
        "ℹ".blue().to_string()
    } else {
        "ℹ".to_string()
    };
    print_message(std::format_args!("{icon} {v}"));
}
/// double width character, add an additional space for alignment
pub(crate) fn warning(v: impl Display) {
    let icon = if stderr_supports_color() {
        "!".yellow().to_string()
    } else {
        "!".to_string()
    };
    print_message(std::format_args!("{icon} {v}"));
}

/// double width character, add an additional space for alignment
pub(crate) fn warning_to_buffer(out: &mut impl Write, v: impl Display) {
    let icon = if stderr_supports_color() {
        "!".yellow().to_string()
    } else {
        "!".to_string()
    };
    print_message_to_buffer(out, std::format_args!("{icon} {v}"));
}

pub(crate) fn package_installed(pkg: &PackageToInstall, environment_description: &str) {
    updated(format!(
        "'{}' installed to environment {environment_description}",
        pkg.id()
    ));
}

/// Page large output to terminal stdout.
/// The output will be printed without a pager if it's not larger than the
/// terminal window or the terminal is not interactive.
pub(crate) fn page_output(s: impl Into<String>) -> anyhow::Result<()> {
    let pager = Pager::new();

    // Allow destructors to run.
    pager.set_exit_strategy(ExitStrategy::PagerQuit)?;
    // Don't use pager if output fits in terminal.
    pager.set_run_no_overflow(false)?;

    pager.set_text(s)?;
    page_all(pager)?;

    Ok(())
}

pub fn stdout_supports_color() -> bool {
    supports_color::on(supports_color::Stream::Stdout).is_some()
}

pub fn stderr_supports_color() -> bool {
    supports_color::on(supports_color::Stream::Stderr).is_some()
}

/// Display a message for packages that were successfully installed for all
/// requested systems.
pub(crate) fn packages_successfully_installed(
    pkgs: &[PackageToInstall],
    environment_description: &str,
) {
    if !pkgs.is_empty() {
        let pkg_list = pkgs
            .iter()
            .map(|p| format!("'{}'", p.id()))
            .collect::<Vec<_>>()
            .join(", ");
        updated(format!(
            "{pkg_list} installed to environment {environment_description}"
        ));
    }
}

/// Display messages for each package that could only be installed for some of
/// the requested systems.
pub(crate) fn packages_installed_with_system_subsets(pkgs: &[PackageToInstall]) {
    for pkg in pkgs.iter() {
        warning(format!(
            "'{}' installed only for the following systems: {}",
            pkg.id(),
            // Only `None` for flakes, which can't reach this code
            // path anyway.
            pkg.systems().unwrap_or_default().join(", ")
        ))
    }
}

/// Display a message for packages that were requested but were already installed.
pub(crate) fn packages_already_installed(pkgs: &[PackageToInstall], environment_description: &str) {
    let already_installed_msg = match pkgs {
        [] => None,
        [pkg] => Some(format!(
            "Package with id '{}' already installed to environment {environment_description}",
            pkg.id()
        )),
        pkgs => {
            let joined = pkgs
                .iter()
                .map(|p| format!("'{}'", p.id()))
                .collect::<Vec<_>>();
            let joined = joined.join(", ");
            Some(format!(
                "Packages with ids {joined} already installed to environment {environment_description}"
            ))
        },
    };
    if let Some(msg) = already_installed_msg {
        warning(msg)
    }
}

/// Display a message for packages that are newly overridden by the composing manifest
pub(crate) fn packages_newly_overridden_by_composer(pkgs: &[String]) {
    let already_installed_msg = match pkgs {
        [] => None,
        [pkg] => Some(format!(
            "This environment now overrides package with id '{}'",
            pkg
        )),
        pkgs => {
            let joined = pkgs.iter().map(|p| format!("'{}'", p)).collect::<Vec<_>>();
            let joined = joined.join(", ");
            Some(format!(
                "This environment now overrides packages with ids {joined}"
            ))
        },
    };
    if let Some(msg) = already_installed_msg {
        info(msg)
    }
}

pub(crate) fn packages_with_additional_outputs(
    install_ids_of_new_pkgs: &[String],
    lockfile: &Lockfile,
    current_system: &System,
) {
    let mut pkgs_with_additional_outputs = vec![];
    let pkgs = lockfile.packages.as_slice();
    // Yes this is n^2, but n is small
    for install_id in install_ids_of_new_pkgs.iter() {
        for pkg in pkgs.iter() {
            if (pkg.install_id() == install_id) && (pkg.system() == current_system) {
                match pkg {
                    LockedPackage::Catalog(locked) => {
                        if !locked
                            .outputs_match_outputs_to_install()
                            .is_some_and(|value| value)
                        {
                            pkgs_with_additional_outputs.push(install_id);
                        }
                    },
                    LockedPackage::Flake(locked) => {
                        if !locked
                            .outputs_match_outputs_to_install()
                            .is_some_and(|value| value)
                        {
                            pkgs_with_additional_outputs.push(install_id);
                        }
                    },
                    _ => {},
                }
            }
        }
    }
    let maybe_msg = match pkgs_with_additional_outputs.as_slice() {
        [] => None,
        [pkg] => Some(format!(
            "'{pkg}' has additional outputs, use 'flox list -a' to see more"
        )),
        pkgs => {
            let joined = pkgs.iter().map(|p| format!("'{}'", p)).collect::<Vec<_>>();
            let joined = joined.join(", ");
            Some(format!(
                "{joined} have additional outputs, use 'flox list -a' to see more"
            ))
        },
    };
    if let Some(msg) = maybe_msg {
        info(msg)
    }
}

/// Format a list of overridden fields for an environment.
fn format_overridden_fields(fields: &[String]) -> String {
    fields
        .iter()
        .map(|key| format!("  - {}", key))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Print notices for any environments that have overridden fields during composition.
pub(crate) fn print_overridden_manifest_fields(lockfile: &Lockfile) {
    let Some(ref compose) = lockfile.compose else {
        return;
    };

    type Field = String;
    type Environment = String;

    // De-duplicate fields by the last "winning" environment.
    let winning_env_by_field: BTreeMap<Field, Environment> = compose
        .warnings
        .iter()
        .filter_map(|warning_context| match &warning_context.warning {
            Warning::Overriding(field) => Some((
                field.to_string(),
                warning_context.higher_priority_name.clone(),
            )),
            _ => None,
        })
        .collect();

    // Invert the de-duplicated map.
    let mut fields_by_env: BTreeMap<Environment, Vec<Field>> = BTreeMap::new();
    for (field, env) in winning_env_by_field {
        fields_by_env.entry(env).or_default().push(field);
    }

    // Sort the notices by the order that the environments were included and
    // then the current composer environment (if present) last.
    let mut messages_by_env: Vec<String> = Vec::new();
    let ordered_envs = compose.include.iter().map(|include| include.name.clone());
    for env in ordered_envs {
        if let Some(fields) = fields_by_env.get(&env) {
            messages_by_env.push(format!(
                "- Environment '{}' set:\n{}",
                env,
                format_overridden_fields(fields),
            ));
        }
    }
    if let Some(fields) = fields_by_env.get(COMPOSER_MANIFEST_ID) {
        messages_by_env.push(format!(
            "- This environment set:\n{}",
            format_overridden_fields(fields),
        ));
    }
    if !messages_by_env.is_empty() {
        let message = formatdoc! {"
                The following manifest fields were overridden during merging:
                {}", messages_by_env.join("\n")
        };
        info(message);
    }
}

#[cfg(test)]
mod tests {
    use flox_rust_sdk::flox::test_helpers::flox_instance;
    use flox_rust_sdk::models::environment::Environment;
    use flox_rust_sdk::models::environment::path_environment::test_helpers::new_path_environment;
    use flox_rust_sdk::utils::logging::test_helpers::test_subscriber_message_only;
    use indoc::indoc;
    use pretty_assertions::assert_eq;
    use tracing::instrument::WithSubscriber;

    use super::*;

    #[tokio::test]
    async fn test_print_overridden_manifest_fields() {
        let (flox, _tempdir) = flox_instance();

        let mut dep1 = new_path_environment(&flox, indoc! {r#"
            version = 1

            [vars]
            overridden_by_all = "set by dep1"
            overridden_by_dep2 = "set by dep1"
            overridden_by_composer = "set by dep1"
        "#});
        dep1.lockfile(&flox).unwrap();

        let mut dep2 = new_path_environment(&flox, indoc! {r#"
            version = 1

            [vars]
            overridden_by_all = "updated by dep2"
            overridden_by_dep2 = "updated by dep2"
        "#});
        dep2.lockfile(&flox).unwrap();

        let composer_original_manifest = formatdoc! {r#"
            version = 1

            [vars]
            overridden_by_all = "updated by composer"
            overridden_by_composer = "updated by composer"

            [include]
            environments = [
                {{ dir = "{dep1_dir}", name = "dep_one" }},
                {{ dir = "{dep2_dir}", name = "dep_two" }},
            ]"#,
            dep1_dir = dep1.parent_path().unwrap().to_string_lossy(),
            dep2_dir = dep2.parent_path().unwrap().to_string_lossy(),
        };
        let mut composer = new_path_environment(&flox, &composer_original_manifest);
        let lockfile = composer.lockfile(&flox).unwrap().into();

        let (subscriber, writer) = test_subscriber_message_only();
        async {
            print_overridden_manifest_fields(&lockfile);
        }
        .with_subscriber(subscriber)
        .await;

        // - environmemnts are listed by the order they were included
        // - composer environment is listed last
        // - environment `dep_one` doesn't appear because its fields are overridden later
        assert_eq!(writer.to_string(), indoc! {"
            ℹ The following manifest fields were overridden during merging:
            - Environment 'dep_two' set:
              - vars.overridden_by_dep2
            - This environment set:
              - vars.overridden_by_all
              - vars.overridden_by_composer
            "});
    }
}
