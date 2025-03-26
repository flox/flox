use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result, bail};
use bpaf::Bpaf;
use flox_rust_sdk::data::CanonicalPath;
use flox_rust_sdk::flox::{DEFAULT_NAME, EnvironmentName, Flox};
use flox_rust_sdk::models::environment::path_environment::{InitCustomization, PathEnvironment};
use flox_rust_sdk::models::environment::{
    CoreEnvironmentError,
    Environment,
    EnvironmentError,
    InstallationAttempt,
    PathPointer,
};
use flox_rust_sdk::models::lockfile::{
    LockedPackage,
    Lockfile,
    ResolutionFailure,
    ResolutionFailures,
    ResolveError,
};
use flox_rust_sdk::models::manifest::raw::{
    CatalogPackage,
    PackageToInstall,
    catalog_packages_to_install,
};
use flox_rust_sdk::models::manifest::typed::ActivateMode;
use flox_rust_sdk::models::user_state::{
    lock_and_read_user_state_file,
    user_state_path,
    write_user_state_file,
};
use flox_rust_sdk::providers::catalog::{
    MsgAttrPathNotFoundNotFoundForAllSystems,
    MsgAttrPathNotFoundNotInCatalog,
};
use indoc::formatdoc;
use itertools::Itertools;
use tracing::{debug, info_span, instrument, span, warn};

use super::services::warn_manifest_changes_for_services;
use super::{EnvironmentSelect, environment_select};
use crate::commands::activate::Activate;
use crate::commands::{
    ConcreteEnvironment,
    EnvironmentSelectError,
    ensure_floxhub_token,
    environment_description,
};
use crate::utils::dialog::{Dialog, Select};
use crate::utils::didyoumean::{DidYouMean, InstallSuggestion};
use crate::utils::errors::format_error;
use crate::utils::message;
use crate::utils::openers::Shell;
use crate::utils::tracing::sentry_set_tag;
use crate::{Exit, environment_subcommand_metric, subcommand_metric};

// Install a package into an environment
#[derive(Bpaf, Clone)]
pub struct Install {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    /// Option to specify a package ID
    #[bpaf(external(pkg_with_id_option), many)]
    id: Vec<PkgWithIdOption>,

    /// Packages specified without explicit ids
    #[bpaf(positional("packages"))]
    packages: Vec<String>,
}

#[derive(Debug, Bpaf, Clone)]
#[bpaf(adjacent)]
#[allow(clippy::manual_non_exhaustive)]
pub struct PkgWithIdOption {
    /// Install a package and assign an explicit ID
    #[bpaf(long("id"), short('i'))]
    _option: (),

    /// ID of the package to install
    #[bpaf(positional("id"))]
    pub id: String,

    /// The pkg-path of the package to install as shown by 'flox search'
    ///
    /// Append `@<version>` to specify a version requirement
    #[bpaf(positional("package"))]
    pub pkg: String,
}

/// A container for packages during a retry attempt.
///
/// If some packages fail to resolve because they lack all of the requested
/// systems, we retry the installation with only the available systems. In that
/// case we warn about those packages, but install the other packages as usual.
/// This container allows us to record whether we should warn or not during
/// this second installation attempt.
#[derive(Debug, Clone)]
struct PackageToInstallRetry {
    system_subset: bool,
    pkg: PackageToInstall,
}

/// A container for the packages that a user attempted to install.
#[derive(Debug, Clone)]
struct PartitionedPackages {
    successes: Vec<PackageToInstall>,
    system_subsets: Vec<PackageToInstall>,
    already_installed: Vec<PackageToInstall>,
}

