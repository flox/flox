//! Namespace-only global package operations over one ordinary environment.
mod inventory;
mod publication;
mod shell;

use std::fmt::Display;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::{env, fs};

use anyhow::Result;
use bpaf::Bpaf;
use flox_core::data::CanonicalPath;
use flox_manifest::Manifest;
use flox_manifest::interfaces::{AsLatestSchema, AsWritableManifest, PackageLookup, WriteManifest};
use flox_manifest::lockfile::Lockfile;
use flox_manifest::parsed::common::KnownSchemaVersion;
use flox_manifest::raw::PackageToInstall;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::fetcher::IncludeFetcher;
use flox_rust_sdk::models::environment::path_environment::PathEnvironment;
use flox_rust_sdk::models::environment::uninstall::UninstallSpec;
use flox_rust_sdk::models::environment::{
    CoreEnvironment,
    DotFlox,
    Environment,
    EnvironmentPointer,
    PathPointer,
    RenderedEnvironmentLinks,
};
use serde::Serialize;
use shell_gen::Shell;
use tempfile::TempDir;
use tracing::instrument;

use self::inventory::Snapshot;
use self::publication::{PublicationReport, State};
use super::install::{PkgWithIdOption, pkg_with_id_option};
use crate::Exit;
use crate::utils::message;

pub type GlobalResult<T> = std::result::Result<T, GlobalError>;

#[derive(Clone, Debug, Serialize, thiserror::Error)]
#[error("{message}")]
pub struct GlobalError {
    pub category: &'static str,
    pub message: String,
    pub path: Option<PathBuf>,
}

impl GlobalError {
    pub fn new(category: &'static str, message: impl Display, path: Option<PathBuf>) -> Self {
        Self {
            category,
            message: message.to_string(),
            path,
        }
    }

    pub fn at(category: &'static str, path: &Path, message: impl Display) -> Self {
        Self::new(category, message, Some(path.to_owned()))
    }
}

#[derive(Clone, Debug)]
pub struct GlobalPaths {
    pub data_dir: PathBuf,
}

impl GlobalPaths {
    pub fn environment_dir(&self) -> PathBuf {
        self.data_dir.join("environments/global")
    }

    pub fn manifest_dir(&self) -> PathBuf {
        self.environment_dir().join(".flox/env")
    }

    pub fn publication_dir(&self) -> PathBuf {
        self.data_dir.join("global-publication")
    }

    pub fn state_file(&self) -> PathBuf {
        self.publication_dir().join("state.toml")
    }
}

#[derive(Clone, Debug, Bpaf)]
pub struct Global {
    /// Print a versioned JSON result, including errors
    #[bpaf(long)]
    json: bool,
    #[bpaf(external(global_command))]
    command: GlobalCommand,
}

#[derive(Clone, Debug, Bpaf)]
enum GlobalCommand {
    /// Install packages into the ordinary global environment
    #[bpaf(command)]
    Install {
        #[bpaf(external(pkg_with_id_option), many)]
        id: Vec<PkgWithIdOption>,
        /// Preview package and publication changes without changing live state
        #[bpaf(long)]
        dry_run: bool,
        #[bpaf(positional("PACKAGE"))]
        packages: Vec<String>,
    },
    /// List installed global packages or public commands
    #[bpaf(command)]
    List {
        /// Show commands and their observed publication status
        #[bpaf(long)]
        commands: bool,
    },
    /// Upgrade global packages; no targets upgrades all packages
    #[bpaf(command)]
    Upgrade {
        #[bpaf(long)]
        dry_run: bool,
        /// Upgrade all packages (also the default when no targets are given)
        #[bpaf(long)]
        all: bool,
        #[bpaf(positional("PACKAGE_OR_GROUP"))]
        targets: Vec<String>,
    },
    /// Uninstall package IDs and remove only verified owned command links
    #[bpaf(command)]
    Uninstall {
        #[bpaf(long)]
        dry_run: bool,
        #[bpaf(positional("PACKAGE"), some("Specify at least one package ID"))]
        targets: Vec<String>,
    },
    /// Reconcile current ordinary output without resolving, fetching, or building
    #[bpaf(command("retry-publication"))]
    RetryPublication {
        #[bpaf(long)]
        dry_run: bool,
    },
    /// Print the global environment or public command directory
    #[bpaf(command)]
    Dir {
        #[bpaf(long)]
        bin: bool,
        #[bpaf(long)]
        sbin: bool,
    },
    /// Compare the global provider with the executable selected by current PATH
    #[bpaf(command)]
    Which {
        #[bpaf(positional("COMMAND"))]
        command: String,
    },
    /// Explicitly configure PATH for global commands
    #[bpaf(command("update-shell"))]
    UpdateShell {
        /// Shell to configure: bash, zsh, fish, or tcsh
        #[bpaf(long, argument("SHELL"), optional)]
        shell: Option<Shell>,
        /// Print the PATH script for the selected shell without editing files
        #[bpaf(long)]
        print_script: bool,
    },
}

