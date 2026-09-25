use std::env;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Instant;

// `::` selects the extern `nix` crate rather than the
// `flox_rust_sdk::providers::nix` module that is also in scope.
use ::nix::sys::signal::Signal;
use anyhow::{Context, Result, bail};
use bpaf::Bpaf;
use flox_core::data::CanonicalPath;
use flox_events::{CliBuildPayload, EventKind, EventsHub, Outcome};
use flox_manifest::lockfile::Lockfile;
use flox_manifest::{Manifest, MigratedTypedOnly};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::{ConcreteEnvironment, Environment};
use flox_rust_sdk::providers::build::{
    COMMON_NIXPKGS_URL,
    FloxBuildMk,
    ManifestBuilder,
    ManifestBuilderError,
    PackageTarget,
    PackageTargetKind,
    PackageTargets,
    locked_nixpkgs_urls,
    nix_expression_dir,
};
use flox_rust_sdk::providers::catalog::base_catalog_url_for_stability_arg;
use flox_rust_sdk::providers::git::{GitCommandProvider, GitProvider};
use flox_rust_sdk::providers::nix;
use flox_rust_sdk::utils::{CommandExt, FLOX_INTERPRETER};
use floxhub_client::{BaseCatalogUrl, CatalogClientTrait, FloxhubClientError};
use indoc::formatdoc;
use itertools::Itertools;
use nef_lock_catalog::{NixFlakeref, catalog_lockfile_path, lock_project_catalog};
use thiserror::Error;
use tracing::{debug, instrument, trace};
use url::Url;

use super::{DirEnvironmentSelect, dir_environment_select, needs_project_files_error};
use crate::utils::catalog_lock::BuildLockGuard;
use crate::utils::events::duration_to_ms;
use crate::utils::message;
use crate::{environment_subcommand_metric, subcommand_metric};

/// How the user invokes the catalog-lock update, for messages that name it.
///
/// NAMING: provisional — the command is expected to be renamed (or folded
/// into a flag such as `flox build --lock-catalog`) once the UX discussion
/// settles. Keep the user-visible name confined to this constant and the
/// `UpdateCatalogs` bpaf declaration so the rename stays a two-line change.
pub(crate) const UPDATE_CATALOGS_COMMAND: &str = "flox build update-catalogs";

#[derive(Debug, Clone, Bpaf)]
pub enum BaseCatalogUrlSelect {
    NixpkgsUrl(#[bpaf(long("nixpkgs-url"), argument("url"), hide)] Url),
    Stability(
        #[bpaf(
            long("stability"),
            argument("stability"),
            help("Select the nixpkgs revision by stability, as tracked by the catalog server.")
        )]
        String,
    ),
}

/// Reusable system override option for commands that need to specify a target system
#[derive(Debug, Default, Bpaf, Clone)]
pub struct SystemOverride {
    #[bpaf(
        argument("system"),
        hide,
        help(
            "Override the Nix system.\n\
            This is used to build packages for a different system than the current system.\n\
            If not specified, the current system as reported by nix is used.\n"
        )
    )]
    system: Option<String>,
}

impl SystemOverride {
    pub fn into_inner(self) -> Option<String> {
        self.system
    }
}

#[derive(Bpaf, Clone)]
pub struct Build {
    #[bpaf(external(dir_environment_select), fallback(Default::default()))]
    environment: DirEnvironmentSelect,

    #[bpaf(external(subcommand_or_build_targets))]
    subcommand_or_targets: SubcommandOrBuildTargets,
}

#[derive(Debug, Bpaf, Clone)]
enum SubcommandOrBuildTargets {
    /// Clean the build directory
    ///
    /// Removes build artifacts and temporary files.
    #[bpaf(command, footer("Run 'man flox-build-clean' for more details."))]
    Clean {
        /// The package(s) to clean.
        /// Corresponds to entries in the 'build' table in the environment's manifest.toml.
        /// If not specified, all packages are cleaned up.
        #[bpaf(positional("package"))]
        targets: Vec<String>,
    },
    /// Import package definition from nixpkgs
    ///
    /// Imports a package definition from nixpkgs for use in the environment.
    #[bpaf(
        command,
        footer("Run 'man flox-build-import-nixpkgs' for more details.")
    )]
    ImportNixpkgs {
        /// Overwrite existing package file
        #[bpaf(long, short)]
        force: bool,

        #[bpaf(external(base_catalog_url_select), optional)]
        base_catalog_url_select: Option<BaseCatalogUrlSelect>,

        /// The package to import (e.g., nixpkgs#hello, github:nixos/nixpkgs#hello)
        #[bpaf(positional("installable"))]
        installable: String,
    },
    /// Update catalog lockfile
    ///
    /// Scans the project's Nix expression builds for catalog references and
    /// locks them to '.flox/catalog.lock', the single set of pinned catalog
    /// inputs every build of the project evaluates against.
    //
    // NAMING: provisional; see the note at [UPDATE_CATALOGS_COMMAND] before
    // renaming.
    #[bpaf(
        command,
        footer("Run 'man flox-build-update-catalogs' for more details.")
    )]
    UpdateCatalogs {},
    BuildTargets {
        #[bpaf(external(base_catalog_url_select), optional)]
        base_catalog_url_select: Option<BaseCatalogUrlSelect>,

        #[bpaf(external(system_override))]
        system_override: SystemOverride,

        /// The package to build.
        /// Corresponds to entries in the 'build' table in the environment's manifest.toml.
        /// If not specified, all packages are built.
        #[bpaf(positional("package"))]
        targets: Vec<String>,
    },
}

