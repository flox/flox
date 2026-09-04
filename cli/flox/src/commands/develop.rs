use std::io;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use bpaf::Bpaf;
use flox_events::LifecycleFields;
use flox_manifest::lockfile::Lockfile;
use flox_manifest::{Manifest, MigratedTypedOnly};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::{ConcreteEnvironment, Environment, GCROOTS_DIR_NAME};
use flox_rust_sdk::providers::build::{
    COMMON_NIXPKGS_URL,
    FloxBuildMk,
    ManifestBuilder,
    PackageTarget,
    PackageTargetKind,
};
use flox_rust_sdk::providers::nix;
use indoc::{formatdoc, indoc};
use nef_lock_catalog::NixFlakeref;
use tempfile::{NamedTempFile, TempDir};
use thiserror::Error;
use tracing::debug;

use super::build::{
    BaseCatalogUrlSelect,
    base_catalog_url_select,
    base_nixpkgs_url_from_url_select,
    check_git_tracking_for_expression_builds,
    packages_to_build,
    prefetch_expression_build_flake_ref,
    prefetch_flake_ref,
};
use super::{
    DirEnvironmentSelect,
    SHELL_COMPLETION_COMMAND,
    dir_environment_select,
    needs_project_files_error,
};
use crate::subcommand_metric;
use crate::utils::catalog_lock::BuildLockGuard;
use crate::utils::detect_shell::INTERACTIVE_BASH_BIN;
use crate::utils::message;

/// The known divergences between this shell and the shell `flox build`
/// actually runs a package's build in. Printed in full on entry — see
/// [`Develop::print_disclosure`] — because the ones most likely to burn a
/// user are exactly the ones nobody opens a manpage to discover.
const DISCLOSURE: &str = indoc! {"
    This shell approximates the build environment for '{name}'.
    It is not the build. Known differences:
      - No build sandbox is applied here. 'flox build' runs the build under
        'nix build', which the Nix daemon may sandbox.
      - Your working tree is visible here, including files git does not
        track. A real build sees only tracked files.
      - '$src' is a snapshot in the Nix store, taken when you entered.
        'genericBuild' builds that snapshot, not your working tree; edits
        reach it only when you re-enter.
      - '$out' and the other output variables point at placeholder paths,
        not at store paths. Nothing installed there is a real build output.
      - The host PATH stays reachable after the build inputs, and if your
        '~/.bashrc' activates a Flox environment, that environment is on
        PATH here too. A real build sees only its own inputs.
      - This shell is interactive and sources '~/.bashrc'. The build shell
        does neither."};

#[derive(Bpaf, Clone)]
pub struct Develop {
    #[bpaf(external(dir_environment_select), fallback(Default::default()))]
    environment: DirEnvironmentSelect,

    #[bpaf(external(base_catalog_url_select), optional)]
    base_catalog_url_select: Option<BaseCatalogUrlSelect>,

    /// Shell command string to run in the development shell instead of entering it interactively
    #[bpaf(
        long("command"),
        short('c'),
        argument("cmd"),
        complete_shell(SHELL_COMPLETION_COMMAND)
    )]
    shell_command: Option<String>,

    /// The Nix expression package to develop.
    /// Corresponds to an expression file in '.flox/pkgs/'.
    /// If omitted, the project's sole Nix expression build is used;
    /// with more than one, name which to develop.
    #[bpaf(positional("package"))]
    pub package: Option<String>,
}