impl Global {
    pub fn subcommand_name(&self) -> &'static str {
        match self.command {
            GlobalCommand::Install { .. } => "global::install",
            GlobalCommand::List { .. } => "global::list",
            GlobalCommand::Upgrade { .. } => "global::upgrade",
            GlobalCommand::Uninstall { .. } => "global::uninstall",
            GlobalCommand::RetryPublication { .. } => "global::retry-publication",
            GlobalCommand::Dir { .. } => "global::dir",
            GlobalCommand::Which { .. } => "global::which",
            GlobalCommand::UpdateShell { .. } => "global::update-shell",
        }
    }

    #[instrument(name = "global", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        let paths = GlobalPaths {
            data_dir: flox.data_dir.clone(),
        };
        let operation = self.subcommand_name().trim_start_matches("global::");
        let mut report = Report::new(operation, &paths);
        let result = self.execute(&flox, &paths, &mut report);
        if let Err(error) = result {
            report.errors.push(error);
        }
        let failed = !report.errors.is_empty()
            || report
                .publication
                .as_ref()
                .is_some_and(|p| p.status != "complete");
        if self.json {
            println!("{}", serde_json::to_string(&report)?);
        } else {
            self.render(&report);
        }
        if failed {
            return Err(Exit(1).into());
        }
        Ok(())
    }

    fn execute(&self, flox: &Flox, paths: &GlobalPaths, report: &mut Report) -> GlobalResult<()> {
        if paths.data_dir.to_str().is_none() {
            return Err(GlobalError::new(
                "unsupported_path",
                "The Flox data directory cannot be represented as JSON or TOML",
                None,
            ));
        }
        match &self.command {
            GlobalCommand::Dir { bin, sbin } => {
                if *bin && *sbin {
                    return Err(GlobalError::new(
                        "usage",
                        "Choose either --bin or --sbin",
                        None,
                    ));
                }
                report.directory = Some(if *bin {
                    paths.data_dir.join("bin")
                } else if *sbin {
                    paths.data_dir.join("sbin")
                } else {
                    paths.environment_dir()
                });
            },
            GlobalCommand::UpdateShell {
                shell,
                print_script,
            } => {
                let shell = shell::setup(paths, *shell, *print_script)?;
                report.errors.extend(shell.errors.clone());
                report.shell = Some(shell);
            },
            GlobalCommand::List { .. } | GlobalCommand::Which { .. } => {
                let mut snapshot = inventory::read(paths, &flox.system)?;
                inventory::observe_publication(paths, &mut snapshot);
                report.errors.extend(snapshot.errors.clone());
                if let GlobalCommand::Which { command } = &self.command {
                    publication::CommandKey::new(publication::Directory::Bin, command.clone())?;
                    report.selected_by_path = inventory::path_lookup(command);
                    snapshot.commands.retain(|entry| entry.name == *command);
                }
                report.after = Some(snapshot);
            },
            GlobalCommand::RetryPublication { dry_run } => {
                report.dry_run = *dry_run;
                report.publication = Some(reconcile(flox, paths, *dry_run));
                report.after = Some(observe(flox, paths));
                report.errors.extend(
                    report
                        .after
                        .as_ref()
                        .expect("observed above")
                        .errors
                        .clone(),
                );
            },
            _ => self.mutate(flox, paths, report)?,
        }
        Ok(())
    }

    fn mutate(&self, flox: &Flox, paths: &GlobalPaths, report: &mut Report) -> GlobalResult<()> {
        // Validate ownership metadata before package work, then re-read it under
        // the publication lock after the ordinary transaction and build.
        let state = State::read(paths)?;
        let before = observe(flox, paths);
        report.package_state_before = before.lockfile.clone();
        report.before = Some(before.clone());
        if let Some(run_link) = &before.run_link {
            let preflight = publication::plan(
                paths,
                &state,
                run_link,
                before.commands.iter().map(inventory::Command::key),
            )?;
            if let Some(conflict) = preflight
                .iter()
                .find(|entry| entry.action == publication::Action::Conflict)
            {
                return Err(GlobalError::at(
                    "ownership_conflict",
                    &conflict.destination,
                    conflict
                        .reason
                        .as_deref()
                        .unwrap_or("Ownership is ambiguous"),
                ));
            }
        }
        let (mutation, dry_run) = Mutation::from_command(&self.command, flox, &before)?;
        report.dry_run = dry_run;
        if dry_run {
            let preview = preview(flox, paths, &mutation)?;
            let mut after = preview.snapshot;
            let canonical_data = fs::canonicalize(&paths.data_dir)
                .map_err(|error| GlobalError::at("unsupported_path", &paths.data_dir, error))?;
            let logical_run = canonical_data.join(
                preview
                    .run_link
                    .strip_prefix(&preview.root)
                    .expect("scratch run link belongs to scratch root"),
            );
            for command in &mut after.commands {
                command.stable_target = command.key().in_directory(&logical_run);
            }
            after.run_link = Some(logical_run.clone());
            report.package_state_after = after.lockfile.clone();
            report.environment_outcome = "not_attempted";
            let entries = publication::plan(
                paths,
                &state,
                &logical_run,
                after.commands.iter().map(inventory::Command::key),
            )?;
            let errors = entries
                .iter()
                .filter(|entry| entry.action == publication::Action::Conflict)
                .map(|entry| {
                    GlobalError::at(
                        "ownership_conflict",
                        &entry.destination,
                        entry.reason.as_deref().unwrap_or("Ownership is ambiguous"),
                    )
                })
                .collect::<Vec<_>>();
            report.publication = Some(PublicationReport {
                status: if errors.is_empty() {
                    "complete"
                } else {
                    "incomplete"
                },
                observed_output: after.observed_output.clone(),
                actual_output: report
                    .before
                    .as_ref()
                    .and_then(|b| b.observed_output.clone()),
                entries,
                completed: vec![],
                errors,
            });
            report.errors.extend(after.errors.clone());
            report.after = Some(after.clone());
            mutation.check_commands(&after)?;
            return Ok(());
        }
        report.environment_outcome = "unknown";
        let result = mutate_live(flox, paths, &mutation);
        let after = observe(flox, paths);
        report.package_state_after = after.lockfile.clone();
        report.after = Some(after.clone());
        if let Err(error) = result {
            report.environment_outcome = "failed";
            report.publication = Some(PublicationReport::unknown(GlobalError::new(
                "environment_failed",
                "Ordinary operation failed; observed targets may already have changed",
                None,
            )));
            return Err(error);
        }
        report.environment_outcome = if report.package_state_before == report.package_state_after {
            "unchanged"
        } else {
            "changed"
        };
        if let Err(error) = mutation.check_commands(&after) {
            report.publication = Some(PublicationReport::unknown(error.clone()));
            return Err(error);
        }
        report.publication = Some(reconcile(flox, paths, false));
        report.after = Some(observe(flox, paths));
        report.errors.extend(
            report
                .after
                .as_ref()
                .expect("observed above")
                .errors
                .clone(),
        );
        Ok(())
    }

    fn render(&self, report: &Report) {
        if let Some(path) = &report.directory {
            message::plain(path.display());
        }
        if let Some(shell) = &report.shell {
            if matches!(self.command, GlobalCommand::UpdateShell {
                print_script: true,
                ..
            }) {
                print!("{}", shell.script);
            } else if shell.errors.is_empty() {
                message::plain(format!(
                    "Configured global command PATH for {}. Open a new shell.",
                    shell.shell
                ));
            } else {
                for path in &shell.changed_files {
                    message::plain(format!(
                        "Updated {} before shell setup stopped.",
                        path.display()
                    ));
                }
            }
        }
        if let Some(snapshot) = &report.after {
            match &self.command {
                GlobalCommand::List { commands: false } => {
                    for package in &snapshot.packages {
                        message::plain(format!("{}: {}", package.install_id, package.request));
                    }
                },
                GlobalCommand::List { commands: true } | GlobalCommand::Which { .. } => {
                    for command in &snapshot.commands {
                        message::plain(format!(
                            "{}/{}: {} [{}] -> {}",
                            command.directory.name(),
                            command.name,
                            if command.provider_install_ids.is_empty() {
                                "requester unresolved".into()
                            } else {
                                command.provider_install_ids.join(", ")
                            },
                            command.publication_status,
                            command.stable_target.display()
                        ));
                    }
                    if matches!(self.command, GlobalCommand::Which { .. }) {
                        message::plain(format!(
                            "Current PATH: {}",
                            report
                                .selected_by_path
                                .as_ref()
                                .map(|p| p.display().to_string())
                                .unwrap_or_else(|| "not found".into())
                        ));
                    }
                },
                _ => {},
            }
        }
        if let Some(publication) = &report.publication {
            message::plain(format!(
                "{}Ordinary environment: {}. Publication: {}.",
                if report.dry_run { "Dry run: " } else { "" },
                report.environment_outcome,
                publication.status
            ));
            for entry in &publication.entries {
                if entry.action != publication::Action::Unchanged {
                    message::plain(format!(
                        "{:?}: {} -> {}",
                        entry.action,
                        entry.destination.display(),
                        entry.target.display()
                    ));
                }
            }
            if !report.path.bin_on_path || !report.path.sbin_on_path {
                message::plain(
                    "PATH setup is incomplete. Run 'flox global update-shell' and open a new shell.",
                );
            }
            for error in &publication.errors {
                eprintln!("{}: {}", error.category, error.message);
            }
            if publication.status != "complete" {
                eprintln!("Inspect preserved conflicts, then run 'flox global retry-publication'.");
            }
        }
        for error in &report.errors {
            eprintln!("{}: {}", error.category, error.message);
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct PathStatus {
    bin: PathBuf,
    sbin: PathBuf,
    bin_on_path: bool,
    sbin_on_path: bool,
}

#[derive(Clone, Debug, Serialize)]
struct Report {
    schema_version: u8,
    operation: &'static str,
    dry_run: bool,
    environment_directory: PathBuf,
    environment_outcome: &'static str,
    before: Option<Snapshot>,
    after: Option<Snapshot>,
    package_state_before: Option<Lockfile>,
    package_state_after: Option<Lockfile>,
    publication: Option<PublicationReport>,
    path: PathStatus,
    directory: Option<PathBuf>,
    selected_by_path: Option<PathBuf>,
    shell: Option<shell::ShellReport>,
    errors: Vec<GlobalError>,
}

impl Report {
    fn new(operation: &'static str, paths: &GlobalPaths) -> Self {
        let current = env::var_os("PATH")
            .map(|v| env::split_paths(&v).collect::<Vec<_>>())
            .unwrap_or_default();
        let bin = paths.data_dir.join("bin");
        let sbin = paths.data_dir.join("sbin");
        Self {
            schema_version: 1,
            operation,
            dry_run: false,
            environment_directory: paths.environment_dir(),
            environment_outcome: "not_attempted",
            before: None,
            after: None,
            package_state_before: None,
            package_state_after: None,
            publication: None,
            path: PathStatus {
                bin_on_path: current.contains(&bin),
                sbin_on_path: current.contains(&sbin),
                bin,
                sbin,
            },
            directory: None,
            selected_by_path: None,
            shell: None,
            errors: vec![],
        }
    }
}

#[derive(Clone, Debug)]
enum Mutation {
    Install(Vec<PackageToInstall>),
    Upgrade(Vec<String>),
    Uninstall(Vec<UninstallSpec>),
}

impl Mutation {
    fn from_command(
        command: &GlobalCommand,
        flox: &Flox,
        before: &Snapshot,
    ) -> GlobalResult<(Self, bool)> {
        let target = |name: &str| -> GlobalResult<String> {
            if before.packages.iter().any(|p| p.install_id == name)
                || before
                    .lockfile
                    .as_ref()
                    .and_then(|lock| lock.migrated_manifest().ok())
                    .is_some_and(|manifest| {
                        manifest
                            .as_latest_schema()
                            .pkg_or_group_found_in_manifest(name)
                    })
            {
                return Ok(name.into());
            }
            let providers = before
                .commands
                .iter()
                .filter(|c| c.name == name)
                .flat_map(|c| c.provider_install_ids.clone())
                .collect::<std::collections::BTreeSet<_>>();
            if providers.len() > 1 {
                return Err(GlobalError::new(
                    "usage",
                    format!("Command '{name}' has multiple providers; use a package install ID"),
                    None,
                ));
            }
            Ok(providers.into_iter().next().unwrap_or_else(|| name.into()))
        };
        match command {
            GlobalCommand::Install {
                id,
                packages,
                dry_run,
            } => {
                let mut parsed = packages
                    .iter()
                    .map(|p| PackageToInstall::parse(&flox.system, p))
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(|error| GlobalError::new("usage", error, None))?;
                for requested in id {
                    let mut package = PackageToInstall::parse(&flox.system, &requested.pkg)
                        .map_err(|error| GlobalError::new("usage", error, None))?;
                    package.set_id(&requested.id);
                    parsed.push(package);
                }
                if parsed.is_empty() {
                    return Err(GlobalError::new(
                        "usage",
                        "Specify at least one package",
                        None,
                    ));
                }
                Ok((Self::Install(parsed), *dry_run))
            },
            GlobalCommand::Upgrade {
                targets,
                all,
                dry_run,
            } => {
                if *all && !targets.is_empty() {
                    return Err(GlobalError::new(
                        "usage",
                        "Do not combine --all with package targets",
                        None,
                    ));
                }
                Ok((
                    Self::Upgrade(
                        targets
                            .iter()
                            .map(|name| target(name))
                            .collect::<GlobalResult<_>>()?,
                    ),
                    *dry_run,
                ))
            },
            GlobalCommand::Uninstall { targets, dry_run } => {
                let specs = targets
                    .iter()
                    .map(|name| {
                        let name = target(name)?;
                        UninstallSpec::parse(&name)
                            .map_err(|error| GlobalError::new("usage", error, None))
                    })
                    .collect::<GlobalResult<_>>()?;
                Ok((Self::Uninstall(specs), *dry_run))
            },
            _ => unreachable!("only package operations create a mutation"),
        }
    }

    fn check_commands(&self, snapshot: &Snapshot) -> GlobalResult<()> {
        if let Self::Install(packages) = self {
            for requested in packages {
                let exposes_command = snapshot
                    .packages
                    .iter()
                    .filter(|package| package.install_id == requested.id())
                    .flat_map(|package| &package.selected_outputs)
                    .any(|output| {
                        snapshot.commands.iter().any(|command| {
                            fs::metadata(command.key().in_directory(output)).is_ok_and(|metadata| {
                                metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
                            })
                        })
                    });
                if !exposes_command {
                    return Err(GlobalError::new(
                        "no_commands",
                        format!(
                            "Package '{}' exposes no public CLI commands in its selected outputs. Ordinary installed state is shown separately.",
                            requested.id()
                        ),
                        None,
                    ));
                }
            }
        }
        Ok(())
    }
}

fn observe(flox: &Flox, paths: &GlobalPaths) -> Snapshot {
    match inventory::read(paths, &flox.system) {
        Ok(mut snapshot) => {
            inventory::observe_publication(paths, &mut snapshot);
            snapshot
        },
        Err(error) => Snapshot {
            errors: vec![error],
            ..Snapshot::default()
        },
    }
}

fn mutate_live(flox: &Flox, paths: &GlobalPaths, mutation: &Mutation) -> GlobalResult<()> {
    let parent = paths.environment_dir();
    let mut environment = if !dot_flox_exists(paths)? {
        if !matches!(mutation, Mutation::Install(_)) {
            return Err(GlobalError::at(
                "environment_failed",
                &parent,
                "No global environment exists yet",
            ));
        }
        fs::create_dir_all(&parent)
            .map_err(|error| GlobalError::at("environment_failed", &parent, error))?;
        PathEnvironment::init_bare(
            PathPointer::new("global".parse().expect("valid environment name")),
            &parent,
            flox,
        )
    } else {
        let dot = DotFlox::open_in(&parent)
            .map_err(|error| GlobalError::at("environment_failed", &parent, error))?;
        let EnvironmentPointer::Path(pointer) = dot.pointer else {
            return Err(GlobalError::at(
                "unsupported_environment",
                &parent,
                "Remote association is deferred",
            ));
        };
        let canonical = CanonicalPath::new(dot.path)
            .map_err(|error| GlobalError::at("environment_failed", &parent, error))?;
        PathEnvironment::open(flox, pointer, canonical)
    }
    .map_err(|error| GlobalError::at("environment_failed", &parent, error))?;
    let result = match mutation {
        Mutation::Install(packages) => environment.install(packages, flox).map(|_| ()),
        Mutation::Uninstall(specs) => environment.uninstall(specs.clone(), flox).map(|_| ()),
        Mutation::Upgrade(targets) => environment
            .upgrade(
                flox,
                &targets.iter().map(String::as_str).collect::<Vec<_>>(),
            )
            .map(|_| ()),
    };
    result.map_err(|error| GlobalError::at("environment_failed", &parent, error))
}

fn reconcile(flox: &Flox, paths: &GlobalPaths, dry_run: bool) -> PublicationReport {
    let result = (|| {
        // A missing environment plus no receipts is an idempotent retry. It
        // grants no ownership of unrelated bin/sbin entries.
        let initial = State::read(paths)?;
        if !dot_flox_exists(paths)? && initial.is_empty() {
            return Ok(PublicationReport {
                status: "complete",
                observed_output: None,
                actual_output: None,
                entries: vec![],
                completed: vec![],
                errors: vec![],
            });
        }
        let _lock = if dry_run {
            None
        } else {
            Some(publication::lock(paths)?)
        };
        let mut state = State::read(paths)?;
        let snapshot = inventory::read(paths, &flox.system)?;
        if let Some(error) = snapshot.errors.into_iter().next() {
            return Err(error);
        }
        let run_link = snapshot.run_link.ok_or_else(|| {
            GlobalError::new(
                "output_unavailable",
                "Ordinary global run link is unavailable",
                None,
            )
        })?;
        let output = snapshot.observed_output.ok_or_else(|| {
            GlobalError::at(
                "output_unavailable",
                &run_link,
                "Current ordinary output is unavailable; retry does not build it",
            )
        })?;
        let entries = publication::plan(
            paths,
            &state,
            &run_link,
            snapshot.commands.iter().map(inventory::Command::key),
        )?;
        let mut report =
            publication::apply(paths, &mut state, &run_link, &output, entries, dry_run);
        match inventory::read(paths, &flox.system) {
            Ok(after) => report.errors.extend(after.errors),
            Err(error) => report.errors.push(error),
        }
        if !report.errors.is_empty() {
            report.status = "incomplete";
        }
        Ok(report)
    })();
    result.unwrap_or_else(PublicationReport::unknown)
}

fn dot_flox_exists(paths: &GlobalPaths) -> GlobalResult<bool> {
    let path = paths.environment_dir().join(".flox");
    match fs::symlink_metadata(&path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(GlobalError::at("environment_failed", &path, error)),
    }
}

#[derive(Debug)]
struct Preview {
    _temporary: TempDir,
    root: PathBuf,
    run_link: PathBuf,
    snapshot: Snapshot,
}

fn preview(flox: &Flox, paths: &GlobalPaths, mutation: &Mutation) -> GlobalResult<Preview> {
    let temporary = tempfile::tempdir_in(&flox.temp_dir)
        .map_err(|error| GlobalError::new("environment_failed", error, None))?;
    let scratch = GlobalPaths {
        data_dir: fs::canonicalize(temporary.path())
            .map_err(|error| GlobalError::at("unsupported_path", temporary.path(), error))?,
    };
    let manifest_dir = scratch.manifest_dir();
    fs::create_dir_all(&manifest_dir)
        .map_err(|error| GlobalError::at("environment_failed", &manifest_dir, error))?;
    if paths.manifest_dir().exists() {
        copy_environment_files(&paths.manifest_dir(), &manifest_dir)?;
    } else {
        if !matches!(mutation, Mutation::Install(_)) {
            return Err(GlobalError::new(
                "environment_failed",
                "No global environment exists yet",
                None,
            ));
        }
        Manifest::parse_toml_typed(format!(
            "schema-version = \"{}\"\n",
            KnownSchemaVersion::latest()
        ))
        .map_err(|error| GlobalError::new("environment_failed", error, None))?
        .as_writable()
        .write_to_file(manifest_dir.join("manifest.toml"))
        .map_err(|error| GlobalError::new("environment_failed", error, None))?;
    }
    let run_dir = scratch.environment_dir().join(".flox/run");
    fs::create_dir_all(&run_dir)
        .map_err(|error| GlobalError::at("environment_failed", &run_dir, error))?;
    let links = RenderedEnvironmentLinks::new_in_base_dir_with_name_and_system(
        &CanonicalPath::new(&run_dir)
            .map_err(|error| GlobalError::at("environment_failed", &run_dir, error))?,
        "global",
        &flox.system,
    );
    let mut core = CoreEnvironment::new(&manifest_dir, IncludeFetcher {
        base_directory: Some(paths.environment_dir()),
    });
    let result = match mutation {
        Mutation::Install(packages) => core
            .install(packages, flox, Some(links.out_link_prefix()))
            .map(|_| ()),
        Mutation::Uninstall(specs) => core
            .uninstall(specs.clone(), flox, Some(links.out_link_prefix()))
            .map(|_| ()),
        Mutation::Upgrade(targets) => core
            .upgrade(
                flox,
                &targets.iter().map(String::as_str).collect::<Vec<_>>(),
                true,
                Some(links.out_link_prefix()),
            )
            .map(|_| ()),
    };
    result.map_err(|error| GlobalError::new("environment_failed", error, None))?;
    // Ordinary idempotent operations may not build. The preview still needs a
    // public tree to describe names, so build only its isolated copy if needed.
    if !links.run.as_path().exists() {
        core.build(flox, Some(links.out_link_prefix()))
            .map_err(|error| GlobalError::new("environment_failed", error, None))?;
    }
    let run_link = links.run.as_path().to_owned();
    let snapshot = inventory::read_at(paths, &manifest_dir, Some(&run_link), &flox.system)?;
    Ok(Preview {
        root: scratch.data_dir,
        _temporary: temporary,
        run_link,
        snapshot,
    })
}

fn copy_environment_files(source: &Path, destination: &Path) -> GlobalResult<()> {
    for entry in fs::read_dir(source)
        .map_err(|error| GlobalError::at("environment_failed", source, error))?
    {
        let entry = entry.map_err(|error| GlobalError::at("environment_failed", source, error))?;
        let target = destination.join(entry.file_name());
        let kind = entry
            .file_type()
            .map_err(|error| GlobalError::at("environment_failed", &entry.path(), error))?;
        if kind.is_dir() {
            fs::create_dir(&target)
                .map_err(|error| GlobalError::at("environment_failed", &target, error))?;
            copy_environment_files(&entry.path(), &target)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), &target)
                .map_err(|error| GlobalError::at("environment_failed", &target, error))?;
        } else {
            return Err(GlobalError::at(
                "unsupported_path",
                &entry.path(),
                "Cannot safely preview an environment containing symlinks or special files",
            ));
        }
    }
    Ok(())
}