impl Build {
    /// Centrally-derived subcommand string for this invocation.
    /// Returns the `build::clean` / `build::import-nixpkgs` /
    /// `build::update-catalogs` form for the build pseudo-subcommands,
    /// preserving the join-key continuity the legacy
    /// `environment_subcommand_metric!` stream already used at
    /// `cli/flox/src/commands/build.rs:146,154,162`.
    pub fn subcommand_name(&self) -> &'static str {
        match &self.subcommand_or_targets {
            SubcommandOrBuildTargets::Clean { .. } => "build::clean",
            SubcommandOrBuildTargets::ImportNixpkgs { .. } => "build::import-nixpkgs",
            SubcommandOrBuildTargets::UpdateCatalogs { .. } => "build::update-catalogs",
            SubcommandOrBuildTargets::BuildTargets { .. } => "build",
        }
    }

    pub async fn handle(self, mut flox: Flox) -> Result<()> {
        match self.subcommand_or_targets {
            SubcommandOrBuildTargets::Clean { targets } => {
                let env = self
                    .environment
                    .detect_concrete_environment(&mut flox, "Clean build files of")?;
                environment_subcommand_metric!("build::clean", env);

                Self::clean(flox, env, targets).await
            },
            SubcommandOrBuildTargets::ImportNixpkgs {
                installable,
                force,
                base_catalog_url_select,
            } => {
                let env = self
                    .environment
                    .detect_concrete_environment(&mut flox, "Import package definition in")?;
                environment_subcommand_metric!("build::import-nixpkgs", env);

                Self::import_nixpkgs(flox, env, installable, force, base_catalog_url_select).await
            },
            SubcommandOrBuildTargets::UpdateCatalogs {} => {
                let env = self
                    .environment
                    .detect_concrete_environment(&mut flox, "Update catalogs in")?;
                environment_subcommand_metric!("build::update-catalogs", env);

                Self::update_catalogs(&flox, env).await
            },
            SubcommandOrBuildTargets::BuildTargets {
                targets,
                base_catalog_url_select,
                system_override,
            } => {
                let env = self
                    .environment
                    .detect_concrete_environment(&mut flox, "Build packages of")?;
                environment_subcommand_metric!("build", env);

                Self::build(
                    flox,
                    env,
                    targets,
                    base_catalog_url_select,
                    system_override.into_inner(),
                )
                .await
            },
        }
    }

    #[instrument(name = "build::clean", skip_all)]
    async fn clean(flox: Flox, mut env: ConcreteEnvironment, packages: Vec<String>) -> Result<()> {
        match &env {
            ConcreteEnvironment::Path(_) => (),
            ConcreteEnvironment::Managed(managed) => {
                bail!(needs_project_files_error(managed, "build"))
            },
            ConcreteEnvironment::Remote(_) => {
                // guarded by DirEnvironmentSelect
                unreachable!("Cannot build from a remote environment")
            },
        };

        let base_dir = env.parent_path()?;
        let expression_ref = NixFlakeref::from_path(env.dot_flox_path())?; // TODO: decouple from env
        let flox_env_build_outputs = env.build(&flox)?;
        let lockfile: Lockfile = env.lockfile(&flox)?.into();

        let lockfile_manifest = lockfile.migrated_manifest()?;
        let packages_to_clean = packages_to_build(&lockfile_manifest, &expression_ref, &packages)?;
        let target_names = packages_to_clean
            .iter()
            .map(|target| target.name())
            .collect::<Vec<_>>();

        let cache_path = env.cache_path()?;
        let builder = FloxBuildMk::new(
            &flox,
            &base_dir,
            &expression_ref,
            &flox_env_build_outputs,
            &cache_path,
        );
        builder.clean(&target_names)?;

        message::updated(format!(
            "Cleaned build targets: {}",
            target_names.iter().join(", ")
        ));

        Ok(())
    }

    #[instrument(name = "build", skip_all, fields(packages))]
    async fn build(
        flox: Flox,
        mut env: ConcreteEnvironment,
        packages: Vec<String>,
        nixpkgs_url_select: Option<BaseCatalogUrlSelect>,
        system_override: Option<String>,
    ) -> Result<()> {
        match &env {
            ConcreteEnvironment::Path(_) => (),
            ConcreteEnvironment::Managed(managed) => {
                bail!(needs_project_files_error(managed, "build"))
            },
            ConcreteEnvironment::Remote(_) => {
                // guarded by DirEnvironmentSelect
                unreachable!("Cannot build from a remote environment")
            },
        };

        let base_dir = env.parent_path()?;
        let built_environments = env.build(&flox)?;

        let lockfile: Lockfile = env.lockfile(&flox)?.into();

        // Used for non building expressions and manifest builds
        prefetch_flake_ref(&COMMON_NIXPKGS_URL)?;

        let lockfile_manifest = lockfile.migrated_manifest()?;
        let (packages_to_build, expression_ref, expression_path_ref) = {
            // TODO: decouple from env
            let expression_parent_dir = env.dot_flox_path();
            let expression_path_ref = NixFlakeref::from_path(&expression_parent_dir)?;
            let packages_to_build =
                packages_to_build(&lockfile_manifest, &expression_path_ref, &packages)?;
            let expression_git_ref = check_git_tracking_for_expression_builds(
                &packages_to_build,
                &expression_parent_dir,
            )?;
            (
                packages_to_build,
                expression_git_ref
                    .clone()
                    .unwrap_or(expression_path_ref.clone()),
                expression_path_ref,
            )
        };

        let target_names = packages_to_build
            .iter()
            .map(|target| target.name())
            .collect::<Vec<_>>();

        let has_expression_build = packages_to_build
            .iter()
            .any(|target| target.kind().is_expression_build());
        let has_manifest_build = packages_to_build
            .iter()
            .any(|target| target.kind().is_manifest_build());
        subcommand_metric!(
            "build",
            "has_expression_build" = has_expression_build,
            "has_manifest_build" = has_manifest_build
        );

        // The catalog lock the NEF evals consume, created by the CLI: the
        // committed .flox/catalog.lock exactly as found, or a fresh
        // ephemeral lock living only for this invocation. Scanning is
        // scoped to the expressions being built — the scanner follows
        // imports, so their references are exactly what the evals look up —
        // except when a manifest build is among the targets, whose `${pkg}`
        // references can pull in any of the project's expressions, so all
        // of them are covered. A project without expression builds needs no
        // lock at all.
        let lock_rel_paths = if has_manifest_build {
            expression_rel_paths(
                &PackageTargets::new(&lockfile_manifest, &expression_path_ref)?.all(),
            )
        } else {
            expression_rel_paths(&packages_to_build)
        };

        disallow_unusable_base_url_select(nixpkgs_url_select.as_ref(), lock_rel_paths.is_empty())?;

        // A manifest build's `${pkg}` references can reach any of the
        // project's expressions, which is what `lock_rel_paths` already covers.
        let expression_nixpkgs_url = match &*lock_rel_paths {
            [] => None,
            _ => Some(base_nixpkgs_url_from_url_select(&flox, nixpkgs_url_select).await?),
        };
        let base_nixpkgs_url = expression_nixpkgs_url
            .as_ref()
            .map(|url| url.as_flake_ref())
            .transpose()?;

        if let (Some(expression_nixpkgs), Some(base_nixpkgs_url)) =
            (&expression_nixpkgs_url, &base_nixpkgs_url)
        {
            prefetch_expression_build_flake_ref(&packages_to_build, base_nixpkgs_url)?;
            report_expression_nixpkgs(expression_nixpkgs, &lockfile, has_manifest_build);
        }

        let catalog_lock = match &*lock_rel_paths {
            [] => None,
            lock_rel_paths => Some(
                BuildLockGuard::new_existing_or_ephemeral(
                    &flox.floxhub_client,
                    env.dot_flox_path(),
                    lock_rel_paths,
                )
                .await?,
            ),
        };

        let cache_path = env.cache_path()?;
        let builder = FloxBuildMk::new(
            &flox,
            &base_dir,
            &expression_ref,
            &built_environments,
            &cache_path,
        );
        let build_start = Instant::now();
        let results = builder.build(
            base_nixpkgs_url.as_ref(),
            &FLOX_INTERPRETER,
            &target_names,
            catalog_lock.as_ref().map(|lock| lock.path()),
            None,
            system_override,
        );
        let build_duration_ms = duration_to_ms(build_start.elapsed());

        // A build the user cancelled has no genuine outcome, so it must not be
        // recorded as a build failure — Ctrl-C sends SIGINT to the whole process
        // group, and we treat the sibling cancellation/termination-request
        // signals (SIGQUIT, SIGHUP, SIGTERM) the same way. A build the OOM killer
        // takes down (SIGKILL) or whose builder crashes (SIGSEGV/SIGABRT/…) is a
        // real failure and is still recorded. Only `BuildFailure` carries status.
        let cancelled = matches!(
            &results,
            Err(ManifestBuilderError::BuildFailure { status })
                if matches!(
                    status.signal().and_then(|s| Signal::try_from(s).ok()),
                    Some(Signal::SIGINT | Signal::SIGQUIT | Signal::SIGHUP | Signal::SIGTERM)
                )
        );
        if !cancelled {
            let mut payload = CliBuildPayload::new(has_expression_build, has_manifest_build)
                .with_duration_ms(build_duration_ms);
            if let Some(lockfile_hash) = lockfile.content_hash() {
                payload = payload.with_lockfile_hash(lockfile_hash);
            }
            let payload = match &results {
                Ok(_) => payload.with_outcome(Outcome::Success),
                Err(err) => payload
                    .with_outcome(Outcome::Failure)
                    .with_error_kind(err.into()),
            };
            if let Err(err) = EventsHub::global().record_event(EventKind::CliBuild(payload)) {
                debug!(error = %err, "Failed to record v2 event");
            }
        }

        let results = results?;

        let current_dir = env::current_dir()
            .context("could not get current directory")?
            .canonicalize()
            .context("could not canonicalize current directory")?;

        let links_to_print = results
            .iter()
            .map(|package| Self::format_result_links(package.result_links.keys(), &current_dir))
            .flatten_ok()
            .collect::<Result<Vec<_>, _>>()?;

        match links_to_print.as_slice() {
            // This case shouldn't occur with the current FloxBuildMk backend,
            // which either errors earlier if nothing will be built,
            // or produces at least one link.
            // Handle anyway for completeness and to avoid errors in case the above changes.
            [] => message::info("Completed build with no outputs"),
            [link] => message::created(format!("Built output: {link}")),
            links => message::created(formatdoc! {"
                Built outputs:
                {}",
                links.join(", ")
            }),
        }

        Ok(())
    }

    /// Parse a Nix installable into flake reference and attribute path
    /// Examples:
    /// - "hello" -> ("nixpkgs", "hello")
    /// - "nixpkgs#hello" -> ("nixpkgs", "hello")
    /// - "github:nixos/nixpkgs#hello" -> ("github:nixos/nixpkgs", "hello")
    fn parse_installable(installable: &str) -> Result<(String, String)> {
        if let Some((flake_ref, attr_path)) = installable.split_once('#') {
            Ok((flake_ref.to_string(), attr_path.to_string()))
        } else {
            // If no '#' is present, assume it's just an attribute path and use nixpkgs as default
            Ok(("nixpkgs".to_string(), installable.to_string()))
        }
    }

    /// Resolve the flake ref that `import_nixpkgs` passes to `nix eval`.
    ///
    /// Rules:
    /// * No flag → return the parsed flake ref from the installable (e.g.
    ///   `"nixpkgs"` for a bare attr path).
    /// * `--stability` or `--nixpkgs-url` flag present → require that the
    ///   installable does not carry an explicit non-nixpkgs flake ref, then
    ///   resolve via the catalog and convert to a `git+…?shallow=1` flake ref.
    /// * Explicit non-nixpkgs ref with a flag → hard error.
    async fn resolve_import_flake_ref(
        flox: &Flox,
        installable: &str,
        base_catalog_url_select: Option<BaseCatalogUrlSelect>,
    ) -> Result<String> {
        let (parsed_flake_ref, _) = Self::parse_installable(installable)?;

        if let Some(sel) = base_catalog_url_select {
            let is_explicit_non_nixpkgs = parsed_flake_ref != "nixpkgs";
            if is_explicit_non_nixpkgs {
                bail!(
                    "Cannot use --stability or --nixpkgs-url together with an explicit flake reference ('{parsed_flake_ref}'). Remove the flag or use a bare attribute path."
                );
            }
            let base_nixpkgs_url = base_nixpkgs_url_from_url_select(flox, Some(sel)).await?;
            Ok(base_nixpkgs_url.as_flake_ref()?.to_string())
        } else {
            Ok(parsed_flake_ref)
        }
    }

    #[instrument(name = "build::import-nixpkgs", skip_all)]
    async fn import_nixpkgs(
        flox: Flox,
        env: ConcreteEnvironment,
        installable: String,
        force: bool,
        base_catalog_url_select: Option<BaseCatalogUrlSelect>,
    ) -> Result<()> {
        match &env {
            ConcreteEnvironment::Path(_) => (),
            ConcreteEnvironment::Managed(managed) => {
                bail!(needs_project_files_error(managed, "import"))
            },
            ConcreteEnvironment::Remote(_) => {
                // guarded by DirEnvironmentSelect
                unreachable!("Cannot import from nixpkgs in a remote environment")
            },
        };

        // Parse the installable to get flake reference and attribute path
        let (_, attr_path) = Self::parse_installable(&installable)?;

        // Resolve the flake_ref to use for nix eval.
        let flake_ref =
            Self::resolve_import_flake_ref(&flox, &installable, base_catalog_url_select).await?;

        // Split package name by dots to create proper directory nesting
        let package_dir = {
            let mut pkgs_dir = nix_expression_dir(&env);
            pkgs_dir.extend(attr_path.split('.'));
            pkgs_dir
        };
        let package_file = package_dir.join("default.nix");

        // Create .flox/pkgs directory and any nested package directories if they don't exist
        std::fs::create_dir_all(&package_dir).context("Failed to create package directory")?;

        // Check if file already exists
        if package_file.exists() && !force {
            bail!(formatdoc! {"
                Package file already exists: {package_file}

                Use --force to overwrite the existing file.
                ", package_file = package_file.display()
            });
        }

        // Get package position using nix eval
        let mut cmd = nix::nix_base_command();
        cmd.args([
            "eval",
            "--raw",
            &format!("{}#{}.meta.position", flake_ref, attr_path),
        ]);

        debug!(cmd = %cmd.display(), "running nix eval command to get package position");
        let output = cmd.output().context("Failed to run nix eval command")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("nix eval command failed: {stderr}");
        }

        let position_output =
            String::from_utf8(output.stdout).context("nix eval command returned invalid UTF-8")?;

        // Split position by ':' to get file and line
        let (file, _line) = position_output
            .split_once(':')
            .ok_or_else(|| anyhow::anyhow!("Invalid position format: {}", position_output))?;

        // Read the package definition from the source file
        let package_content = std::fs::read(file)
            .with_context(|| format!("Failed to read package source file: {}", file))?;

        std::fs::write(&package_file, package_content).context("Failed to write package file")?;

        message::created(format!(
            "Imported package '{}' to {}",
            installable,
            package_file.display()
        ));

        Ok(())
    }

    #[instrument(name = "build::update-catalogs", skip_all)]
    async fn update_catalogs(flox: &Flox, mut env: ConcreteEnvironment) -> Result<()> {
        match &env {
            ConcreteEnvironment::Path(_) => (),
            ConcreteEnvironment::Managed(managed) => {
                bail!(needs_project_files_error(managed, "update catalogs"))
            },
            ConcreteEnvironment::Remote(_) => {
                // guarded by DirEnvironmentSelect
                unreachable!("Cannot update catalogs of a remote environment")
            },
        };

        let expression_ref = NixFlakeref::from_path(env.dot_flox_path())?;
        let lockfile: Lockfile = env.lockfile(flox)?.into();
        let manifest = lockfile.migrated_manifest()?;

        let rel_file_paths =
            expression_rel_paths(&PackageTargets::new(&manifest, &expression_ref)?.all());

        if rel_file_paths.is_empty() {
            message::plain(
                "No Nix expression builds found; only expression builds reference the catalog.",
            );
            return Ok(());
        }

        let lockfile_path = catalog_lockfile_path(env.dot_flox_path());
        let references = lock_project_catalog(
            &flox.floxhub_client,
            nix_expression_dir(&env),
            &rel_file_paths,
            &lockfile_path,
        )
        .await?;

        if references.is_empty() {
            message::created(formatdoc! {"
                No catalog references found; wrote an empty '.flox/catalog.lock'.
                Commit the file so every revision builds against the same inputs."});
        } else {
            message::created(formatdoc! {"
                Locked {count} catalog reference(s) to '.flox/catalog.lock'.
                Commit the file so every revision builds against the same inputs.",
                count = references.len(),
            });
        }
        Ok(())
    }

    /// If so, shorten symlink for a package it if in the current directory.
    ///
    /// current_dir should be canonicalized
    fn format_result_links(
        package_result_links: impl IntoIterator<Item = impl AsRef<Path>>,
        current_dir: impl AsRef<Path>,
    ) -> Result<Vec<String>> {
        package_result_links
            .into_iter()
            .map(|result_link| {
                let result_link = result_link.as_ref();
                let parent = result_link
                    .parent()
                    .expect("symlink must be in a directory");

                let parent = parent
                    .canonicalize()
                    .context("couldn't canonicalize parent of build symlink")?;

                if parent == current_dir.as_ref() {
                    Ok(format!(
                        "./{}",
                        result_link
                            .file_name()
                            .expect("symlink must have a file name")
                            .to_string_lossy()
                    ))
                } else {
                    Ok(result_link.display().to_string())
                }
            })
            .collect::<Result<Vec<_>>>()
    }
}

/// Name the nixpkgs an expression build resolved, or warn that the
/// environment's manifest builds disagree with it.
///
/// The revision moves with the catalog, so a build that breaks against an
/// unchanged working tree has nothing else to point at.
fn report_expression_nixpkgs(
    expression_nixpkgs: &BaseCatalogUrl,
    lockfile: &Lockfile,
    has_manifest_build: bool,
) {
    let diverged = if has_manifest_build {
        revisions_differing_from(&locked_nixpkgs_urls(lockfile), expression_nixpkgs)
    } else {
        Vec::new()
    };

    if diverged.is_empty() {
        message::plain(format!(
            "Nix expression builds use nixpkgs {}.",
            describe_nixpkgs(expression_nixpkgs)
        ));
        return;
    }

    message::warning(formatdoc! {"
        Manifest builds and Nix expression builds in this environment use
        different nixpkgs revisions, so packages built from one may not link
        against packages built from the other.
          manifest builds:        nixpkgs {manifest}
          Nix expression builds:  nixpkgs {expression}
        Run 'flox upgrade' to move this environment to the current revision.
        ",
        manifest = diverged.join(", "),
        expression = describe_nixpkgs(expression_nixpkgs),
    });
}

/// Compared by revision, not by url: the same revision reached through a
/// differently shaped url is the same package set.
fn revisions_differing_from(locked: &[BaseCatalogUrl], expression: &BaseCatalogUrl) -> Vec<String> {
    let expression_rev = expression.rev();
    let mut seen = Vec::new();
    let mut diverged = Vec::new();

    for url in locked {
        let differs = match (url.rev(), &expression_rev) {
            (Some(locked_rev), Some(expression_rev)) => &locked_rev != expression_rev,
            _ => url != expression,
        };
        if !differs {
            continue;
        }

        let key = url.rev().unwrap_or_else(|| url.to_string());
        if seen.contains(&key) {
            continue;
        }

        seen.push(key);
        diverged.push(describe_nixpkgs(url));
    }

    diverged
}

/// A nixpkgs revision short enough to compare by eye, or the whole url when
/// there is no revision to name.
pub(crate) fn describe_nixpkgs(url: &BaseCatalogUrl) -> String {
    match url.rev() {
        Some(rev) if rev.len() >= 7 => format!("rev {}", &rev[..7]),
        Some(rev) => format!("rev {rev}"),
        None => url.to_string(),
    }
}

/// Refuse a nixpkgs selection that cannot affect anything this invocation
/// builds, rather than ignoring it silently.
pub(crate) fn disallow_unusable_base_url_select(
    nixpkgs_url_select: Option<&BaseCatalogUrlSelect>,
    selects_nothing: bool,
) -> Result<()> {
    let Some(select) = nixpkgs_url_select else {
        return Ok(());
    };

    if !selects_nothing {
        return Ok(());
    }

    let flag = match select {
        BaseCatalogUrlSelect::NixpkgsUrl(_) => "--nixpkgs-url",
        BaseCatalogUrlSelect::Stability(_) => "--stability",
    };

    bail!(formatdoc! {"
        The '{flag}' option only applies to Nix expression builds, and this
        command builds none.
        A manifest build always uses the nixpkgs its environment is locked to.
        Omit '{flag}', or name a Nix expression build instead.
        "
    })
}

/// Determine the [BaseCatalogUrl] used for expression builds
/// using the following rules:
///
/// * If the command line arguments address a specific nixpkgs url
///   (i.e. `BaseCatalogUrlSelect::NixpkgsUrl` / `--nixpkgs-url <url>`)
///   this url is used as is.  The catalog service may require the url to already be
///   present in the catalog.  This is an advanced option and is hidden for that
///   reason.
/// * If the command line arguments address a stability
///   (i.e. `BaseCatalogUrlSelect::Stability` / `--stability <stability>`)
///   queries the nixpkgs url for the given stability from the catalog server
///   and uses the latest revision for that stability.
/// * If neither argument is provided, uses the latest nixpkgs url for the
///   default stability ("stable") from the catalog server.
///
/// The environment's own locked nixpkgs is never consulted, so the manifest
/// cannot pin, or stale, what an expression build resolves against.
pub(crate) async fn base_nixpkgs_url_from_url_select(
    flox: &Flox,
    nixpkgs_url_select: Option<BaseCatalogUrlSelect>,
) -> Result<BaseCatalogUrl, anyhow::Error> {
    let catalog = &flox.floxhub_client;
    let base_catalog_info_fut = catalog.get_base_catalog_info();

    let stability = match nixpkgs_url_select {
        Some(BaseCatalogUrlSelect::NixpkgsUrl(url)) => {
            return Ok(BaseCatalogUrl::from(url.as_str()));
        },
        Some(BaseCatalogUrlSelect::Stability(stability)) => Some(stability),
        None => None,
    };

    match base_catalog_url_for_stability_arg(stability.as_deref(), base_catalog_info_fut).await {
        Ok(url) => Ok(url),
        Err(err @ FloxhubClientError::StabilityError(_)) => Err(err.into()),
        Err(err) => Err(base_catalog_unreachable(err)),
    }
}

/// The error an offline or air-gapped user meets.
///
/// The cause is folded into the message rather than left as a source, because
/// `main` joins an uncategorized error's chain with ": " — which would append
/// the transport error to the advice sentence and bury it.
fn base_catalog_unreachable(err: impl std::fmt::Display) -> anyhow::Error {
    anyhow::anyhow!(formatdoc! {"
        Could not get information about the base catalog.
        {err}
        Pass '--nixpkgs-url <url>' to build against a nixpkgs revision
        directly, without consulting the catalog server.
        "
    })
}

/// Enforce the existence of a git repository when building nix expressions,
/// to avoid costly and potentially insecure copies to the nix store.
/// Additionally, ensure that the expression files are tracked by git,
/// so that they are guaranteed to be found by the build subsystem,
/// which filters any untracked sources
/// allowing us to provide cleaner messaging on the way.
pub(crate) fn check_git_tracking_for_expression_builds<'p>(
    packages_to_build: impl IntoIterator<Item = &'p PackageTarget>,
    expression_parent_dir: &CanonicalPath,
) -> Result<Option<NixFlakeref>> {
    let mut expression_builds = packages_to_build
        .into_iter()
        .filter(|target| target.kind().is_expression_build())
        .peekable();

    if expression_builds.peek().is_none() {
        return Ok(None);
    }

    let expression_builds: Vec<_> = expression_builds
        .map(|target| {
            let PackageTargetKind::ExpressionBuild(metadata) = target.kind() else {
                unreachable!("kind checked above");
            };
            (target.name(), metadata)
        })
        .collect();

    let expression_builds_formatted = expression_builds
        .iter()
        .map(|(name, _)| format!("  - {name}"))
        .join("\n");

    let git = match GitCommandProvider::discover(expression_parent_dir) {
        Err(err) => {
            trace!(%err, "git discovery error");

            bail!(formatdoc! {"
                Building nix expression build(s) requires git version control.
                Only git tracked files (including the expressions themselves) will be available during nix expression builds.

                Expression build(s):
                {expression_builds_formatted}
            "});
        },
        Ok(git) => git,
    };
    for (name, metadata) in expression_builds {
        let mut cmd = git.new_command();
        let file_path = expression_parent_dir
            .join("pkgs")
            .join(&metadata.rel_file_path);

        cmd.arg("ls-files").arg("--error-unmatch").arg(&file_path);
        cmd.stderr(Stdio::null());
        cmd.stdout(Stdio::null());

        let status = cmd.status()?;
        if !status.success() {
            bail!(formatdoc! {"
               The Nix expression for '{name}' does not appear to be tracked by git.
               Only git tracked files (including the expressions themselves) will be available during nix expression builds.

               Nix expression: '{name}' defined in '{file_path}'
               ", file_path = file_path.display()
            });
        }
    }

    let rel_project_path = expression_parent_dir
        .strip_prefix(git.path())
        .expect("git repository is common parent of all files contained");

    let expression_git_ref = NixFlakeref::from_git_with_dir(
        &Url::from_directory_path(git.path()).expect("path should be a valid unix path"),
        Some(rel_project_path),
    )?;

    Ok(Some(expression_git_ref))
}

/// Download the source tree denoted by a flake reference into the Nix store.
///
/// This is used to download the nixpkgs we depend on during a flox build
/// at a known time i.e. within the cli/rust context.
/// We do this to a) avoid silent delays during the actual build execution,
/// due to nixpkgs downloads, and b) provide better messaging
/// about what flox spends time on during the build.
#[instrument(skip_all, fields(%flakeref, progress = format!("Downloading Nix build tools from '{flakeref}'")))]
pub(crate) fn prefetch_flake_ref(flakeref: &Url) -> Result<(), PrefetchError> {
    let mut cmd = nix::nix_base_command();
    cmd.args(["flake", "prefetch", flakeref.as_str()]);

    debug!(cmd = %cmd.display(), "running prefetch command");
    let output = cmd.output().map_err(PrefetchError::CallNixFlakePrefetch)?;

    if !output.status.success() {
        return Err(PrefetchError::PrefetchFailed {
            flakeref: flakeref.clone(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }

    Ok(())
}

pub(crate) fn prefetch_expression_build_flake_ref<'p>(
    packages_to_build: impl IntoIterator<Item = &'p PackageTarget>,
    flakeref: &Url,
) -> Result<(), PrefetchError> {
    if packages_to_build
        .into_iter()
        .any(|p| p.kind().is_expression_build())
    {
        return prefetch_flake_ref(flakeref);
    }

    debug!("No expression build target, skipping prefetch of {flakeref}");
    Ok(())
}

#[derive(Debug, Error)]
pub(crate) enum PrefetchError {
    #[error("Failed to call 'nix flake prefetch'")]
    CallNixFlakePrefetch(#[source] std::io::Error),
    #[error(
        "Failed to download Nix build tools from '{flakeref}'\n\
        {stderr}"
    )]
    PrefetchFailed { flakeref: Url, stderr: String },
}

/// The expression files of `targets`, relative to the project's expression
/// directory — the inputs a catalog lock is scanned from. Manifest builds
/// contribute none of their own.
pub(crate) fn expression_rel_paths(targets: &[PackageTarget]) -> Vec<PathBuf> {
    targets
        .iter()
        .filter_map(|target| match target.kind() {
            PackageTargetKind::ExpressionBuild(expression) => {
                Some(expression.rel_file_path.clone())
            },
            PackageTargetKind::ManifestBuild { .. } => None,
        })
        .collect()
}

pub(crate) fn packages_to_build<'o>(
    manifest: &'o Manifest<MigratedTypedOnly>,
    expression_ref: &'o NixFlakeref,
    packages: &[impl AsRef<str>],
) -> Result<Vec<PackageTarget>> {
    let available_targets = PackageTargets::new(manifest, expression_ref)?;

    if available_targets.is_empty() {
        bail!(formatdoc! {"
            No packages found to build.

            Add a build by modifying the '[build]' section of the manifest with 'flox edit'
            or add expression files in '{expression_ref}'.
            ", expression_ref = expression_ref.as_url()
        });
    }

    let selected = if !packages.is_empty() {
        available_targets.select(packages)?
    } else {
        available_targets.all()
    };

    Ok(selected)
}

#[cfg(test)]
mod test {
    use std::fs::File;

    use flox_rust_sdk::flox::test_helpers::flox_instance;
    use flox_rust_sdk::models::environment::path_environment::test_helpers::new_path_environment;
    use flox_rust_sdk::providers::build::ExpressionBuildMetadata;
    use flox_rust_sdk::providers::build::test_helpers::prepare_nix_expressions_in;
    use flox_rust_sdk::providers::nix::test_helpers::known_store_path;
    use floxhub_client::{BaseCatalogInfo, BaseCatalogUrl};
    use tempfile::tempdir_in;

    use super::*;

    /// Test that check_and_display_symlink shortens the symlink when in the
    /// current directory,
    #[test]
    fn symlink_gets_shortened_when_in_current_dir() {
        let (flox, _temp_dir) = flox_instance();
        let dot_flox_parent_path = tempdir_in(&flox.temp_dir)
            .unwrap()
            .keep()
            .canonicalize()
            .unwrap();
        let package = "foo";
        let symlink = dot_flox_parent_path.join(format!("result-{package}"));
        // We just want some random symlink possibly into the /nix/store
        std::os::unix::fs::symlink(known_store_path(), &symlink).unwrap();
        let displayed =
            Build::format_result_links([&symlink], dot_flox_parent_path.canonicalize().unwrap())
                .unwrap();
        assert_eq!(displayed, vec![format!("./result-{package}")]);

        let displayed = Build::format_result_links([&symlink], &flox.temp_dir).unwrap();
        assert_eq!(displayed, vec![symlink.to_string_lossy()]);
    }

    /// Test that conflicting build names are detected if builds are defined via the manifest and nix expressions.
    #[test]
    fn conflicting_build_names() {
        let pname = "conflict".to_string();

        let (flox, tempdir) = flox_instance();

        // Create a manifest (may be empty)
        let manifest = formatdoc! {r#"
            version = 1

            [build]
            conflict.command = ""
        "#};

        let mut env = new_path_environment(&flox, &manifest);

        // Create expressions
        let expressions_ref =
            prepare_nix_expressions_in(&tempdir, &[(&[&pname], &formatdoc! {r#"
                {{runCommand}}: runCommand "{pname}" {{}} ""
            "#})]);

        let lockfile: Lockfile = env.lockfile(&flox).unwrap().into();
        let lockfile_manifest = lockfile.migrated_manifest().unwrap();
        let result = packages_to_build(&lockfile_manifest, &expressions_ref, &Vec::<String>::new());
        assert!(result.is_err());
    }

    /// A selection is refused only when this invocation reaches no Nix
    /// expression build, because that is the only case where it can have no
    /// effect. The refusal names the flag the user actually passed.
    #[test]
    fn base_url_select_refused_only_when_it_selects_nothing() {
        let stability = BaseCatalogUrlSelect::Stability("stable".to_string());

        let err = disallow_unusable_base_url_select(Some(&stability), true)
            .expect_err("a selection that reaches no expression build is refused");
        assert!(
            err.to_string().contains("'--stability' option"),
            "unexpected error: {err}"
        );

        // Reaching any expression build makes the selection meaningful, even
        // when a manifest build was the package named on the command line.
        disallow_unusable_base_url_select(Some(&stability), false)
            .expect("a selection that reaches an expression build is allowed");
    }

    /// The refusal names `--nixpkgs-url` when that is what was passed. It used
    /// to say `--stability` either way, which sent users of the other flag
    /// looking for a flag they had not used.
    #[test]
    fn base_url_select_refusal_names_the_flag_that_was_passed() {
        let url =
            BaseCatalogUrlSelect::NixpkgsUrl("https://github.com/NixOS/nixpkgs".parse().unwrap());

        let err = disallow_unusable_base_url_select(Some(&url), true)
            .expect_err("a selection that reaches no expression build is refused");

        let message = err.to_string();
        assert!(
            message.contains("'--nixpkgs-url' option"),
            "unexpected: {message}"
        );
        assert!(!message.contains("--stability"), "unexpected: {message}");
    }

    /// A stability the catalog does not carry is not an unreachable catalog:
    /// the server answered, and its answer lists what it does carry. Wrapping
    /// it in the unreachable-catalog message would open with a false claim and
    /// bury that list.
    #[tokio::test]
    async fn unknown_stability_keeps_its_own_error() {
        use floxhub_client::FloxhubClient;
        use floxhub_client::client::test_helpers::client_config;
        use httpmock::MockServer;

        let (mut flox, _temp_dir) = flox_instance();

        let server = MockServer::start_async().await;
        server.mock(|when, then| {
            when.path("/api/v1/catalog/info/base-catalog");
            then.status(200)
                .json_body(serde_json::to_value(BaseCatalogInfo::new_mock()).unwrap());
        });
        flox.floxhub_client =
            FloxhubClient::new(client_config(server.base_url().as_str())).unwrap();

        let err = base_nixpkgs_url_from_url_select(
            &flox,
            Some(BaseCatalogUrlSelect::Stability("typo".to_string())),
        )
        .await
        .expect_err("a stability the catalog does not carry is an error");

        let message = err.to_string();
        assert!(
            message.contains("Stability 'typo' does not exist"),
            "unexpected: {message}"
        );
        assert!(
            message.contains("Available stabilities are"),
            "the available stabilities are the actionable part: {message}"
        );
        assert!(
            !message.contains("Could not get information about the base catalog"),
            "the catalog was reached: {message}"
        );
    }

    /// The same revision reached through a differently shaped url is the same
    /// package set, so it is not a divergence — and a revision is named once
    /// however many packages are locked to it.
    #[test]
    fn divergence_is_decided_and_deduped_by_revision() {
        let expression = BaseCatalogUrl::from("https://github.com/flox/nixpkgs?rev=abc1234");

        let same_rev_other_shape =
            BaseCatalogUrl::from("https://example.invalid/nixpkgs?rev=abc1234&extra=1");
        assert_eq!(
            revisions_differing_from(&[same_rev_other_shape], &expression),
            Vec::<String>::new()
        );

        let other = BaseCatalogUrl::from("https://github.com/flox/nixpkgs?rev=def5678");
        let other_again = BaseCatalogUrl::from("https://github.com/flox/nixpkgs?rev=def5678");
        assert_eq!(
            revisions_differing_from(&[other, other_again], &expression),
            vec!["rev def5678".to_string()]
        );
    }

    #[tokio::test]
    async fn explicit_stability_selects_that_stabilitys_latest_page() {
        let mock_base_catalog_info = BaseCatalogInfo::new_mock();

        let actual = base_catalog_url_for_stability_arg(Some("not-default"), async {
            Ok(mock_base_catalog_info.clone())
        })
        .await
        .unwrap();

        let expected_url = mock_base_catalog_info
            .url_for_latest_page_with_stability("not-default")
            .unwrap();

        assert_eq!(actual, expected_url);
    }

    #[tokio::test]
    async fn no_stability_selects_the_default_stabilitys_latest_page() {
        let mock_base_catalog_info = BaseCatalogInfo::new_mock();

        let actual =
            base_catalog_url_for_stability_arg(None, async { Ok(mock_base_catalog_info.clone()) })
                .await
                .unwrap();

        let expected_url = mock_base_catalog_info
            .url_for_latest_page_with_default_stability()
            .unwrap();
        assert_eq!(actual, expected_url);
    }

    #[test]
    fn expression_builds_require_git_repo() {
        let base_dir = tempfile::tempdir().unwrap();
        let rel_file_path = Path::new("./expression.nix");
        {
            let pkgs_dir = base_dir.path().join("pkgs");
            std::fs::create_dir(&pkgs_dir).unwrap();
            let abs_file_path = pkgs_dir.join(rel_file_path);
            File::create(&abs_file_path).unwrap();
        }

        let packages = vec![PackageTarget::new_unchecked(
            "expression",
            PackageTargetKind::ExpressionBuild(ExpressionBuildMetadata {
                rel_file_path: rel_file_path.to_path_buf(),
            }),
        )];

        // fail without a git repository containing the expression dir
        let result = check_git_tracking_for_expression_builds(
            &packages,
            &CanonicalPath::new(base_dir.path()).unwrap(),
        );
        assert!(result.is_err());

        // fail if the expression isn't tracked
        let git = GitCommandProvider::init(base_dir.path(), false).unwrap();
        let result = check_git_tracking_for_expression_builds(
            &packages,
            &CanonicalPath::new(base_dir.path()).unwrap(),
        );
        assert!(result.is_err(), "expression needs to be tracked");

        git.add(&[&Path::new("pkgs").join(rel_file_path)]).unwrap();
        let result = check_git_tracking_for_expression_builds(
            &packages,
            &CanonicalPath::new(base_dir.path()).unwrap(),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn manifest_builds_do_not_require_git_repo() {
        let packages = vec![PackageTarget::new_unchecked(
            "manifest",
            PackageTargetKind::ManifestBuild { sandbox: None },
        )];
        let base_dir = tempfile::tempdir().unwrap();

        let result = check_git_tracking_for_expression_builds(
            &packages,
            &CanonicalPath::new(base_dir.path()).unwrap(),
        );
        assert!(result.is_ok());
    }

    #[tokio::test]
    #[serial_test::serial(import_nixpkgs)]
    async fn import_nixpkgs_creates_package_file() {
        let (flox, _temp_dir) = flox_instance();

        let manifest = formatdoc! {r#"
            version = 1
        "#};

        let env = new_path_environment(&flox, &manifest);

        let package_name = "hello";

        // Get the actual parent path from the environment
        let actual_base_dir = env.parent_path().unwrap();
        let package_file = actual_base_dir
            .join(".flox")
            .join("pkgs")
            .join(package_name)
            .join("default.nix");

        // Ensure the package file doesn't exist initially
        assert!(!package_file.exists());

        // Import the package
        Build::import_nixpkgs(
            flox,
            ConcreteEnvironment::Path(env),
            package_name.to_string(),
            false,
            None,
        )
        .await
        .unwrap();

        // Verify the package file was created
        assert!(package_file.exists());
        assert!(package_file.is_file());

        // Verify the file contains expected content
        let content = std::fs::read_to_string(&package_file).unwrap();
        assert!(content.contains("hello"));
        assert!(content.contains("stdenv.mkDerivation"));
    }

    #[tokio::test]
    #[serial_test::serial(import_nixpkgs)]
    async fn import_nixpkgs_creates_pkgs_directory() {
        let (flox, _temp_dir) = flox_instance();

        let manifest = formatdoc! {r#"
            version = 1
        "#};

        let env = new_path_environment(&flox, &manifest);
        let actual_base_dir = env.parent_path().unwrap();
        let pkgs_dir = actual_base_dir.join(".flox").join("pkgs");

        // Ensure the pkgs directory doesn't exist initially
        assert!(!pkgs_dir.exists());

        // Import a package
        Build::import_nixpkgs(
            flox,
            ConcreteEnvironment::Path(env),
            "hello".to_string(),
            false,
            None,
        )
        .await
        .unwrap();

        // Verify the pkgs directory was created
        assert!(pkgs_dir.exists());
        assert!(pkgs_dir.is_dir());
    }

    #[tokio::test]
    async fn import_nixpkgs_fails_when_file_exists_without_force() {
        let (flox, _temp_dir) = flox_instance();

        let manifest = formatdoc! {r#"
            version = 1
        "#};

        let env = new_path_environment(&flox, &manifest);
        let actual_base_dir = env.parent_path().unwrap();
        let package_name = "hello";
        let pkgs_dir = actual_base_dir.join(".flox").join("pkgs");
        let package_file = pkgs_dir.join(package_name).join("default.nix");

        // Create the .flox/pkgs directory and a dummy file
        std::fs::create_dir_all(package_file.parent().unwrap()).unwrap();
        std::fs::write(&package_file, "dummy content").unwrap();

        // Verify the file exists
        assert!(package_file.exists());

        // Try to import the same package without --force (should fail)
        let result = Build::import_nixpkgs(
            flox,
            ConcreteEnvironment::Path(env),
            package_name.to_string(),
            false,
            None,
        )
        .await;
        assert!(result.is_err());

        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("Package file already exists"));
        assert!(error_msg.contains("Use --force to overwrite"));
    }

    #[tokio::test]
    #[serial_test::serial(import_nixpkgs)]
    async fn import_nixpkgs_overwrites_with_force_flag() {
        let (flox, _temp_dir) = flox_instance();

        let manifest = formatdoc! {r#"
            version = 1
        "#};

        let env = new_path_environment(&flox, &manifest);
        let actual_base_dir = env.parent_path().unwrap();
        let package_name = "hello";
        let pkgs_dir = actual_base_dir.join(".flox").join("pkgs");
        let package_file = pkgs_dir.join(package_name).join("default.nix");

        // Create the .flox/pkgs directory and a dummy file
        std::fs::create_dir_all(package_file.parent().unwrap()).unwrap();
        std::fs::write(&package_file, "dummy content").unwrap();

        // Get the original file modification time
        let original_metadata = std::fs::metadata(&package_file).unwrap();
        let original_modified = original_metadata.modified().unwrap();

        // Wait a bit to ensure different modification time
        std::thread::sleep(std::time::Duration::from_millis(10));

        // Import the same package with --force (should succeed and overwrite)
        Build::import_nixpkgs(
            flox,
            ConcreteEnvironment::Path(env),
            package_name.to_string(),
            true,
            None,
        )
        .await
        .unwrap();

        // Verify the file was overwritten (different modification time)
        let new_metadata = std::fs::metadata(&package_file).unwrap();
        let new_modified = new_metadata.modified().unwrap();
        assert!(new_modified > original_modified);

        // Verify the file contains the actual package content, not dummy content
        let content = std::fs::read_to_string(&package_file).unwrap();
        assert!(content.contains("stdenv.mkDerivation"));
        assert!(!content.contains("dummy content"));
    }

    // Note: Testing managed environment failure is complex as it requires
    // creating a proper ManagedEnvironment which needs FloxHub integration.
    // This test is skipped for now but the functionality is tested in the
    // match statement in the import_nixpkgs function.

    #[tokio::test]
    #[serial_test::serial(import_nixpkgs)]
    async fn import_nixpkgs_handles_different_packages() {
        let packages = vec!["hello", "cowsay", "git"];

        for package_name in packages {
            let (flox, _temp_dir) = flox_instance();

            let manifest = formatdoc! {r#"
                version = 1
            "#};

            let env = new_path_environment(&flox, &manifest);
            let actual_base_dir = env.parent_path().unwrap();
            let package_file = actual_base_dir
                .join(".flox")
                .join("pkgs")
                .join(package_name)
                .join("default.nix");

            // Import the package
            let result = Build::import_nixpkgs(
                flox,
                ConcreteEnvironment::Path(env),
                package_name.to_string(),
                false,
                None,
            )
            .await;
            result.unwrap_or_else(|err| panic!("failed to import '{package_name}': {err:?}"));

            // Verify the package file was created
            assert!(
                package_file.exists(),
                "Package file not created for: {}",
                package_name
            );

            // Verify the file contains expected content
            let content = std::fs::read_to_string(&package_file).unwrap();
            assert!(
                content.contains(package_name),
                "Package file doesn't contain package name: {}",
                package_name
            );
            assert!(
                content.contains("stdenv.mkDerivation"),
                "Package file doesn't contain stdenv.mkDerivation for: {}",
                package_name
            );
        }
    }

    /// Passing --stability or --nixpkgs-url together with an explicit non-nixpkgs
    /// flake ref (e.g. github:nixos/nixpkgs#hello) must produce a clear error
    /// before any nix or catalog calls are made.
    #[tokio::test]
    async fn import_nixpkgs_conflict_errors_when_installable_has_explicit_flake_ref() {
        let (flox, _temp_dir) = flox_instance();

        let manifest = formatdoc! {r#"
            version = 1
        "#};

        let env = new_path_environment(&flox, &manifest);

        // github:nixos/nixpkgs#hello carries an explicit non-nixpkgs flake ref
        let result = Build::import_nixpkgs(
            flox,
            ConcreteEnvironment::Path(env),
            "github:nixos/nixpkgs#hello".to_string(),
            false,
            Some(BaseCatalogUrlSelect::NixpkgsUrl(
                "https://github.com/NixOS/nixpkgs".parse().unwrap(),
            )),
        )
        .await;

        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(
            error_msg.contains("Cannot use --stability or --nixpkgs-url"),
            "Expected conflict error, got: {error_msg}"
        );
        assert!(
            error_msg.contains("github:nixos/nixpkgs"),
            "Error should mention the conflicting flake ref, got: {error_msg}"
        );
    }

    /// Passing --stability with the literal "nixpkgs" flake prefix is allowed —
    /// the flag overrides which nixpkgs revision is used, which is its purpose.
    /// This test verifies no conflict error is raised; it stops before the catalog
    /// network call because the test environment has no live catalog.
    #[test]
    fn import_nixpkgs_allows_bare_nixpkgs_prefix_with_flag() {
        // Verify that parse_installable treats "nixpkgs#hello" as flake_ref="nixpkgs"
        let (flake_ref, attr_path) = Build::parse_installable("nixpkgs#hello").unwrap();
        assert_eq!(flake_ref, "nixpkgs");
        assert_eq!(attr_path, "hello");

        // And a bare attribute path should also produce flake_ref="nixpkgs"
        let (flake_ref_bare, _) = Build::parse_installable("hello").unwrap();
        assert_eq!(flake_ref_bare, "nixpkgs");

        // The conflict check should NOT fire for flake_ref == "nixpkgs":
        // is_explicit_non_nixpkgs = ("nixpkgs" != "nixpkgs") = false
        let is_explicit_non_nixpkgs = flake_ref != "nixpkgs";
        assert!(
            !is_explicit_non_nixpkgs,
            "nixpkgs# prefix must be treated as no explicit override"
        );
    }

    // --- Test B: base_nixpkgs_url_from_url_select wrapper via httpmock server ---

    /// `Stability` variant: serving `BaseCatalogInfo::new_mock()` from an
    /// httpmock server and calling with
    /// `BaseCatalogUrlSelect::Stability("not-default")` returns the URL for
    /// the first page that carries "not-default" in the fixture.
    #[tokio::test]
    async fn base_nixpkgs_url_from_url_select_stability_returns_catalog_url() {
        use floxhub_client::FloxhubClient;
        use floxhub_client::client::test_helpers::client_config;
        use httpmock::MockServer;

        let (mut flox, _temp_dir) = flox_instance();

        let server = MockServer::start_async().await;
        server.mock(|when, then| {
            when.path("/api/v1/catalog/info/base-catalog");
            then.status(200)
                .json_body(serde_json::to_value(BaseCatalogInfo::new_mock()).unwrap());
        });
        flox.floxhub_client =
            FloxhubClient::new(client_config(server.base_url().as_str())).unwrap();

        let result = base_nixpkgs_url_from_url_select(
            &flox,
            Some(BaseCatalogUrlSelect::Stability("not-default".to_string())),
        )
        .await
        .unwrap();

        // The mock fixture has base_url="https://mock.flox.dev" and page0 rev="",
        // so the selected URL is "https://mock.flox.dev?rev=".
        let expected = BaseCatalogInfo::new_mock()
            .url_for_latest_page_with_stability("not-default")
            .expect("fixture must have not-default stability");

        assert_eq!(result, expected);
    }

    /// `NixpkgsUrl` pass-through: passing a raw URL returns it as a
    /// `BaseCatalogUrl` without any catalog call, and `.as_flake_ref()` adds
    /// the `git+` prefix and `?shallow=1` query parameter.
    #[tokio::test]
    async fn base_nixpkgs_url_from_url_select_nixpkgs_url_passthrough() {
        let (flox, _temp_dir) = flox_instance();

        // No mock seeded — a catalog call here would panic.
        let raw_url: url::Url = "https://github.com/NixOS/nixpkgs".parse().unwrap();
        let result = base_nixpkgs_url_from_url_select(
            &flox,
            Some(BaseCatalogUrlSelect::NixpkgsUrl(raw_url.clone())),
        )
        .await
        .unwrap();

        let expected = BaseCatalogUrl::from(raw_url.as_str());
        assert_eq!(result, expected, "URL must be passed through unchanged");

        // Verify that as_flake_ref() produces the expected git+…?shallow=1 form.
        let flake_ref = result.as_flake_ref().expect("should convert to flake ref");
        assert_eq!(
            flake_ref.as_str(),
            "git+https://github.com/NixOS/nixpkgs?shallow=1"
        );
    }

    // --- Test C: resolve_import_flake_ref helper tests ---

    /// Flag + bare attribute path: the catalog is queried, the result is
    /// converted to a `git+…?rev=…&shallow=1` flake ref string.
    #[tokio::test]
    async fn resolve_import_flake_ref_stability_flag_bare_attr_path() {
        use floxhub_client::FloxhubClient;
        use floxhub_client::client::test_helpers::client_config;
        use httpmock::MockServer;

        let (mut flox, _temp_dir) = flox_instance();

        let server = MockServer::start_async().await;
        server.mock(|when, then| {
            when.path("/api/v1/catalog/info/base-catalog");
            then.status(200)
                .json_body(serde_json::to_value(BaseCatalogInfo::new_mock()).unwrap());
        });
        flox.floxhub_client =
            FloxhubClient::new(client_config(server.base_url().as_str())).unwrap();

        let result = Build::resolve_import_flake_ref(
            &flox,
            "hello",
            Some(BaseCatalogUrlSelect::Stability("not-default".to_string())),
        )
        .await
        .unwrap();

        // Expected: the catalog URL for "not-default" converted to a flake ref.
        let base_url = BaseCatalogInfo::new_mock()
            .url_for_latest_page_with_stability("not-default")
            .expect("fixture must have not-default stability");
        let expected = base_url
            .as_flake_ref()
            .expect("should convert to flake ref")
            .to_string();

        assert_eq!(result, expected);
        assert!(
            result.starts_with("git+"),
            "flake ref must start with git+, got: {result}"
        );
        assert!(
            result.contains("shallow=1"),
            "flake ref must contain shallow=1, got: {result}"
        );
    }

    /// Flag + `nixpkgs#hello` prefix: treated the same as a bare attr path
    /// (no conflict error), catalog is queried.
    #[tokio::test]
    async fn resolve_import_flake_ref_stability_flag_nixpkgs_prefixed_attr() {
        use floxhub_client::FloxhubClient;
        use floxhub_client::client::test_helpers::client_config;
        use httpmock::MockServer;

        let (mut flox, _temp_dir) = flox_instance();

        let server = MockServer::start_async().await;
        server.mock(|when, then| {
            when.path("/api/v1/catalog/info/base-catalog");
            then.status(200)
                .json_body(serde_json::to_value(BaseCatalogInfo::new_mock()).unwrap());
        });
        flox.floxhub_client =
            FloxhubClient::new(client_config(server.base_url().as_str())).unwrap();

        let result = Build::resolve_import_flake_ref(
            &flox,
            "nixpkgs#hello",
            Some(BaseCatalogUrlSelect::Stability("not-default".to_string())),
        )
        .await
        .unwrap();

        // Same expected URL as the bare-attr-path case.
        let base_url = BaseCatalogInfo::new_mock()
            .url_for_latest_page_with_stability("not-default")
            .expect("fixture must have not-default stability");
        let expected = base_url
            .as_flake_ref()
            .expect("should convert to flake ref")
            .to_string();

        assert_eq!(result, expected);
    }

    /// Flag + an explicit non-nixpkgs flake ref must return the conflict error.
    #[tokio::test]
    async fn resolve_import_flake_ref_conflict_error_for_explicit_non_nixpkgs_ref() {
        let (flox, _temp_dir) = flox_instance();

        // No mock seeded — the conflict bail must fire before any catalog call.
        let result = Build::resolve_import_flake_ref(
            &flox,
            "github:nixos/nixpkgs#hello",
            Some(BaseCatalogUrlSelect::NixpkgsUrl(
                "https://github.com/NixOS/nixpkgs".parse().unwrap(),
            )),
        )
        .await;

        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("Cannot use --stability or --nixpkgs-url"),
            "expected conflict error, got: {msg}"
        );
    }

    /// No flag + bare attr path: returns the literal string `"nixpkgs"` with
    /// no catalog call.
    #[tokio::test]
    async fn resolve_import_flake_ref_no_flag_returns_nixpkgs_literal() {
        let (flox, _temp_dir) = flox_instance();

        // No mock seeded — no catalog call should occur.
        let result = Build::resolve_import_flake_ref(&flox, "hello", None)
            .await
            .unwrap();

        assert_eq!(result, "nixpkgs");
    }
}
