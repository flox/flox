use std::collections::BTreeMap;
use std::fmt::Display;
use std::io::Write;

use flox_rust_sdk::models::lockfile::Lockfile;
use flox_rust_sdk::models::manifest::composite::{COMPOSER_MANIFEST_ID, Warning};
use flox_rust_sdk::models::manifest::raw::PackageToInstall;
use indoc::formatdoc;
use tracing::info;

/// Write a message to stderr.
///
/// This is a wrapper around `eprintln!` that can be further extended
/// to include logging, word wrapping, ANSI filtereing etc.
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
    print_message(std::format_args!("❌ ERROR: {v}"));
}
pub(crate) fn created(v: impl Display) {
    print_message(std::format_args!("✨ {v}"));
}
/// double width character, add an additional space for alignment
pub(crate) fn deleted(v: impl Display) {
    print_message(std::format_args!("🗑️  {v}"));
}
pub(crate) fn updated(v: impl Display) {
    print_message(std::format_args!("✅ {v}"));
}
/// double width character, add an additional space for alignment
pub(crate) fn info(v: impl Display) {
    print_message(std::format_args!("ℹ️  {v}"));
}
/// double width character, add an additional space for alignment
pub(crate) fn warning(v: impl Display) {
    print_message(std::format_args!("⚠️  {v}"));
}

/// double width character, add an additional space for alignment
pub(crate) fn warning_to_buffer(out: &mut impl Write, v: impl Display) {
    print_message_to_buffer(out, std::format_args!("⚠️  {v}"));
}

pub(crate) fn package_installed(pkg: &PackageToInstall, environment_description: &str) {
    updated(format!(
        "'{}' installed to environment {environment_description}",
        pkg.id()
    ));
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
            ℹ️  The following manifest fields were overridden during merging:
            - Environment 'dep_two' set:
              - vars.overridden_by_dep2
            - This environment set:
              - vars.overridden_by_all
              - vars.overridden_by_composer
            "});
    }
}