impl Develop {
    pub fn subcommand_name(&self) -> &'static str {
        "develop"
    }

    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("develop");
        Self::develop(flox, self).await
    }

    async fn develop(mut flox: Flox, opts: Develop) -> Result<()> {
        let Develop {
            environment,
            base_catalog_url_select,
            shell_command,
            package,
        } = opts;

        let mut env = environment.detect_concrete_environment(&mut flox, "Develop packages of")?;
        Self::refuse_managed_environment(&env)?;

        let base_dir = env.parent_path()?;
        let cache_path = env.cache_path()?;
        let lockfile: Lockfile = env.lockfile(&flox)?.into();
        let lockfile_manifest = lockfile.migrated_manifest()?;

        let expression_parent_dir = env.dot_flox_path();
        let expression_path_ref = NixFlakeref::from_path(&expression_parent_dir)?;
        let target = Self::resolve_target(&lockfile_manifest, &expression_path_ref, package)?;

        // An unsandboxed manifest build already refuses `--stability`
        // (`disallow_base_url_select_for_manifest_builds`, build.rs), but
        // this check never runs here: `refuse_manifest_build` below rejects
        // every manifest-build target unconditionally, before `--stability`
        // is even inspected. If that blanket refusal is ever loosened,
        // revisit whether the two checks need to be shared rather than
        // duplicated (`build.rs`'s NEF preamble and this one have already
        // drifted once).
        Self::refuse_manifest_build(&target)?;

        let expression_git_ref =
            check_git_tracking_for_expression_builds([&target], &expression_parent_dir)?;
        let expression_ref = expression_git_ref.unwrap_or(expression_path_ref);

        // Both refusal paths above are cheap; `env.build()` below realises
        // the environment's own build inputs and is the slow step, so nothing
        // between the refusals and here should need it.
        let built_environments = env.build(&flox)?;

        // The catalog lock the NEF eval consumes, created by the CLI
        // exactly as `flox build` does: the committed .flox/catalog.lock as
        // found, or a fresh ephemeral lock scoped to this package (the
        // scanner follows its imports) living only as long as this command.
        let rel_file_path = match target.kind() {
            PackageTargetKind::ExpressionBuild(expression) => expression.rel_file_path.clone(),
            // Guarded by `refuse_manifest_build` above.
            PackageTargetKind::ManifestBuild { .. } => {
                unreachable!("manifest builds are refused before the eval")
            },
        };
        let catalog_lock =
            BuildLockGuard::new_existing_or_ephemeral(&flox.floxhub_client, env.dot_flox_path(), [
                &rel_file_path,
            ])
            .await?;

        let base_nixpkgs_url =
            base_nixpkgs_url_from_url_select(&flox, base_catalog_url_select, Some(&lockfile))
                .await?
                .as_flake_ref()?;

        prefetch_flake_ref(&COMMON_NIXPKGS_URL)?;
        prefetch_expression_build_flake_ref([&target], &base_nixpkgs_url)?;

        let eval_results = FloxBuildMk::new(
            &flox,
            &base_dir,
            &expression_ref,
            &built_environments,
            &cache_path,
        )
        .eval(
            &base_nixpkgs_url,
            std::slice::from_ref(&target),
            catalog_lock.path(),
            None,
        )?;
        let drv_path = eval_results
            .first()
            .context("eval() returned no results for the requested package")?
            .drv_path
            .clone();

        // Project-scoped and keyed by system + package name, mirroring the
        // `<system>.<name>` convention `PathEnvironment`'s own GC roots use
        // (`flox-rust-sdk/src/models/environment/path_environment.rs`) --
        // ".develop" keeps this apart from that environment-level root even
        // when the package name matches the environment name.
        let gc_root_path = env.dot_flox_path().join(GCROOTS_DIR_NAME).join(format!(
            "{}.{}.develop",
            flox.system,
            target.name()
        ));
        let env_script_path =
            Self::print_dev_env(&flox, &drv_path, target.name().as_ref(), &gc_root_path)?;

        // `exec` replaces this process, so the dispatcher's end-of-run
        // `command_completed` emit (main.rs) never runs; record it here
        // first, mirroring the in-place handoff `activate` performs before
        // its own `exec` (activate.rs:741-771).
        let hub = flox_events::EventsHub::global();
        if let Err(err) = hub.record_command_completed("develop".to_string(), LifecycleFields {
            exit_code: 0,
            duration_ms: None,
            error_kind: None,
        }) {
            debug!(error = %err, "Failed to record v2 cli.command_completed event before exec");
        }
        if let Err(err) = hub.flush(flox_events::force_flush_requested()) {
            debug!(error = %err, "Failed to flush v2 events before exec");
        }

        // Mirrors 'flox activate -c': the command string runs in a
        // non-interactive subshell — no ~/.bashrc, no prompt, no
        // disclosure — with the development environment sourced first, and
        // the command's exit status becomes this process's. The string
        // reaches the wrapper as an environment variable and is run with
        // `eval`, so its text is never interpolated into shell source (the
        // same injection-safety rule the rcfile follows).
        if let Some(shell_command) = shell_command {
            let mut command = Command::new(&*INTERACTIVE_BASH_BIN);
            command.env("_FLOX_DEVELOP_ENV_SCRIPT", &env_script_path);
            command.env("_FLOX_DEVELOP_COMMAND", &shell_command);
            command
                .arg("--noprofile")
                .arg("--norc")
                .arg("-c")
                .arg(r#"source "$_FLOX_DEVELOP_ENV_SCRIPT" && eval "$_FLOX_DEVELOP_COMMAND""#);
            debug!(command = ?command, "exec'ing development shell command");

            // exec should never return
            return Err(command.exec()).context("failed to exec development shell command");
        }

        let rcfile_path = Self::render_rcfile(&flox)?;

        Self::print_disclosure(target.name().as_ref());

        let mut command = Command::new(&*INTERACTIVE_BASH_BIN);
        // `pname` and the `print-dev-env` script path are passed as
        // environment variables and referenced by name from the rcfile
        // (`render_rcfile`) rather than interpolated into its text, so
        // nothing here needs shell escaping.
        command.env("_FLOX_DEVELOP_PNAME", target.name().as_ref());
        command.env("_FLOX_DEVELOP_ENV_SCRIPT", &env_script_path);
        // `bash --rcfile` only reads the file for an interactive shell, and
        // bash decides interactivity from whether stdin and stderr are
        // ttys — not from `--rcfile` being present. Piped/redirected
        // invocations (`flox develop pkg < /dev/null`, `... 2>log`) would
        // otherwise silently land the caller in a plain host shell with no
        // build environment. `-i` forces interactive mode regardless of
        // tty status; `--noprofile` matches the equivalent branch in
        // `flox-activations/src/attach.rs` (`activate_interactive`) that
        // also has to force an rcfile-sourcing shell for non-tty callers.
        // `-i` must come after `--rcfile`: this bash rejects it as `--:
        // invalid option` when it precedes a long option.
        command
            .arg("--noprofile")
            .arg("--rcfile")
            .arg(&rcfile_path)
            .arg("-i");
        debug!(command = ?command, "exec'ing development shell");

        // exec should never return
        Err(command.exec()).context("failed to exec development shell")
    }

    /// Resolve the package to develop from the optional CLI argument.
    ///
    /// A named package is validated against the environment's known
    /// targets exactly as `flox build <package>` validates its own.
    /// With no argument, this mirrors `flox build`'s bare-invocation
    /// convention for a single-build project: the sole Nix expression
    /// build is used if there is exactly one. Manifest builds are never
    /// candidates for that fallback — `refuse_manifest_build` below
    /// refuses one unconditionally, so silently falling into one here
    /// would only relocate that refusal to a worse error.
    ///
    /// The zero-candidate case has two causes, and `packages_to_build`
    /// distinguishes them for free: called with an empty package list, it
    /// bails with its own "No packages found to build." before this
    /// function sees anything if the project defines no builds at all, so
    /// an empty `expression_targets` here only ever means the project has
    /// manifest builds and nothing else — the one case that needs its own
    /// message, pointing at `flox activate` the same way
    /// `refuse_manifest_build` does for a named manifest build.
    fn resolve_target(
        manifest: &Manifest<MigratedTypedOnly>,
        expression_ref: &NixFlakeref,
        package: Option<String>,
    ) -> Result<PackageTarget> {
        if let Some(package) = package {
            let targets = packages_to_build(manifest, expression_ref, &[package])?;
            return targets
                .into_iter()
                .next()
                .context("packages_to_build returned no targets for the requested package");
        }

        let mut expression_targets: Vec<PackageTarget> =
            packages_to_build(manifest, expression_ref, &Vec::<String>::new())?
                .into_iter()
                .filter(|target| target.kind().is_expression_build())
                .collect();

        match expression_targets.len() {
            0 => bail!(formatdoc! {"
                This project defines manifest builds but no Nix expression build to develop.
                An unsandboxed manifest build already runs against the activated environment,
                so the shell it would get is one you can enter today.

                Next:
                  $ flox activate
                "
            }),
            1 => Ok(expression_targets.remove(0)),
            _ => {
                expression_targets.sort_by_key(|target| target.name().to_string());
                let candidates = expression_targets
                    .iter()
                    .map(PackageTarget::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                bail!(formatdoc! {"
                    Multiple Nix expression packages found: {candidates}.

                    Name the one to develop:
                      $ flox develop <package>
                    "
                })
            },
        }
    }

    /// A pushed or pulled (FloxHub-linked) environment cannot be assumed to
    /// have its project files — the `.flox/pkgs/` expression and the git
    /// history `develop` needs to locate and hash it — available locally,
    /// so this reuses the same refusal `flox build`/`flox publish` give a
    /// managed environment.
    fn refuse_managed_environment(env: &ConcreteEnvironment) -> Result<()> {
        match env {
            ConcreteEnvironment::Path(_) => Ok(()),
            ConcreteEnvironment::Managed(managed) => {
                bail!(needs_project_files_error(managed, "develop"))
            },
            ConcreteEnvironment::Remote(_) => {
                // guarded by DirEnvironmentSelect
                unreachable!("Cannot develop from a remote environment")
            },
        }
    }

    /// An unsandboxed manifest build already runs against the activated
    /// environment, which is most of what this command would give it, so
    /// naming one is refused with guidance rather than served by a second
    /// code path.
    fn refuse_manifest_build(target: &PackageTarget) -> Result<()> {
        if !target.kind().is_manifest_build() {
            return Ok(());
        }

        let name = target.name();
        bail!(formatdoc! {r#"
            Cannot develop '{name}': it is a manifest build, not a Nix expression build.
            An unsandboxed manifest build already runs against the activated environment,
            so the shell it would get is one you can enter today.
            If '{name}' declares any 'sandbox' mode other than "off", set 'sandbox = "off"' first.

            Next:
              $ flox activate                      <- Enter the environment
              $ <steps from build.{name}.command>  <- Run the build by hand
            "#, name = name});
    }

    /// Realise `drv_path`'s inputs and capture `nix print-dev-env`'s output
    /// to a file under `flox.temp_dir`, wrapping a raw nix failure per this
    /// repo's rule against surfacing internal tool output (`AGENTS.md`).
    ///
    /// `nix print-dev-env` realises the derivation's *inputs* and a wrapper
    /// derivation that dumps the environment; it does not run the package's
    /// build phases, which is what lets a package that fails to build still
    /// yield a shell.
    ///
    /// `gc_root_path` is rooted with `nix build --out-link`, the idiom
    /// `providers/buildenv.rs` already uses elsewhere in this codebase,
    /// rather than a `nix profile`: a profile's own indirection (`p ->
    /// p-1-link -> store path`) buys nothing here and appears nowhere else
    /// in flox. `print-dev-env` is first pointed at a throwaway `--profile`
    /// under `flox.temp_dir`, purely to learn the realised wrapper
    /// derivation's store path (`std::fs::canonicalize` resolves the
    /// profile's symlink chain); `nix build --out-link` then roots that
    /// store path directly, returning near-instantly since the path is
    /// already realised (confirmed locally against a warm store). Its
    /// store references include every realised build input (confirmed with
    /// `nix-store -q --references`), so rooting it keeps the whole closure
    /// alive for as long as the shell is open. This is the only root a
    /// develop shell gets -- `develop()` ends in `exec`, so no flox process
    /// survives to hold a session-scoped one. `--out-link` overwrites
    /// exactly its own path on each call, so re-entering the same package
    /// repoints the existing root rather than adding a new one.
    fn print_dev_env(
        flox: &Flox,
        drv_path: &Path,
        pname: &str,
        gc_root_path: &Path,
    ) -> Result<PathBuf, DevelopError> {
        let gc_root_dir = gc_root_path
            .parent()
            .expect("gc_root_path is always constructed with a parent directory");
        std::fs::create_dir_all(gc_root_dir).map_err(DevelopError::CreateGcRootDir)?;

        // `--profile` refuses to write over a pre-existing regular file
        // (verified locally: "filesystem error: in read_symlink: Invalid
        // argument"), so this needs a path that doesn't exist yet rather
        // than one `NamedTempFile` has already created. A fresh `TempDir`
        // gives that for free -- nix creates the symlink itself inside it.
        let discovery_dir =
            TempDir::new_in(&flox.temp_dir).map_err(DevelopError::CreateGcRootDir)?;
        let discovery_profile_path = discovery_dir.path().join("develop-profile");

        let mut cmd = nix::nix_base_command();
        cmd.arg("print-dev-env").arg(drv_path);
        cmd.arg("--profile").arg(&discovery_profile_path);
        cmd.stdout(Stdio::piped());
        // `Command::output()` always pipes both streams (overriding any
        // `Stdio` set beforehand), which buffers nix's build/eval progress
        // out of sight for however long a cold store takes to realise the
        // derivation's inputs. Spawning directly and inheriting stderr
        // keeps that progress visible; stdout is piped straight to the
        // env-script file below instead of held in memory.
        cmd.stderr(Stdio::inherit());

        let mut child = cmd.spawn().map_err(DevelopError::CallPrintDevEnv)?;
        let mut stdout = child
            .stdout
            .take()
            .expect("stdout is piped by cmd.stdout(Stdio::piped()) above");

        let env_script_path = NamedTempFile::new_in(&flox.temp_dir)
            .map_err(DevelopError::CreateEnvScriptFile)?
            .into_temp_path();
        // SAFETY: according to the docs, this is fallible on _Windows_
        let env_script_path = env_script_path
            .keep()
            .expect("failed to keep env script file");
        let mut env_script_file =
            std::fs::File::create(&env_script_path).map_err(DevelopError::CreateEnvScriptFile)?;
        io::copy(&mut stdout, &mut env_script_file).map_err(DevelopError::CreateEnvScriptFile)?;

        let status = child.wait().map_err(DevelopError::CallPrintDevEnv)?;

        if !status.success() {
            // The `drvPath` is not GC-rooted between the eval above and this
            // call, and this window is longer than the one the makefile's
            // own `build` goal guards internally. A pre-flight existence
            // check cannot close that race and only adds a stat to the
            // happy path, so the classification happens here, on the
            // failure path only.
            if !drv_path.exists() {
                return Err(DevelopError::DerivationGarbageCollected {
                    pname: pname.to_string(),
                });
            }
            // stderr was inherited above, so nix's own failure output is
            // already on the terminal; embedding it here too would violate
            // this repo's rule against surfacing internal tool output
            // (AGENTS.md).
            return Err(DevelopError::PrintDevEnv {
                pname: pname.to_string(),
            });
        }

        let env_derivation_path = std::fs::canonicalize(&discovery_profile_path)
            .map_err(DevelopError::CreateGcRootDir)?;

        let mut gc_root_cmd = nix::nix_base_command();
        gc_root_cmd
            .arg("build")
            .arg("--out-link")
            .arg(gc_root_path)
            .arg(&env_derivation_path);
        let gc_root_status = gc_root_cmd
            .status()
            .map_err(DevelopError::CreateGcRootDir)?;
        if !gc_root_status.success() {
            return Err(DevelopError::CreateGcRootDir(std::io::Error::other(
                format!("'nix build --out-link' exited with {gc_root_status}"),
            )));
        }

        Ok(env_script_path)
    }

    /// Render the rcfile a develop shell is exec'd with: `~/.bashrc` first
    /// (guarded by `_flox_sourcing_rc`), then the `print-dev-env` output,
    /// then the Flox prompt.
    ///
    /// The order is load-bearing: the env script overwrites `PATH` with the
    /// build inputs and then appends whatever `PATH` was live when it was
    /// sourced, so `~/.bashrc` must run first or the user's own `PATH`
    /// entries land in front of the build inputs and shadow the build's
    /// toolchain.
    fn render_rcfile(flox: &Flox) -> Result<PathBuf, DevelopError> {
        // `pname` (from `showAttrPath`, which Nix-quotes non-identifier
        // names and passes characters like backticks through verbatim —
        // e.g. a package file named `` `id`.nix ``) and the env-script path
        // both need to reach the rcfile below. The prompt line embeds
        // `pname` *inside* an already-double-quoted assignment, where
        // backticks and `$(...)` are still live, so quoting `pname` at the
        // Rust level can't neutralize them — only keeping the value out of
        // the file's text does. Both are passed as environment variables
        // (set on the exec'd `Command` in `develop()`) and referenced here
        // by name instead of being interpolated into the generated source.
        let rcfile_content = formatdoc! {r#"
            # 1. User config first — REQUIRED to be first. The env script
            #    sourced below overwrites PATH with the build inputs and then
            #    appends whatever PATH was live when it was sourced. Run
            #    ~/.bashrc afterwards instead and the user's own PATH entries
            #    land in front of the build inputs, shadowing the build's
            #    toolchain. `$HOME` is read directly by the shell that
            #    sources this file rather than interpolated here, so a
            #    non-UTF-8 `$HOME` still resolves correctly instead of
            #    silently failing the `-f` test below.
            #
            #    _flox_sourcing_rc is the guard flox's own activation rcfile
            #    sets (flox-activations/src/gen_rc/bash.rs), read back at
            #    attach.rs. It stops a subshell 'flox activate' inside
            #    ~/.bashrc from re-sourcing ~/.bashrc from inside this very
            #    sourcing. It does NOT suppress the activation itself — see
            #    the disclosure printed before this shell's prompt.
            if [ -n "${{PS1:-}}" ] && [ -f "$HOME/.bashrc" ]; then
              export _flox_sourcing_rc=true
              source "$HOME/.bashrc"
              unset _flox_sourcing_rc
            fi

            # 2. The `nix print-dev-env` output: build inputs, stdenv
            #    functions, NIX_BUILD_TOP/TMPDIR fixups, shellHook eval. It
            #    sets nix_saved_PATH/nix_saved_XDG_DATA_DIRS itself, from the
            #    PATH live at this point — this rcfile must not pre-set or
            #    clobber them.
            source "${{_FLOX_DEVELOP_ENV_SCRIPT}}"

            # 3. Flox prompt: wrap the existing PS1, never replace it. This
            #    duplicates the wrap-not-replace logic in
            #    assets/environment-interpreter/activate/activate.d/set-prompt.bash
            #    rather than sourcing it: that asset depends on activation-time
            #    state (FLOX_PROMPT_ENVIRONMENTS, _activate_d) a develop shell
            #    never sets. The saved-PS1 variable is develop-private
            #    (`_FLOX_DEVELOP_SAVE_PS1`, not `FLOX_SAVE_BASH_PS1`):
            #    `set-prompt.bash` reads and clears the shared name on
            #    `flox deactivate`, and a `~/.bashrc` that runs `flox
            #    activate` would otherwise both capture this shell's
            #    already-marked prompt as the "original" one and have its
            #    own restore wipe this shell's marker out from under it.
            if [ -n "${{PS1:-}}" ]; then
              if [ -z "${{_FLOX_DEVELOP_SAVE_PS1:-}}" ]; then
                export _FLOX_DEVELOP_SAVE_PS1="$PS1"
              fi
              if [ "${{NO_COLOR:-0}}" = "0" ]; then
                __flox_develop_marker="\[\e[1m\]flox [develop: ${{_FLOX_DEVELOP_PNAME}}]\[\e[0m\] "
              else
                __flox_develop_marker="flox [develop: ${{_FLOX_DEVELOP_PNAME}}] "
              fi
              case "$_FLOX_DEVELOP_SAVE_PS1" in
                *\\n*) PS1="${{_FLOX_DEVELOP_SAVE_PS1/\\n/\\n$__flox_develop_marker}}" ;;
                *\\012*) PS1="${{_FLOX_DEVELOP_SAVE_PS1/\\012/\\012$__flox_develop_marker}}" ;;
                *) PS1="$__flox_develop_marker$_FLOX_DEVELOP_SAVE_PS1" ;;
              esac
              unset __flox_develop_marker
            fi
            "#
        };

        let rcfile_path = NamedTempFile::new_in(&flox.temp_dir)
            .map_err(DevelopError::CreateRcFile)?
            .into_temp_path();
        // SAFETY: according to the docs, this is fallible on _Windows_
        let rcfile_path = rcfile_path.keep().expect("failed to keep rcfile");
        std::fs::write(&rcfile_path, rcfile_content).map_err(DevelopError::CreateRcFile)?;

        Ok(rcfile_path)
    }

    /// Print the fixed six-item disclosure list as a single `message::info`
    /// block, honoring the CLI's one-emoji-per-response rule.
    fn print_disclosure(pname: &str) {
        message::info(DISCLOSURE.replace("{name}", pname));
    }
}