impl Install {
    #[instrument(name = "install", skip_all)]
    pub async fn handle(self, mut flox: Flox) -> Result<()> {
        environment_subcommand_metric!("install", self.environment);

        debug!(
            "attempting to install packages [{}] to {:?}",
            self.packages.as_slice().join(", "),
            self.environment
        );

        // Ensure the user is logged in for the following remote operations
        if let EnvironmentSelect::Remote(_) = self.environment {
            ensure_floxhub_token(&mut flox).await?;
        }

        let mut packages_to_install = self
            .packages
            .iter()
            .map(|p| PackageToInstall::parse(&flox.system, p))
            .collect::<Result<Vec<_>, _>>()?;
        let pkgs_with_ids = self
            .id
            .iter()
            .map(|p| {
                let mut pkg = PackageToInstall::parse(&flox.system, &p.pkg);
                if let Ok(ref mut pkg) = pkg {
                    pkg.set_id(&p.id);
                }
                pkg
            })
            .collect::<Result<Vec<_>, _>>()?;
        packages_to_install.extend(pkgs_with_ids.into_iter());
        if packages_to_install.is_empty() {
            bail!("Must specify at least one package");
        }

        let concrete_environment = match self
            .environment
            .detect_concrete_environment(&flox, "Install to")
        {
            Ok(concrete_environment) => concrete_environment,
            Err(EnvironmentSelectError::EnvironmentError(
                ref e @ EnvironmentError::DotFloxNotFound(ref dir),
            )) => {
                let parent = dir.parent().unwrap_or(dir).display();
                bail!(formatdoc! {"
                {e}

                Create an environment with 'flox init --dir {parent}'"
                })
            },
            Err(e @ EnvironmentSelectError::EnvNotFoundInCurrentDirectory) => {
                let bail_message = formatdoc! {"
                    {e}

                    Create an environment with 'flox init' or install to an environment found elsewhere with 'flox install {} --dir <PATH>'",
                self.packages.join(" ")};
                if !Dialog::can_prompt() {
                    bail!(bail_message);
                }
                let user_state_path = user_state_path(&flox);
                let (lock, mut user_state) = lock_and_read_user_state_file(&user_state_path)?;
                if user_state.confirmed_create_default_env.is_some() {
                    bail!(bail_message);
                }
                let msg = formatdoc! {"
                    Packages must be installed into a Flox environment, which can be
                    a user 'default' environment or attached to a directory.
                "};
                message::plain(msg);
                let package_list = package_list_for_prompt(&packages_to_install)
                    .context("must specify at least one package to install")?;
                let (choice_idx, _) = Dialog {
                    message: &format!(
                        "Would you like to install {package_list} to the 'default' environment?"
                    ),
                    help_message: None,
                    typed: Select {
                        options: vec!["Yes", "No"],
                    },
                }
                .raw_prompt()?;
                let should_install_to_default_env = choice_idx == 0;
                if !should_install_to_default_env {
                    user_state.confirmed_create_default_env = Some(false);
                    write_user_state_file(&user_state, &user_state_path, lock)
                        .context("failed to save default environment choice")?;
                    let msg = format!(
                        "Create an environment with 'flox init' or install to an environment found elsewhere with 'flox install {} --dir <PATH>'",
                        self.packages.join(" ")
                    );
                    message::plain(msg);
                    return Err(Exit(1.into()).into());
                }
                let env = create_default_env(&flox)?;
                user_state.confirmed_create_default_env = Some(should_install_to_default_env);
                write_user_state_file(&user_state, &user_state_path, lock)
                    .context("failed to save default environment choice")?;
                prompt_to_modify_rc_file()?;
                ConcreteEnvironment::Path(env)
            },
            Err(EnvironmentSelectError::Anyhow(e)) => Err(e)?,
            Err(e) => Err(e)?,
        };
        let description = environment_description(&concrete_environment)?;

        let mut environment = concrete_environment.into_dyn_environment();

        // We don't know the contents of the packages field when the span is created
        sentry_set_tag(
            "packages",
            Install::format_packages_for_tracing(&packages_to_install),
        );

        let span = info_span!(
            "install",
            progress = format!(
                "Installing {} package(s) to environment {description}...",
                packages_to_install.len()
            )
        );
        let installation = span.in_scope(|| environment.install(&packages_to_install, &flox));

        let (packages_retried, installation) = match installation {
            Ok(installation) => (None, installation),
            Err(err) => {
                if let Some((failures, packages_retry)) =
                    Self::need_retry_with_valid_systems(&err, &packages_to_install)
                {
                    let res = Self::retry_install_for_valid_systems(
                        &flox,
                        environment.as_mut(),
                        failures,
                        &packages_retry,
                    );
                    match res {
                        Ok(installation) => (Some(packages_retry), installation),
                        Err(err) => (None, Self::handle_error(err, &flox, &packages_to_install)?),
                    }
                } else {
                    (None, Self::handle_error(err, &flox, &packages_to_install)?)
                }
            },
        };

        let lockfile_path = environment.lockfile_path(&flox)?;
        let lockfile_path = CanonicalPath::new(lockfile_path)?;
        let lockfile_content = std::fs::read_to_string(&lockfile_path)?;

        // Check for warnings in the lockfile
        let lockfile: Lockfile = serde_json::from_str(&lockfile_content)?;
        // TODO: move this behind the `installation.new_manifest.is_some()`
        // check below so we don't warn when we don't even install anything
        for warning in Self::generate_warnings(
            &lockfile.packages,
            &catalog_packages_to_install(&packages_to_install),
        ) {
            message::warning(warning);
        }

        let installed = if let Some(packages_retried) = packages_retried {
            packages_retried
        } else {
            packages_to_install
                .iter()
                .map(|p| PackageToInstallRetry {
                    system_subset: false,
                    pkg: p.clone(),
                })
                .collect::<Vec<_>>()
        };
        let partitioned =
            Self::partition_installed_packages(&installed, &installation.already_installed);

        // Print status messages for the installation attempt
        message::packages_successfully_installed(&partitioned.successes, &description);
        message::packages_installed_with_system_subsets(&partitioned.system_subsets);
        message::packages_already_installed(&partitioned.already_installed, &description);

        if installation.new_manifest.is_some() {
            warn_manifest_changes_for_services(&flox, environment.as_ref());
        }

        Ok(())
    }

    fn format_packages_for_tracing(packages: &[PackageToInstall]) -> String {
        packages
            .iter()
            .map(|p| match p {
                PackageToInstall::Catalog(pkg) => pkg.pkg_path.clone(),
                PackageToInstall::Flake(pkg) => pkg.url.to_string(),
                PackageToInstall::StorePath(pkg) => pkg.store_path.display().to_string(),
            })
            .join(",")
    }

    fn partition_installed_packages(
        pkgs: &[PackageToInstallRetry],
        already_installed_map: &HashMap<String, bool>,
    ) -> PartitionedPackages {
        let (partials, maybe_successes): (Vec<_>, Vec<_>) =
            pkgs.iter().partition(|p| p.system_subset);
        let partials = partials
            .into_iter()
            .cloned()
            .map(|p| p.pkg)
            .collect::<Vec<_>>();
        let maybe_successes = maybe_successes
            .into_iter()
            .cloned()
            .map(|p| p.pkg)
            .collect::<Vec<_>>();
        let (successes, already_installed): (Vec<_>, Vec<_>) = maybe_successes
            .into_iter()
            .partition(|p| already_installed_map.get(p.id()).is_some_and(|v| !*v));
        PartitionedPackages {
            successes,
            system_subsets: partials,
            already_installed,
        }
    }

    fn need_retry_with_valid_systems(
        err: &EnvironmentError,
        requested_packages: &[PackageToInstall],
    ) -> Option<(ResolutionFailures, Vec<PackageToInstallRetry>)> {
        match err {
            EnvironmentError::Core(CoreEnvironmentError::Resolve(
                ResolveError::ResolutionFailed(failures),
            )) if failures.0.iter().all(|f| {
                matches!(f, ResolutionFailure::PackageUnavailableOnSomeSystems { .. })
            }) =>
            {
                let mut packages = requested_packages
                    .iter()
                    .cloned()
                    .map(|p| {
                        (p.id().to_string(), PackageToInstallRetry {
                            system_subset: false,
                            pkg: p,
                        })
                    })
                    .collect::<HashMap<_, _>>();

                for failure in &failures.0 {
                    let ResolutionFailure::PackageUnavailableOnSomeSystems {
                        catalog_message:
                            MsgAttrPathNotFoundNotFoundForAllSystems {
                                install_id,
                                valid_systems,
                                ..
                            },
                        invalid_systems: _,
                    } = failure
                    else {
                        unreachable!(
                            "already checked that these failures are 'package unavailable on some systems'"
                        )
                    };

                    let Some(pkg_retry) = packages.get_mut(install_id) else {
                        warn!(install_id, "resolution failure for non-existent package");
                        continue;
                    };

                    let PackageToInstall::Catalog(CatalogPackage {
                        ref mut systems, ..
                    }) = pkg_retry.pkg
                    else {
                        warn!(
                            install_id,
                            ?pkg_retry.pkg,
                            "resolution failure for non-catalog package"
                        );
                        continue;
                    };

                    // Update this package retry
                    *systems = Some(valid_systems.clone());
                    pkg_retry.system_subset = true;
                }

                let packages = packages.into_values().collect::<Vec<_>>();
                Some((failures.clone(), packages))
            },
            _ => None,
        }
    }

    fn retry_install_for_valid_systems(
        flox: &Flox,
        environment: &mut dyn Environment,
        failures: ResolutionFailures,
        packages_to_retry: &[PackageToInstallRetry],
    ) -> Result<InstallationAttempt, EnvironmentError> {
        let span = span!(
            tracing::Level::INFO,
            "install",
            progress = "Installing packages for available systems"
        );
        let packages_installable = packages_to_retry
            .iter()
            .cloned()
            .map(|p| p.pkg)
            .collect::<Vec<_>>();
        let install_result = span.in_scope(|| environment.install(&packages_installable, flox));

        match install_result {
            Ok(install_attempt) => Ok(install_attempt),
            Err(err) => {
                debug!("install error: {:?}", err);
                let mut failures = failures;
                let msg = formatdoc! {"
                    While attempting to install for available systems, the following error occurred:
                    {err}
                    ", err = format_error(&err).trim()
                };
                failures.0.push(ResolutionFailure::FallbackMessage { msg });
                Err(EnvironmentError::Core(CoreEnvironmentError::Resolve(
                    ResolveError::ResolutionFailed(failures),
                )))
            },
        }
    }

    /// Handle an error that occurred during installation.
    /// Some errors are recoverable and will return with [Ok].
    fn handle_error(
        err: EnvironmentError,
        flox: &Flox,
        packages: &[PackageToInstall],
    ) -> Result<InstallationAttempt> {
        debug!("install error: {:?}", err);

        subcommand_metric!(
            "install",
            "failed_packages" = Install::format_packages_for_tracing(packages)
        );

        match err {
            // Try to make suggestions when a package isn't found
            EnvironmentError::Core(CoreEnvironmentError::Resolve(
                ResolveError::ResolutionFailed(failures),
            )) => {
                let (need_didyoumean, mut other_failures): (Vec<_>, Vec<_>) = failures
                    .0
                    .into_iter()
                    .partition(|f| matches!(f, ResolutionFailure::PackageNotFound { .. }));
                // Essentially we're going to convert the `PackageNotFound` variants into
                // `FallbackMessage` variants, which are just strings we're going to generate
                // with `DidYouMean`.
                // We use `DidYouMean` to generate the suggestions,
                // separately from attempting an install,
                // or other kind of resolution.
                // This is because `DidYouMean` may take an unknown amount of time,
                // performing a search.
                // For the same reason `DidYouMean` is also showing a spinner
                // while the search is in progress.
                for failure in need_didyoumean.into_iter() {
                    let ResolutionFailure::PackageNotFound(MsgAttrPathNotFoundNotInCatalog {
                        attr_path,
                        ..
                    }) = failure
                    else {
                        unreachable!("already checked that these failures are 'package not found'")
                    };
                    let suggestion = DidYouMean::<InstallSuggestion>::new(flox, &attr_path);
                    let head = format!("Could not find package '{attr_path}'.");
                    let msg = if suggestion.has_suggestions() {
                        tracing::debug!(query = attr_path, "found suggestions for package");
                        formatdoc! {"
                        {head}
                        {suggestion}"}
                    } else {
                        format!("{head}\nTry 'flox search' with a broader search term.")
                    };
                    other_failures.push(ResolutionFailure::FallbackMessage { msg });
                }
                Err(EnvironmentError::Core(CoreEnvironmentError::Resolve(
                    ResolveError::ResolutionFailed(ResolutionFailures(other_failures)),
                ))
                .into())
            },
            err => Err(err.into()),
        }
    }

    /// Generate warnings to print to the user about unfree and broken packages.
    fn generate_warnings(
        locked_packages: &[LockedPackage],
        packages_to_install: &[CatalogPackage],
    ) -> Vec<String> {
        let mut warnings = vec![];

        // There could be multiple packages with the same install_id but different systems.
        // A package could be broken on one system but not another.
        // So just keep track of which install_ids we've warned for.
        // TODO: does the warning need to take system into account?
        let mut warned_unfree = HashSet::new();
        let mut warned_broken = HashSet::new();
        for locked_package in locked_packages.iter() {
            // If unfree && just installed && we haven't already warned for this install_id,
            // warn that this package is unfree
            if locked_package.unfree() == Some(true)
                && packages_to_install
                    .iter()
                    .any(|p| locked_package.install_id() == p.id)
                && !warned_unfree.contains(&locked_package.install_id())
            {
                warnings.push(format!("The package '{}' has an unfree license, please verify the licensing terms of use", locked_package.install_id()));
                warned_unfree.insert(locked_package.install_id());
            }

            // If broken && just installed && we haven't already warned for this install_id,
            // warn that this package is broken
            if locked_package.broken() == Some(true)
                && packages_to_install
                    .iter()
                    .any(|p| locked_package.install_id() == p.id)
                && !warned_broken.contains(&locked_package.install_id())
            {
                warnings.push(format!("The package '{}' is marked as broken, it may not behave as expected during runtime.", locked_package.install_id()));
                warned_broken.insert(locked_package.install_id());
            }
        }
        warnings
    }
}

/// Returns a formatted string representing a possibly truncated list of
/// packages to install.
fn package_list_for_prompt(packages: &[PackageToInstall]) -> Option<String> {
    match packages {
        [] => None,
        [p] => Some(format!("'{}'", p.id())),
        [first, second] => Some(format!("'{}, {}'", first.id(), second.id())),
        [first, second, ..] => Some(format!("'{}, {}, ...'", first.id(), second.id())),
    }
}

/// Creates a default environment for the user, skipping checks for init
/// customizations and skipping the normal `init` output.
fn create_default_env(flox: &Flox) -> Result<PathEnvironment, anyhow::Error> {
    let home_dir = dirs::home_dir().context("user must have a home directory")?;
    let customization = InitCustomization {
        activate_mode: Some(ActivateMode::Run),
        ..Default::default()
    };
    PathEnvironment::init(
        PathPointer::new(
            EnvironmentName::from_str(DEFAULT_NAME)
                .context("'default' is a known-valid environment name")?,
        ),
        &home_dir,
        &customization,
        flox,
    )
    .context("failed to initialize default environment")
}

fn prompt_to_modify_rc_file() -> Result<bool, anyhow::Error> {
    let shell = Activate::detect_shell_for_in_place()?;
    let shell_cmd = match shell {
        // TODO: should we use source <(flox activate -d ~) for bash?
        // There are unicode quoting issues with the current form
        // We can't use <() for zsh because it blocks input which can make it
        // impossible to Ctrl-C
        Shell::Bash(_) | Shell::Zsh(_) => r#"eval "$(flox activate -d ~ -m run)""#,
        Shell::Tcsh(_) => r#"eval "`flox activate -d ~ -m run`""#,
        Shell::Fish(_) => "flox activate -d ~ -m run | source",
    };
    let rc_file_names = match shell {
        Shell::Bash(_) => vec![".bashrc", ".profile"],
        Shell::Zsh(_) => vec![".zshrc", ".zprofile"],
        Shell::Tcsh(_) => vec![".tcshrc"],
        Shell::Fish(_) => vec!["config.fish"],
    };
    let joined = rc_file_names.join(" and ");
    let msg = |files: &[&str]| {
        let file_or_files = if files.len() > 1 { "files" } else { "file" };
        formatdoc! {"
            The 'default' environment can be activated automatically for every new shell
            by adding one line to your {joined} {file_or_files}:
            {shell_cmd}
        "}
    };

    message::plain(msg(&rc_file_names));
    let prompt = format!("Would you like Flox to add this configuration to {joined} now?");
    let (choice_idx, _) = Dialog {
        message: &prompt,
        help_message: None,
        typed: Select {
            options: vec!["Yes", "No"],
        },
    }
    .raw_prompt()?;
    let should_modify_rc_file = choice_idx == 0;

    let read_more_msg = formatdoc! {"
        -> Read more about the 'default' environment at:
           https://flox.dev/docs/tutorials/default-environment/"};
    let restart_msg = formatdoc! {"
        The 'default' environment will be activated for every new shell.
        -> Restart your shell to continue using the default environment."};
    if !should_modify_rc_file {
        message::plain(&read_more_msg);
        return Ok(false);
    }
    for rc_file_name in rc_file_names.iter() {
        let rc_file_path = locate_rc_file(&shell, rc_file_name)?;
        ensure_rc_file_exists(&rc_file_path)?;
        add_activation_to_rc_file(&rc_file_path, shell_cmd)?;
        message::updated(format!("Configuration added to your {rc_file_name} file."));
    }
    message::plain(&restart_msg);
    message::plain(&read_more_msg);
    message::plain(""); // need a blank line before package installation result
    Ok(true)
}

fn locate_rc_file(shell: &Shell, name: impl AsRef<str>) -> Result<PathBuf, anyhow::Error> {
    use Shell::*;
    let home = dirs::home_dir().context("failed to locate home directory")?;
    let rc_file = match shell {
        Bash(_) => home.join(name.as_ref()),
        Zsh(_) => home.join(name.as_ref()),
        Tcsh(_) => home.join(name.as_ref()),
        // Note, this `.config` is _not_ what you get from `dirs::config_dir`,
        // which points at `Application Support`
        Fish(_) => home.join(".config/fish").join(name.as_ref()),
    };
    Ok(rc_file)
}

fn ensure_rc_file_exists(path: impl AsRef<Path>) -> Result<(), anyhow::Error> {
    let path = path.as_ref();
    if !path.exists() {
        std::fs::create_dir_all(path.parent().context("RC file had no parent")?)
            .context("failed to create parent directory for RC file")?;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .context("failed to create empty RC file")?;
    }
    Ok(())
}

fn add_activation_to_rc_file(
    path: impl AsRef<Path>,
    cmd: impl AsRef<str>,
) -> Result<(), anyhow::Error> {
    let backup = path.as_ref().with_extension(".pre_flox");
    if backup.exists() {
        std::fs::remove_file(&backup).context("failed to remove old backup of RC file")?;
    }
    std::fs::copy(&path, backup).context("failed to make backup of RC file")?;
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .context("failed to open RC file")?;
    file.write(format!("{}\n", cmd.as_ref()).as_bytes())
        .context("failed to write to RC file")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use flox_rust_sdk::flox::test_helpers::flox_instance;
    use flox_rust_sdk::models::environment::path_environment::test_helpers::new_path_environment_in;
    use flox_rust_sdk::models::lockfile::LockedPackageCatalog;
    use flox_rust_sdk::models::lockfile::test_helpers::fake_catalog_package_lock;
    use flox_rust_sdk::models::manifest::raw::{CatalogPackage, PackageToInstall};
    use flox_rust_sdk::providers::catalog::{Client, GENERATED_DATA, MockClient, SystemEnum};
    use flox_test_utils::manifests::EMPTY_ALL_SYSTEMS;

    use super::{add_activation_to_rc_file, ensure_rc_file_exists};
    use crate::commands::EnvironmentSelect;
    use crate::commands::install::{Install, package_list_for_prompt};
    use crate::utils::message::history::History;

    /// [Install::generate_warnings] shouldn't warn for packages not in packages_to_install
    #[test]
    fn generate_warnings_empty() {
        let locked_packages = vec![];
        let packages_to_install = vec![];
        assert_eq!(
            Install::generate_warnings(&locked_packages, &packages_to_install),
            Vec::<String>::new()
        );
    }

    /// [Install::generate_warnings] should warn for an unfree package
    #[test]
    fn generate_warnings_unfree() {
        let (foo_iid, _, mut foo_locked) = fake_catalog_package_lock("foo", None);
        foo_locked.unfree = Some(true);
        let locked_packages = vec![foo_locked.into()];
        let packages_to_install = vec![CatalogPackage {
            id: foo_iid.clone(),
            pkg_path: "foo".to_string(),
            version: None,
            systems: None,
        }];
        assert_eq!(
            Install::generate_warnings(&locked_packages, packages_to_install.as_slice()),
            vec![format!(
                "The package '{foo_iid}' has an unfree license, please verify the licensing terms of use"
            )]
        );
    }

    /// [Install::generate_warnings] should only warn for an unfree package once
    /// even if it's installed on multiple systems
    #[test]
    fn generate_warnings_unfree_multi_system() {
        let (foo_iid, _, mut foo_locked) = fake_catalog_package_lock("foo", None);
        foo_locked.unfree = Some(true);

        // TODO: fake_package shouldn't hardcode system?
        let foo_locked_second_system = LockedPackageCatalog {
            system: SystemEnum::Aarch64Linux.to_string(),
            ..foo_locked.clone()
        };

        let locked_packages = vec![foo_locked.into(), foo_locked_second_system.into()];
        let packages_to_install = vec![CatalogPackage {
            id: foo_iid.clone(),
            pkg_path: "foo".to_string(),
            version: None,
            systems: None,
        }];
        assert_eq!(
            Install::generate_warnings(&locked_packages, packages_to_install.as_slice()),
            vec![format!(
                "The package '{foo_iid}' has an unfree license, please verify the licensing terms of use"
            )]
        );
    }

    /// [Install::generate_warnings] should warn for a broken package
    #[test]
    fn generate_warnings_broken() {
        let (foo_iid, _, mut foo_locked) = fake_catalog_package_lock("foo", None);
        foo_locked.broken = Some(true);
        let locked_packages = vec![foo_locked.into()];
        let packages_to_install = vec![CatalogPackage {
            id: foo_iid.clone(),
            pkg_path: "foo".to_string(),
            version: None,
            systems: None,
        }];
        assert_eq!(
            Install::generate_warnings(&locked_packages, packages_to_install.as_slice()),
            vec![format!(
                "The package '{foo_iid}' is marked as broken, it may not behave as expected during runtime."
            )]
        );
    }

    /// [Install::generate_warnings] should only warn for a broken package once
    /// even if it's installed on multiple systems
    #[test]
    fn generate_warnings_broken_multi_system() {
        let (foo_iid, _, mut foo_locked) = fake_catalog_package_lock("foo", None);
        foo_locked.broken = Some(true);

        // TODO: fake_package shouldn't hardcode system?
        let foo_locked_second_system = LockedPackageCatalog {
            system: SystemEnum::Aarch64Linux.to_string(),
            ..foo_locked.clone()
        };

        let locked_packages = vec![foo_locked.into(), foo_locked_second_system.into()];
        let packages_to_install = vec![CatalogPackage {
            id: foo_iid.clone(),
            pkg_path: "foo".to_string(),
            version: None,
            systems: None,
        }];
        assert_eq!(
            Install::generate_warnings(&locked_packages, packages_to_install.as_slice()),
            vec![format!(
                "The package '{foo_iid}' is marked as broken, it may not behave as expected during runtime."
            )]
        );
    }

    #[test]
    fn package_list_for_prompt_is_formatted_correctly() {
        let packages = vec![
            PackageToInstall::parse(&"dummy-system".to_string(), "hello").unwrap(),
            PackageToInstall::parse(&"dummy-system".to_string(), "ripgrep").unwrap(),
            PackageToInstall::parse(&"dummy-system".to_string(), "bpftrace").unwrap(),
        ];
        assert_eq!(
            format!("'hello'"),
            package_list_for_prompt(&packages[0..1]).unwrap()
        );
        assert_eq!(
            format!("'hello, ripgrep'"),
            package_list_for_prompt(&packages[0..2]).unwrap()
        );
        assert_eq!(
            format!("'hello, ripgrep, ...'"),
            package_list_for_prompt(&packages).unwrap()
        );
    }

    #[test]
    fn creates_rc_file_if_parent_doesnt_exist() {
        let tmpdir = tempfile::tempdir().unwrap();
        let parent = tmpdir.path().join("foo");
        let rc_file_path = parent.join(".bashrc");
        ensure_rc_file_exists(&rc_file_path).unwrap();
        assert!(rc_file_path.exists());
    }

    #[test]
    fn creates_rc_file_if_doesnt_exist() {
        let tmpdir = tempfile::tempdir().unwrap();
        let rc_file_path = tmpdir.path().join(".bashrc");
        ensure_rc_file_exists(&rc_file_path).unwrap();
        assert!(rc_file_path.exists());
    }

    #[test]
    fn creates_rc_file_backup() {
        let tmpdir = tempfile::tempdir().unwrap();
        let rc_file_path = tmpdir.path().join(".bashrc");
        ensure_rc_file_exists(&rc_file_path).unwrap();
        let backup = rc_file_path.with_extension(".pre_flox");
        add_activation_to_rc_file(&rc_file_path, "be activated").unwrap();
        assert!(backup.exists());
    }

    #[tokio::test]
    async fn warns_about_incomplete_system_availability() {
        let (mut flox, tempdir) = flox_instance();
        let is_linux = flox.system.ends_with("linux");
        let response_path = if is_linux {
            GENERATED_DATA.join("resolve/darwin_ps_all.json")
        } else {
            GENERATED_DATA.join("resolve/bpftrace.json")
        };
        let pkg_path = if is_linux { "darwin.ps" } else { "bpftrace" };
        let install_id = if is_linux { "ps" } else { "bpftrace" };
        let installed_systems = if is_linux {
            "aarch64-darwin, x86_64-darwin"
        } else {
            "aarch64-linux, x86_64-linux"
        };
        let client = MockClient::new(Some(response_path)).unwrap();
        flox.catalog_client = Client::Mock(client);

        let _env = new_path_environment_in(&flox, EMPTY_ALL_SYSTEMS, tempdir.path());
        let install_cmd = Install {
            environment: EnvironmentSelect::Dir(tempdir.path().to_path_buf()),
            id: vec![],
            packages: vec![pkg_path.to_string()],
        };
        install_cmd.handle(flox).await.expect("installation failed");
        let msgs = &History::global().messages();
        let expected = format!(
            "⚠\u{fe0f}  '{install_id}' installed only for the following systems: {installed_systems}"
        );
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0], expected);
    }
}
