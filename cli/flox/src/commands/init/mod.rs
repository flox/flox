use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result, anyhow, bail};
use bpaf::Bpaf;
use flox_core::activate::mode::ActivateMode;
use flox_core::data::environment_ref::{DEFAULT_NAME, EnvironmentName, RemoteEnvironmentRef};
use flox_manifest::raw::{CatalogPackage, PackageToInstall};
use flox_rust_sdk::data::AttrPath;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::path_environment::{InitCustomization, PathEnvironment};
use flox_rust_sdk::models::environment::remote_environment::RemoteEnvironment;
use flox_rust_sdk::models::environment::{ConcreteEnvironment, Environment, PathPointer};
use flox_rust_sdk::providers::catalog::{
    ALL_SYSTEMS,
    ClientTrait,
    PackageDescriptor,
    PackageGroup,
    PackageResolutionInfo,
};
use flox_rust_sdk::providers::git::{GitCommandProvider, GitProvider};
use flox_rust_sdk::providers::manifest_init::ManifestInitializer;
use indoc::{formatdoc, indoc};
use path_dedot::ParseDot;
use tracing::{debug, info_span, instrument};

use crate::commands::{SHELL_COMPLETION_DIR, ensure_floxhub_token, environment_description};
use crate::subcommand_metric;
use crate::utils::dialog::Dialog;
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

#[derive(Bpaf, Clone)]
enum InitEnvironmentTypeSelect {
    Path {
        /// Directory to create the environment in (default: current directory)
        #[bpaf(long, short, argument("path"), complete_shell(SHELL_COMPLETION_DIR))]
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

        /// Don't auto-detect language support for a project or
        /// make suggestions.
        #[bpaf(long)]
        no_auto_setup: bool,
    },
    FloxHub {
        /// A FloxHub environment
        #[bpaf(long("reference"), long("ref"), short('r'), argument("owner>/<name"))]
        environment_ref: RemoteEnvironmentRef,
    },
}

// Create an environment in the current directory
#[derive(Bpaf, Clone)]
pub struct Init {
    #[bpaf(external(init_environment_type_select))]
    type_select: InitEnvironmentTypeSelect,

    /// Set up the environment with the emptiest possible manifest.
    #[bpaf(short, long)]
    bare: bool,
}

impl Init {
    #[instrument(name = "init", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("init");