#[derive(Debug, Error)]
pub(crate) enum DevelopError {
    #[error("Failed to call 'nix print-dev-env'.")]
    CallPrintDevEnv(#[source] std::io::Error),

    #[error("Failed to write the development shell's environment script.")]
    CreateEnvScriptFile(#[source] std::io::Error),

    #[error("Failed to write the development shell's rcfile.")]
    CreateRcFile(#[source] std::io::Error),

    #[error("Failed to prepare the development shell's GC root.")]
    CreateGcRootDir(#[source] std::io::Error),

    #[error("Failed to build the development environment for '{pname}'.")]
    PrintDevEnv { pname: String },

    #[error(
        "The derivation for '{pname}' was garbage collected between evaluation and use.\nPlease try again."
    )]
    DerivationGarbageCollected { pname: String },
}

#[cfg(test)]
mod tests {
    use flox_rust_sdk::flox::test_helpers::{flox_instance, flox_instance_with_optional_floxhub};
    use flox_rust_sdk::models::environment::managed_environment::test_helpers::mock_managed_environment_in;
    use flox_rust_sdk::models::environment::path_environment::test_helpers::new_path_environment;
    use flox_rust_sdk::providers::build::test_helpers::{
        prepare_empty_expressions_ref,
        prepare_nix_expressions_in,
    };
    use flox_rust_sdk::providers::build::{ExpressionBuildMetadata, PackageTargetKind};

    use super::*;

    /// A pushed/pulled environment is refused with the same
    /// `needs_project_files_error` message `flox build`/`flox publish`
    /// give, naming FloxHub and the `flox pull --copy` escape hatch.
    #[test]
    fn refuse_managed_environment_names_floxhub_and_pull_copy() {
        let owner = "owner".parse().unwrap();
        let (flox, tempdir) = flox_instance_with_optional_floxhub(Some(&owner));
        let environment_path = tempdir.path().join("environment");
        std::fs::create_dir(&environment_path).unwrap();
        let managed =
            mock_managed_environment_in(&flox, "version = 1\n", owner, &environment_path, None);

        let message = Develop::refuse_managed_environment(&ConcreteEnvironment::Managed(managed))
            .unwrap_err()
            .to_string();
        assert!(message.contains("FloxHub"));
        assert!(message.contains("flox pull --copy"));
    }

    #[test]
    fn refuse_manifest_build_allows_expression_builds() {
        let target = PackageTarget::new_unchecked(
            "greet",
            PackageTargetKind::ExpressionBuild(ExpressionBuildMetadata {
                rel_file_path: Default::default(),
            }),
        );
        assert!(Develop::refuse_manifest_build(&target).is_ok());
    }

    #[test]
    fn refuse_manifest_build_names_activate_and_sandbox() {
        let target = PackageTarget::new_unchecked("greet", PackageTargetKind::ManifestBuild {
            sandbox: None,
        });
        let message = Develop::refuse_manifest_build(&target)
            .unwrap_err()
            .to_string();
        assert!(message.contains("flox activate"));
        assert!(message.contains("sandbox"));
        assert!(message.contains("greet"));
    }

    /// The `drvPath` a develop shell is built from is not GC-rooted between
    /// the eval that produced it and the `nix print-dev-env` call that
    /// consumes it. If the derivation is collected in that window, the
    /// failure is classified as `DerivationGarbageCollected` rather than
    /// surfacing nix's own "does not exist" error, per the flox repo's rule
    /// against surfacing internal tool output.
    #[test]
    fn print_dev_env_reports_a_collected_derivation() {
        let (flox, _temp_dir) = flox_instance();
        let missing_drv_path = flox.temp_dir.join("does-not-exist.drv");
        let gc_root_path = flox.temp_dir.join("develop-gc-root-test");

        let err =
            Develop::print_dev_env(&flox, &missing_drv_path, "greet", &gc_root_path).unwrap_err();

        assert!(matches!(
            err,
            DevelopError::DerivationGarbageCollected { pname } if pname == "greet"
        ));
    }

    /// With no package argument and exactly one Nix expression build in
    /// the project, that build is the one resolved -- mirroring `flox
    /// build`'s own bare-invocation behavior for a single-build project.
    #[test]
    fn resolve_target_selects_sole_expression_build() {
        let (flox, tempdir) = flox_instance();
        let mut env = new_path_environment(&flox, "version = 1\n");
        let expression_ref = prepare_nix_expressions_in(&tempdir, &[(&["greet"], indoc! {r#"
            {runCommand}: runCommand "greet" {} ""
        "#})]);
        let lockfile: Lockfile = env.lockfile(&flox).unwrap().into();
        let lockfile_manifest = lockfile.migrated_manifest().unwrap();

        let target = Develop::resolve_target(&lockfile_manifest, &expression_ref, None).unwrap();
        assert_eq!(target.name().to_string(), "greet");
    }

    /// With more than one Nix expression build and no package argument,
    /// resolution is refused with an error naming every candidate rather
    /// than picking one arbitrarily.
    #[test]
    fn resolve_target_names_candidates_when_multiple_expression_builds_exist() {
        let (flox, tempdir) = flox_instance();
        let mut env = new_path_environment(&flox, "version = 1\n");
        let expression_ref = prepare_nix_expressions_in(&tempdir, &[
            (&["greet"], indoc! {r#"
                {runCommand}: runCommand "greet" {} ""
            "#}),
            (&["farewell"], indoc! {r#"
                {runCommand}: runCommand "farewell" {} ""
            "#}),
        ]);
        let lockfile: Lockfile = env.lockfile(&flox).unwrap().into();
        let lockfile_manifest = lockfile.migrated_manifest().unwrap();

        // Joined in sorted order ("farewell" before "greet"), not insertion
        // or `HashMap` iteration order: a `--partial` check for each name
        // separately would still pass if the sort in `resolve_target` were
        // deleted, since both names appear either way.
        let message = Develop::resolve_target(&lockfile_manifest, &expression_ref, None)
            .unwrap_err()
            .to_string();
        assert!(message.contains("farewell, greet"));
    }

    /// With no Nix expression builds and no manifest builds either, a bare
    /// `flox develop` is refused with `packages_to_build`'s own "no
    /// packages found" error -- the same one `flox build` gets for the
    /// same project state -- rather than a second message for the same
    /// condition.
    #[test]
    fn resolve_target_errors_when_no_expression_builds_exist() {
        let (flox, _tempdir) = flox_instance();
        let mut env = new_path_environment(&flox, "version = 1\n");
        let expression_ref = prepare_empty_expressions_ref();
        let lockfile: Lockfile = env.lockfile(&flox).unwrap().into();
        let lockfile_manifest = lockfile.migrated_manifest().unwrap();

        let message = Develop::resolve_target(&lockfile_manifest, expression_ref, None)
            .unwrap_err()
            .to_string();
        assert!(message.contains("No packages found to build"));
    }

    /// A manifest build is not a candidate for the bare-invocation
    /// fallback: entering its shell unconditionally is the job
    /// `refuse_manifest_build` already refuses. Unlike a project with no
    /// builds at all, this project has something to build -- just not
    /// with `flox develop` -- so the refusal must say so and point at
    /// `flox activate` instead of repeating the "no packages found"
    /// message a genuinely empty project gets.
    #[test]
    fn resolve_target_points_at_activate_when_only_manifest_builds_exist() {
        let (flox, _tempdir) = flox_instance();
        let manifest = formatdoc! {r#"
            version = 1

            [build.greet]
            command = ""
        "#};
        let mut env = new_path_environment(&flox, &manifest);
        let expression_ref = prepare_empty_expressions_ref();
        let lockfile: Lockfile = env.lockfile(&flox).unwrap().into();
        let lockfile_manifest = lockfile.migrated_manifest().unwrap();

        let message = Develop::resolve_target(&lockfile_manifest, expression_ref, None)
            .unwrap_err()
            .to_string();
        assert!(message.contains("flox activate"));
        assert!(!message.contains("No packages found to build"));
    }

    /// A named package is still validated against the environment's known
    /// targets, independent of how many Nix expression builds exist.
    #[test]
    fn resolve_target_selects_named_package_among_several() {
        let (flox, tempdir) = flox_instance();
        let mut env = new_path_environment(&flox, "version = 1\n");
        let expression_ref = prepare_nix_expressions_in(&tempdir, &[
            (&["greet"], indoc! {r#"
                {runCommand}: runCommand "greet" {} ""
            "#}),
            (&["farewell"], indoc! {r#"
                {runCommand}: runCommand "farewell" {} ""
            "#}),
        ]);
        let lockfile: Lockfile = env.lockfile(&flox).unwrap().into();
        let lockfile_manifest = lockfile.migrated_manifest().unwrap();

        let target = Develop::resolve_target(
            &lockfile_manifest,
            &expression_ref,
            Some("farewell".to_string()),
        )
        .unwrap();
        assert_eq!(target.name().to_string(), "farewell");
    }
}