/// Restrict early JSON errors to the global namespace, without consuming parser
/// help/completion responses or an operand named `global` on another command.
pub(crate) fn json_requested() -> bool {
    let args = env::args_os().skip(1).collect::<Vec<_>>();
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        if arg == "--floxhub-url" {
            index += 2;
            continue;
        }
        let Some(text) = arg.to_str() else {
            return false;
        };
        if text == "--quiet"
            || text == "--verbose"
            || text == "--debug"
            || text == "--beta"
            || text.starts_with("--floxhub-url=")
            || (text.starts_with('-')
                && text.len() > 1
                && text[1..].chars().all(|c| c == 'v' || c == 'q'))
        {
            index += 1;
            continue;
        }
        return text == "global"
            && args[index + 1..]
                .iter()
                .take_while(|arg| *arg != "--")
                .any(|arg| arg == "--json");
    }
    false
}

pub(crate) fn render_early_error(data_dir: &Path, category: &'static str, error: impl Display) {
    let paths = GlobalPaths {
        data_dir: data_dir.to_owned(),
    };
    let mut report = Report::new("global", &paths);
    report.environment_outcome = "unknown";
    report.errors.push(GlobalError::new(category, error, None));
    // Every field is JSON-serializable; unsupported path bytes produce an
    // explicit structured error rather than a lossy path or a panic.
    match serde_json::to_string(&report) {
        Ok(json) => println!("{json}"),
        Err(_) => println!(
            "{{\"schema_version\":1,\"operation\":\"global\",\"errors\":[{{\"category\":\"unsupported_path\",\"message\":\"A path cannot be represented as JSON\",\"path\":null}}]}}"
        ),
    }
}