        match self.type_select {
            InitEnvironmentTypeSelect::Path {
                dir,
                env_name,
                auto_setup,
                no_auto_setup,
            } => {
                let dir = match dir {
                    Some(dir) => dir.clone(),
                    None => {
                        std::env::current_dir().context("Couldn't determine current directory")?
                    },
                };

                let Some(home_dir) = dirs::home_dir() else {
                    bail!("Couldn't determine home directory");
                };

                let default_environment = dir == home_dir;

                let env_name = if let Some(ref name) = env_name {
                    EnvironmentName::from_str(name)?
                } else if default_environment {
                    EnvironmentName::from_str(DEFAULT_NAME)?
                } else {
                    let name = dir
                        .parse_dot()?
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .context("Can't init in root")?;
                    EnvironmentName::from_str(&slug::slugify(name))?
                };

                let do_auto_setup = AutoSetupBehavior::from_arguments(
                    auto_setup,
                    no_auto_setup,
                    default_environment,
                    self.bare,
                );

                init_local_environment(&flox, &dir, &env_name, self.bare, do_auto_setup).await?;
            },
            InitEnvironmentTypeSelect::FloxHub { environment_ref } => {
                let mut flox = flox;
                ensure_floxhub_token(&mut flox).await?;
                init_floxhub_environment_decorated(&flox, environment_ref, self.bare)?;
            },
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AutoSetupBehavior {
    Skip,
    RunAndPrompt,
    RunAndForce,
}

impl AutoSetupBehavior {
    /// Derive the setup behavior from the three overlapping flags
    /// and runtime working dir.
    ///
    /// By default we run setup hooks and prompt users for confirmation.
    /// With `--auto-setup` we always run hooks and apply them without further input.
    /// Unless `--auto-setup` is present, providing `--no-auto-setup`, or `--bare`,
    /// or operating in the home directory/creating a local default environment,
    /// will skip running init hooks.
    fn from_arguments(
        auto_setup: bool,
        no_auto_setup: bool,
        is_default_environment: bool,
        is_bare: bool,
    ) -> Self {
        if auto_setup {
            return AutoSetupBehavior::RunAndForce;
        }

        if no_auto_setup || is_bare || is_default_environment {
            return AutoSetupBehavior::Skip;
        }

        AutoSetupBehavior::RunAndPrompt
    }
}

// TODO: clean up how we pass around path and flox
trait InitHook {
    async fn prompt_user(&mut self, flox: &Flox, path: &Path) -> Result<bool>;

    fn get_init_customization(&self) -> InitCustomization;
}

async fn init_local_environment(
    flox: &Flox,
    dir: &Path,
    name: &EnvironmentName,
    bare: bool,
    auto_setup: AutoSetupBehavior,
) -> Result<()> {
    let mut customization = if auto_setup != AutoSetupBehavior::Skip {
        debug!("attempting auto-setup");
        run_language_hooks(flox, dir, auto_setup == AutoSetupBehavior::RunAndForce)
            .await
            .unwrap_or_else(|e| {
                message::warning(format!("Failed to generate init suggestions: {e}"));
                InitCustomization::default()
            })
    } else {
        debug!("skipping auto-setup");
        InitCustomization::default()
    };

    if Some(dir) == std::env::home_dir().as_deref() {
        debug!("environment in home-dir initialized in runtime mode");
        customization.activate_mode = Some(ActivateMode::Run);
    }

    let path_pointer = PathPointer::new(name.clone());
    let env = if customization.packages.is_some() {
        info_span!(
            "init_with_suggested_packages",
            progress = "Installing Flox suggested packages"
        )
        .in_scope(|| PathEnvironment::init(path_pointer, dir, &customization, flox))?
    } else if bare {
        debug!("creating environment with bare manifest");
        PathEnvironment::init_bare(path_pointer, dir, flox)?
    } else {
        debug!("creating environment");
        PathEnvironment::init(path_pointer, dir, &customization, flox)?
    };

    let env_in_git_repo = GitCommandProvider::discover(dir).is_ok();

    message::created(format!(
        "Created environment '{name}' ({system})",
        name = env.name(),
        system = flox.system
    ));
    if let Some(packages) = customization.packages {
        let description = environment_description(&ConcreteEnvironment::Path(env))?;
        for package in packages {
            message::package_installed(&PackageToInstall::Catalog(package), &description);
        }
    }

    message::plain(indoc! {"

        Next:
          $ flox search <package>    <- Search for a package
          $ flox install <package>   <- Install a package into an environment
          $ flox activate            <- Enter the environment
          $ flox edit                <- Add environment variables and shell hooks"
    });

    // Don't recommend `flox push` if the env is in a git repo because they
    // can already git track it and there's a higher likelihood of using
    // build+publish, subsets of which require a git repo, and don't work
    // with remote environments.
    if !env_in_git_repo {
        let hint_indentation = "  ";
        message::plain(formatdoc! {"
            {}$ flox push                <- Use the environment from other machines or
                                            share it with someone on FloxHub"
        , hint_indentation});
    }

    message::plain("");
    Ok(())
}

/// Same as [RemoteEnvironment::init_floxhub_environment]
/// but with added decoration/messaging on success.
fn init_floxhub_environment_decorated(
    flox: &Flox,
    env_ref: RemoteEnvironmentRef,
    bare: bool,
) -> Result<()> {
    RemoteEnvironment::init_floxhub_environment(flox, env_ref.clone(), bare)?;
    message::created(format!("Created environment '{env_ref}'"));
    message::plain(formatdoc! {"

        Next:
          Search for a package                      -> $ flox search <package>
          Install a package into an environment     -> $ flox install -r {env_ref} <package>
          Enter the environment                     -> $ flox activate -r {env_ref}
          Add environment variables and shell hooks -> $ flox edit -r {env_ref}"
    });
    Ok(())
}

/// Run all language hooks and return a single combined customization
async fn run_language_hooks(
    flox: &Flox,
    path: &Path,
    force_auto_setup: bool,
) -> Result<InitCustomization> {
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
        if force_auto_setup || (Dialog::can_prompt() && hook.prompt_user(flox, path).await?) {
            customizations.push(hook.get_init_customization())
        }
    }

    Ok(combine_customizations(customizations))
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
    let mut packages_set = HashSet::<CatalogPackage>::new();
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

            # End autogenerated by Flox
            ", custom_profile_fish_scripts.join("\n\n")}
    });
    let custom_profile_tcsh = (!custom_profile_tcsh_scripts.is_empty()).then(|| {
        formatdoc! {"
            # Autogenerated by Flox

            {}

            # End autogenerated by Flox
            ", custom_profile_tcsh_scripts.join("\n\n")}
    });
    let custom_profile_zsh = (!custom_profile_zsh_scripts.is_empty()).then(|| {
        formatdoc! {"
            # Autogenerated by Flox

            {}

            # End autogenerated by Flox
            ", custom_profile_zsh_scripts.join("\n\n")}
    });

    let packages = (!packages_set.is_empty())
        .then(|| packages_set.into_iter().collect::<Vec<CatalogPackage>>());

    InitCustomization {
        hook_on_activate: custom_hook_on_activate,
        profile_common: custom_profile_common,
        profile_bash: custom_profile_bash,
        profile_fish: custom_profile_fish,
        profile_tcsh: custom_profile_tcsh,
        profile_zsh: custom_profile_zsh,
        packages,
        activate_mode: None, // Language hooks don't touch mode.
    }
}

/// Create a temporary TOML document containing just the contents of the passed
/// [InitCustomization], and return it as a string.
fn format_customization(customization: &InitCustomization) -> Result<String> {
    let mut customization = customization.clone();
    // combine_customizations will add a newline after scripts,
    // so do the same here
    for script in [
        &mut customization.hook_on_activate,
        &mut customization.profile_common,
        &mut customization.profile_bash,
        &mut customization.profile_fish,
        &mut customization.profile_tcsh,
        &mut customization.profile_zsh,
    ]
    .into_iter()
    // Clippy prefers stripping out None with flatten()
    .flatten()
    {
        *script = format!("{}\n", script);
    }

    Ok(ManifestInitializer::new_minimal(&customization).to_string())
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
    /// pname or the last component of [Self::attr_path]
    pub name: String,
    /// Path to the package in the catalog
    /// Checked to be non-empty
    pub attr_path: AttrPath,
    /// Version of the package in the catalog
    /// "N/A" if not found
    ///
    /// Used for display purposes only,
    /// version constraints should be added based on the original query.
    pub display_version: String,
    /// The actual version of the package
    pub version: Option<String>,
}

impl From<ProvidedPackage> for CatalogPackage {
    fn from(value: ProvidedPackage) -> Self {
        CatalogPackage {
            id: value.name,
            pkg_path: value.attr_path.into(),
            version: value.version,
            systems: None,
            outputs: None,
        }
    }
}

impl From<PackageResolutionInfo> for ProvidedPackage {
    fn from(value: PackageResolutionInfo) -> Self {
        Self {
            name: value.install_id,
            attr_path: value.attr_path.into(),
            display_version: value.version.clone(),
            version: Some(value.version),
        }
    }
}

impl From<&PackageResolutionInfo> for ProvidedPackage {
    fn from(value: &PackageResolutionInfo) -> Self {
        Self {
            name: value.install_id.clone(),
            attr_path: value.attr_path.clone().into(),
            display_version: value.version.clone(),
            version: Some(value.version.clone()),
        }
    }
}

/// Searches for a given attr_path and optional version, returning an error if
/// there are no matches.
async fn find_compatible_package(
    flox: &Flox,
    attr_path: &str,
    version: Option<&str>,
) -> Result<ProvidedPackage> {
    match try_find_compatible_package(flox, attr_path, version).await? {
        Some(pkg) => Ok(pkg),
        None => Err(anyhow!(
            "Flox couldn't find any compatible versions of {attr_path}"
        )),
    }
}

/// Searches for a given attr_path and optional version
async fn try_find_compatible_package(
    flox: &Flox,
    attr_path: &str,
    version: Option<&str>,
) -> Result<Option<ProvidedPackage>> {
    let pkg = {
        tracing::debug!(
            attr_path,
            version = version.unwrap_or("null"),
            "using catalog client to find compatible package version"
        );

        let resolved_groups = flox
            .catalog_client
            .resolve(vec![PackageGroup {
                descriptors: vec![PackageDescriptor {
                    attr_path: attr_path.to_string(),
                    install_id: attr_path.to_string(),
                    version: version.map(|v| v.to_string()),
                    allow_pre_releases: None,
                    derivation: None,
                    allow_broken: None,
                    allow_insecure: None,
                    allow_unfree: None,
                    allowed_licenses: None,
                    allow_missing_builds: None,
                    systems: ALL_SYSTEMS.to_vec(),
                }],
                name: attr_path.to_string(),
            }])
            .await?;
        let pkg: Option<ProvidedPackage> = resolved_groups
            .first()
            .and_then(|pkg_group| pkg_group.page.as_ref())
            .and_then(|page| page.packages.as_ref())
            .and_then(|pkgs| pkgs.first().cloned())
            .map(|pkg| {
                // Type-inference fails without the fully-qualified method call
                <PackageResolutionInfo as Into<ProvidedPackage>>::into(pkg)
            });
        let Some(pkg) = pkg else {
            tracing::debug!(attr_path, "no compatible package version found");
            return Ok(None);
        };
        pkg
    };

    tracing::debug!(
        attr_path,
        version = pkg.version.as_ref().unwrap_or(&"null".to_string()),
        "found matching package version"
    );
    Ok(Some(pkg))
}

/// For languages like Node, Python, etc where there are separate packages for
/// each major version, attempt to find the major version package that matches
/// a semver requirement.
///
/// Submits a single request with a separate package group for each major version
/// package, and only returns those that matched the semver requirement.
async fn try_find_compatible_major_version_package(
    flox: &Flox,
    description: &str, // only used for logging
    major_version_packages: &[impl AsRef<str>],
    version: Option<&str>,
) -> Result<Vec<ProvidedPackage>> {
    tracing::debug!(
        package = description,
        version = version.unwrap_or("null"),
        "using catalog client to find compatible major version package"
    );

    let pkg_groups = major_version_packages
        .iter()
        .map(|pkg_name| group_for_single_package(pkg_name.as_ref(), version))
        .collect::<Vec<_>>();
    let resolved_groups = flox.catalog_client.resolve(pkg_groups).await?;
    let candidate_pkgs: Vec<ProvidedPackage> = resolved_groups
        .into_iter()
        .filter_map(|maybe_pkg_group| {
            maybe_pkg_group
                .page
                .as_ref()
                .and_then(|page| page.packages.as_ref())
                .and_then(|pkgs| pkgs.first().cloned())
        })
        .map(|pkg| {
            // Type-inference fails without the fully-qualified method call
            <PackageResolutionInfo as Into<ProvidedPackage>>::into(pkg)
        })
        .collect::<Vec<_>>();

    if candidate_pkgs.is_empty() {
        tracing::debug!(package = description, "no compatible package version found");
    } else {
        let found = candidate_pkgs
            .iter()
            .map(|pkg| pkg.attr_path.to_string())
            .collect::<Vec<_>>();
        tracing::debug!(
            found = found.join(","),
            "found matching major version package"
        );
    }

    Ok(candidate_pkgs)
}

fn group_for_single_package(attr_path: &str, version: Option<&str>) -> PackageGroup {
    PackageGroup {
        descriptors: vec![PackageDescriptor {
            attr_path: attr_path.to_string(),
            install_id: attr_path.to_string(),
            version: version.map(|v| v.to_string()),
            allow_pre_releases: None,
            derivation: None,
            allow_broken: None,
            allow_insecure: None,
            allow_unfree: None,
            allowed_licenses: None,
            allow_missing_builds: None,
            systems: ALL_SYSTEMS.to_vec(),
        }],
        name: attr_path.to_string(),
    }
}

#[cfg(test)]
mod tests {

    use flox_core::data::environment_ref::EnvironmentOwner;
    use flox_rust_sdk::flox::test_helpers::flox_instance_with_optional_floxhub;
    use flox_rust_sdk::models::environment::ManagedPointer;
    use flox_rust_sdk::utils::logging::test_helpers::test_subscriber_message_only;
    use indoc::indoc;
    use pretty_assertions::assert_eq;

    use super::*;

    impl ProvidedPackage {
        pub(crate) fn new(
            name: impl ToString,
            attr_path: impl IntoIterator<Item = impl ToString>,
            version: &str,
        ) -> Self {
            Self {
                name: name.to_string(),
                attr_path: attr_path
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
                    CatalogPackage {
                        id: "pip".to_string(),
                        pkg_path: "python311Packages.pip".to_string(),
                        version: None,
                        systems: None,
                        outputs: None,
                    },
                    CatalogPackage {
                        id: "package2".to_string(),
                        pkg_path: "path2".to_string(),
                        version: None,
                        systems: None,
                        outputs: None,
                    },
                ]),
                activate_mode: None,
            },
            InitCustomization {
                hook_on_activate: Some("hook_on_activate2".to_string()),
                profile_common: Some("profile_common2".to_string()),
                profile_bash: Some("profile_bash2".to_string()),
                profile_fish: Some("profile_fish2".to_string()),
                profile_tcsh: Some("profile_tcsh2".to_string()),
                profile_zsh: Some("profile_zsh2".to_string()),
                packages: Some(vec![
                    CatalogPackage {
                        id: "pip".to_string(),
                        pkg_path: "python311Packages.pip".to_string(),
                        version: None,
                        systems: None,
                        outputs: None,
                    },
                    CatalogPackage {
                        id: "package1".to_string(),
                        pkg_path: "path1".to_string(),
                        version: None,
                        systems: None,
                        outputs: None,
                    },
                ]),
                activate_mode: None,
            },
        ];

        let mut combined = combine_customizations(customizations);
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

                        # End autogenerated by Flox
                "#}
                .to_string()
            ),
            profile_tcsh: Some(
                indoc! {r#"
                        # Autogenerated by Flox

                        profile_tcsh1

                        profile_tcsh2

                        # End autogenerated by Flox
                "#}
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
                CatalogPackage {
                    id: "package1".to_string(),
                    pkg_path: "path1".to_string(),
                    version: None,
                    systems: None,
                    outputs: None
                },
                CatalogPackage {
                    id: "package2".to_string(),
                    pkg_path: "path2".to_string(),
                    version: None,
                    systems: None,
                    outputs: None
                },
                CatalogPackage {
                    id: "pip".to_string(),
                    pkg_path: "python311Packages.pip".to_string(),
                    version: None,
                    systems: None,
                    outputs: None
                },
            ]),
            activate_mode: None,
        });
    }

    /// Verify that format_customization() correctly converts InitCustomization to TOML.
    #[test]
    fn format_customization_exhaustive() {
        // Create a test InitCustomization with various fields populated
        let customization = InitCustomization {
            hook_on_activate: Some("echo 'Activating environment'".to_string()),
            profile_common: Some("export COMMON_VAR=value".to_string()),
            profile_bash: Some("export BASH_VAR=value".to_string()),
            profile_fish: Some("set -x FISH_VAR value".to_string()),
            profile_tcsh: Some("setenv TCSH_VAR value".to_string()),
            profile_zsh: Some("export ZSH_VAR=value".to_string()),
            packages: Some(vec![CatalogPackage {
                id: "test-package".to_string(),
                pkg_path: "test.package".to_string(),
                version: Some("1.0.0".to_string()),
                systems: None,
                outputs: None,
            }]),
            activate_mode: None,
        };

        let toml_str = format_customization(&customization).unwrap();
        // Use indoc to create the expected TOML with proper indentation
        let expected_toml = indoc! {r#"
            [install]
            test-package.pkg-path = "test.package"
            test-package.version = "1.0.0"

            [hook]
            on-activate = """
              echo 'Activating environment'
            """

            [profile]
            common = """
              export COMMON_VAR=value
            """
            bash = """
              export BASH_VAR=value
            """
            fish = """
              set -x FISH_VAR value
            """
            tcsh = """
              setenv TCSH_VAR value
            """
            zsh = """
              export ZSH_VAR=value
            """
        "#};

        // Compare the generated TOML string with our expected output
        assert_eq!(toml_str, expected_toml);
    }

    /// Ensures we don't add empty tables or comments we don't want
    #[test]
    fn format_customization_empty() {
        let customization = InitCustomization::default();
        let toml_str = format_customization(&customization).unwrap();
        assert_eq!(toml_str, "");
    }

    #[test]
    fn init_floxhub_environment_initializes_and_prints_message() {
        let owner = EnvironmentOwner::from_str("test").unwrap();
        let name = EnvironmentName::from_str("foo").unwrap();
        let env_ref = RemoteEnvironmentRef::new_from_parts(owner.clone(), name.clone());

        let (flox, _tempdir_handle) = flox_instance_with_optional_floxhub(Some(&owner));
        let (subscriber, written) = test_subscriber_message_only();

        tracing::subscriber::with_default(subscriber, || {
            init_floxhub_environment_decorated(&flox, env_ref.clone(), false)
        })
        .unwrap();

        assert!(
            written
                .to_string()
                .contains(&format!("Created environment '{env_ref}'"))
        );

        assert!(
            written
                .to_string()
                .contains("Add environment variables and shell hooks")
        );

        RemoteEnvironment::new(&flox, ManagedPointer::new(owner, name, &flox.floxhub), None)
            .expect("find initialized remote environment");
    }
}
