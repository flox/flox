use std::io::{BufWriter, stdout};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::{env, fs};

use anyhow::{Context, Result, anyhow, bail};
use bpaf::{Bpaf, Parser};
use crossterm::tty::IsTty;
use flox_activations::sandbox::grants;
use flox_core::activate::context::{
    ActivateCtx,
    ActivateMode,
    AttachCtx,
    AttachProjectCtx,
    InvocationType,
    SandboxMode,
};
use flox_core::activate::sandbox_backend::{FLOX_SANDBOX_BACKEND_VAR, SandboxBackend};
use flox_core::activate::vars::{FLOX_ACTIVATIONS_BIN, FLOX_ACTIVATIONS_VERBOSITY_VAR};
use flox_core::activations::activation_state_dir_path;
use flox_core::data::System;
use flox_core::data::environment_ref::DEFAULT_NAME;
use flox_core::traceable_path;
use flox_events::EventsHub;
use flox_manifest::interfaces::{AsLatestSchema, AsWritableManifest, WriteManifest};
use flox_manifest::parsed::Inner;
use flox_manifest::parsed::common::IncludeDescriptor;
use flox_manifest::{Manifest, MigratedTypedOnly};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::floxmeta_branch::BranchOrd;
use flox_rust_sdk::models::environment::generations::{GenerationId, GenerationsExt};
use flox_rust_sdk::models::environment::managed_environment::DivergedMetadata;
use flox_rust_sdk::models::environment::{
    ConcreteEnvironment,
    Environment,
    EnvironmentError,
    UpgradeResult,
};
use flox_rust_sdk::providers::lock_manifest::LockResult;
use flox_rust_sdk::providers::services::process_compose::{PROCESS_COMPOSE_BIN, ProcessStates};
use flox_rust_sdk::providers::upgrade_checks::UpgradeInformationGuard;
use flox_rust_sdk::utils::FLOX_INTERPRETER;
use indoc::{formatdoc, indoc};
use toml_edit::Key;
use tracing::{debug, trace, warn};

use super::{
    EnvironmentSelect,
    UninitializedEnvironment,
    activated_environments,
    environment_description,
    environment_select,
};
use crate::commands::check_for_upgrades::spawn_detached_check_for_upgrades_process;
use crate::commands::general::update_config_with_query;
use crate::commands::services::ServicesCommandsError;
use crate::commands::{
    EnvironmentSelectError,
    SHELL_COMPLETION_COMMAND,
    SHELL_COMPLETION_FILE,
    ensure_environment_trust,
    render_composition_manifest,
    uninitialized_environment_description,
};
use crate::config::{AutoActivationPreference, Config, EnvironmentPromptConfig};
use crate::utils::detect_shell::{detect_shell_for_in_place, detect_shell_for_subshell};
use crate::utils::errors::format_diverged_metadata;
use crate::utils::events::env_detail_from_concrete;
use crate::utils::message;
use crate::utils::metrics::metrics_events_host;
use crate::utils::upgrade_output::{count_upgrade_categories, format_upgrade_summary};
use crate::{Exit, environment_subcommand_metric, subcommand_metric, utils};

#[derive(Debug, Clone, Bpaf)]
pub enum CommandSelect {
    ShellCommand {
        /// Shell command string to run in a subshell started in the activated environment
        #[bpaf(
            long("command"),
            short('c'),
            argument("cmd"),
            complete_shell(SHELL_COMPLETION_COMMAND)
        )]
        shell_command: String,
    },
    ExecCommand {
        /// Command to exec in the activated environment. This does not run any profile scripts
        #[bpaf(positional("cmd"), strict, complete_shell(SHELL_COMPLETION_COMMAND))]
        command: String,
        #[bpaf(positional("arg"), strict, complete_shell(SHELL_COMPLETION_FILE), many)]
        args: Vec<String>,
    },
}

#[derive(Bpaf, Clone)]
pub struct Activate {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    pub environment: EnvironmentSelect,

    #[bpaf(external(activate_subcommand_or_options))]
    pub subcommand_or_options: ActivateSubcommandOrOptions,
}

impl Activate {
    /// Centrally-derived subcommand string for this invocation. Returns
    /// the `activate::allow` / `activate::deny` form for the auto-activate
    /// permission-management sub-commands, preserving the join-key
    /// continuity the legacy `environment_subcommand_metric!` stream
    /// already used.
    pub fn subcommand_name(&self) -> &'static str {
        match &self.subcommand_or_options {
            ActivateSubcommandOrOptions::AutoActivate { auto_activate } => match auto_activate {
                AutoActivate::Allow => "activate::allow",
                AutoActivate::Deny => "activate::deny",
            },
            ActivateSubcommandOrOptions::ActivateOptions { .. } => "activate",
        }
    }
}

#[derive(Bpaf, Clone)]
pub enum ActivateSubcommandOrOptions {
    AutoActivate {
        #[bpaf(external(auto_activate))]
        auto_activate: AutoActivate,
    },

    ActivateOptions {
        #[bpaf(external(activate_options))]
        options: ActivateOptions,
    },
}

#[derive(Bpaf, Debug, Clone, Copy)]
pub enum AutoActivate {
    /// Allow auto-activation for an environment
    #[bpaf(command)]
    Allow,

    /// Deny auto-activation for an environment
    #[bpaf(command)]
    Deny,
}

#[derive(Bpaf, Clone)]
pub struct ActivateOptions {
    /// Trust a remote environment temporarily for this activation, including
    /// the includes of any remote environments.
    #[bpaf(long, short)]
    pub trust: bool,

    /// Print an activation script to stdout instead of spawning a subshell
    #[bpaf(long("print-script"), short, hide)]
    pub print_script: bool,

    /// Whether to start services when activating the environment
    #[bpaf(long, short)]
    pub start_services: bool,

    /// Suppress automatic service startup even if configured in manifest
    #[bpaf(long)]
    pub no_start_services: bool,

    /// Activate the environment in either "dev" or "run" mode.
    /// Overrides the "options.activate.mode" setting in the manifest.
    #[bpaf(short, long)]
    pub mode: Option<ActivateMode>,

    // Help text lives in `sandbox_flag()` because external parsers don't
    // pick up doc comments.
    #[bpaf(external(sandbox_flag))]
    pub sandbox: Option<SandboxMode>,

    /// Select the sandbox enforcement backend: "libsandbox" (default), "nix",
    /// "host-native", "srt", "oci", or "libkrun". Overrides
    /// FLOX_SANDBOX_BACKEND; only takes effect with an active --sandbox mode.
    /// Experimental prototype.
    #[bpaf(long("sandbox-backend"), argument("BACKEND"))]
    pub sandbox_backend: Option<SandboxBackend>,

    /// Activate a FloxHub environment at a specific generation.
    #[bpaf(long, short)]
    pub generation: Option<GenerationId>,

    #[bpaf(external(command_select), optional)]
    pub command: Option<CommandSelect>,
}

/// Parser for the `--sandbox` flag.
///
/// `--sandbox <MODE>` selects an explicit mode; a bare `--sandbox` (followed
/// by another flag, `--`, or end of input) selects `prompt`. The
/// `[valued, bare]` ordering is load-bearing: the valued branch must win when
/// a mode word follows, while the hidden bare branch matches when no value is
/// present.
fn sandbox_flag() -> impl Parser<Option<SandboxMode>> {
    let valued = bpaf::long("sandbox")
        .help("Mediate filesystem access during this activation: \"off\" (default), \"warn\", \"enforce\", or \"prompt\". A bare --sandbox means \"prompt\". Experimental prototype; requires the sandbox_activate feature flag.")
        .argument::<SandboxMode>("MODE");
    let bare = bpaf::long("sandbox").req_flag(SandboxMode::Prompt).hide();
    bpaf::construct!([valued, bare]).optional()
}

impl ActivateOptions {
    /// Validate that `--start-services` and `--no-start-services` are not
    /// used together, since they are mutually exclusive.
    fn validate_service_flags(&self) -> Result<()> {
        if self.start_services && self.no_start_services {
            bail!("--start-services and --no-start-services are mutually exclusive");
        }
        Ok(())
    }
}

impl Activate {
    pub async fn handle(self, mut config: Config, mut flox: Flox) -> Result<()> {
        let options = match self.subcommand_or_options {
            ActivateSubcommandOrOptions::AutoActivate { auto_activate } => {
                return self
                    .handle_auto_activation_subcommand(auto_activate, config, flox)
                    .await;
            },
            ActivateSubcommandOrOptions::ActivateOptions { options } => {
                options.validate_service_flags()?;
                options
            },
        };

        let mut concrete_environment = match self
            .environment
            .to_concrete_environment(&mut flox, options.generation)
            .await
        {
            Ok(concrete_environment) => concrete_environment,
            Err(e @ EnvironmentSelectError::EnvNotFoundInCurrentDirectory) => {
                bail!(formatdoc! {"
            {e}

            Create an environment with 'flox init'"
                })
            },
            Err(EnvironmentSelectError::Anyhow(e)) => Err(e)?,
            Err(e) => Err(e)?,
        };

        environment_subcommand_metric!(
            "activate",
            concrete_environment,
            start_services = options.start_services,
            mode = options
                .mode
                .clone()
                .unwrap_or(ActivateMode::Dev)
                .to_string()
        );

        // Both telemetry stacks emit in parallel through the dormant
        // phase; the new-pipeline mirrors below are no-ops in production
        // until the cutover PR installs an `EventsHub` client.
        //
        // This v2 emit sits at the same dispatch point as the legacy
        // `environment_subcommand_metric!` above — before the remote-trust
        // check below — to mirror it 1:1 (parity contract). The activation
        // *outcome* is carried on `cli.command_completed` (exit_code), not by
        // the presence of this dispatch-time event, so emitting before a
        // possible trust decline is intentional, not a logged false success.
        let v2_env_detail = env_detail_from_concrete(&concrete_environment);
        let v2_mode = options
            .mode
            .clone()
            .unwrap_or(ActivateMode::Dev)
            .to_string();
        if let Err(err) = EventsHub::global().record_environment_activate_with(v2_env_detail, |p| {
            p.with_start_services(options.start_services)
                .with_mode(v2_mode)
        }) {
            debug!(error = %err, "Failed to record v2 event");
        }

        if let ConcreteEnvironment::Remote(ref env) = concrete_environment
            && !options.trust
        {
            ensure_environment_trust(
                &mut config,
                &flox,
                &env.env_ref(),
                false,
                &env.manifest_without_migrating(&flox)?
                    .as_writable()
                    .to_string(),
            )
            .await?;
        }

        let invocation_type = match options.command {
            None => {
                if options.print_script || !stdout().is_tty() {
                    InvocationType::InPlace
                } else {
                    InvocationType::Interactive
                }
            },
            Some(CommandSelect::ExecCommand {
                ref command,
                ref args,
            }) => {
                if command.is_empty() {
                    bail!("empty command provided");
                } else {
                    let mut exec_command = vec![command.clone()];
                    exec_command.extend(args.iter().cloned());
                    InvocationType::ExecCommand(exec_command)
                }
            },
            Some(CommandSelect::ShellCommand { ref shell_command }) => {
                InvocationType::ShellCommand(shell_command.clone())
            },
        };

        // Reject sandboxed in-place activation before anything reaches the
        // stdout statement stream. A non-TTY stdout silently selects InPlace,
        // so this also blocks `flox activate --sandbox prompt | tee`, which would
        // otherwise unsandbox silently. This early check only sees the CLI
        // flag; a manifest-sourced mode is re-checked after resolution in
        // `ActivateOptions::activate`.
        ensure_sandbox_not_in_place(options.sandbox.unwrap_or_default(), &invocation_type)?;

        if (invocation_type == InvocationType::Interactive
            || invocation_type == InvocationType::InPlace)
            && config.flox.upgrade_notifications.unwrap_or(true)
        {
            // Read the results of a previous upgrade check
            // and print a message if an upgrade is available.
            notify_upgrades_if_available(&flox, &mut concrete_environment, &self.environment)?;
        } else {
            debug!("Upgrade notification disabled");
        }

        // Spawn a detached process to check for upgrades in the background.
        let environment =
            UninitializedEnvironment::from_concrete_environment(&concrete_environment);
        spawn_detached_check_for_upgrades_process(
            &environment,
            None,
            &concrete_environment.log_path()?,
            None,
        )?;

        options
            .activate(
                config,
                flox,
                concrete_environment,
                invocation_type,
                Vec::new(),
            )
            .await
    }

    async fn handle_auto_activation_subcommand(
        self,
        subcommand: AutoActivate,
        config: Config,
        mut flox: Flox,
    ) -> Result<()> {
        if !flox.features.auto_activate {
            let cmd_name = match subcommand {
                AutoActivate::Allow => "allow",
                AutoActivate::Deny => "deny",
            };
            bail!(
                "'{}' requires the auto_activate feature flag. Set FLOX_FEATURES_AUTO_ACTIVATE=true.",
                cmd_name
            );
        }

        let concrete_environment = self
            .environment
            .to_concrete_environment(&mut flox, None)
            .await
            .context("Failed to find environment")?;

        let verb = match subcommand {
            AutoActivate::Allow => {
                environment_subcommand_metric!("activate::allow", concrete_environment);
                allow(&config, &concrete_environment)?;
                "allowed"
            },
            AutoActivate::Deny => {
                environment_subcommand_metric!("activate::deny", concrete_environment);
                deny(&config, &concrete_environment)?;
                "denied"
            },
        };

        let description = environment_description(&concrete_environment)?;
        message::updated(formatdoc! {"
            Auto-activation {verb} for {description}.
        "});

        Ok(())
    }
}

impl ActivateOptions {
    /// This function contains the bulk of the implementation for
    /// Activate::handle,
    /// but it allows us to create an activation for use by `services start` or
    /// `services restart`.
    ///
    /// The `services_for_ephemeral_activation` parameter specifies services to start with an
    /// ephemeral activation. If non-empty, the activation runs ephemerally (waits for output
    /// rather than exec'ing). If empty and `self.start_services` is true, all services for the
    /// current system will be started with a non-ephemeral activation.
    pub async fn activate(
        self,
        mut config: Config,
        flox: Flox,
        mut concrete_environment: ConcreteEnvironment,
        invocation_type: InvocationType,
        services_for_ephemeral_activation: Vec<String>,
    ) -> Result<()> {
        // Gate `--sandbox` behind the feature flag here rather than only in
        // `Activate::handle`, so the ephemeral activation path used by
        // `flox services start/restart` is covered too.
        if self.sandbox.is_some() && !flox.features.sandbox_activate {
            bail!(
                "'--sandbox' requires the sandbox_activate feature flag. Set FLOX_FEATURES_SANDBOX_ACTIVATE=true."
            );
        }

        let now_active = UninitializedEnvironment::from_concrete_environment(&concrete_environment);

        let lockfile = match concrete_environment.lockfile(&flox)? {
            LockResult::Changed(lockfile) => {
                message::print_overridden_manifest_fields(&lockfile);
                lockfile
            },
            LockResult::Unchanged(lockfile) => lockfile,
        };
        let manifest = &lockfile.migrated_manifest()?;

        let is_ephemeral = !services_for_ephemeral_activation.is_empty();

        let sandbox_mode = resolve_sandbox_mode(
            self.sandbox,
            manifest.as_latest_schema().options.sandbox,
            flox.features.sandbox_activate,
            is_ephemeral,
        )?;

        // A manifest-sourced mode first becomes visible here, after the
        // handle-level in-place guard (which only sees the CLI flag) has
        // already run, so re-check the rejection with the resolved mode.
        // Otherwise `options.sandbox = "prompt"` piped to `tee` would activate
        // in-place unsandboxed.
        ensure_sandbox_not_in_place(sandbox_mode, &invocation_type)?;

        // Apply the selected enforcement backend. `libsandbox` (the default) is
        // applied later as env-var injection during attach; the native-wrapper
        // backends instead re-exec this whole `flox activate` under an OS
        // sandbox and then run a vanilla activation inside it. A
        // scaffolded/planned backend fails loudly rather than silently applying
        // libsandbox under another name (which would make a benchmark
        // misattribute its results). `_FLOX_SANDBOX_WRAPPED` marks the inner
        // activation so it neither wraps again nor also applies libsandbox.
        let already_wrapped = std::env::var_os(WRAPPED_MARKER_VAR).is_some();
        let sandbox_mode = if already_wrapped {
            // Inside an OS-sandbox wrap: the wrapper enforces, so suppress the
            // libsandbox injection and run a vanilla activation.
            SandboxMode::Off
        } else if sandbox_mode == SandboxMode::Off {
            sandbox_mode
        } else {
            match resolve_sandbox_backend(
                self.sandbox_backend,
                manifest.as_latest_schema().options.sandbox_backend,
            )? {
                SandboxBackend::Libsandbox => sandbox_mode,
                SandboxBackend::HostNative => {
                    ensure_advisory_mode_supported(SandboxBackend::HostNative, sandbox_mode)?;
                    // Re-exec under the OS sandbox; never returns on success.
                    wrap_activation_host_native(&concrete_environment.dot_flox_path())?;
                    unreachable!("wrap_activation_host_native execs or errors");
                },
                SandboxBackend::Srt => {
                    ensure_advisory_mode_supported(SandboxBackend::Srt, sandbox_mode)?;
                    // Re-exec under Anthropic's sandbox-runtime; never returns.
                    wrap_activation_srt(&concrete_environment.dot_flox_path())?;
                    unreachable!("wrap_activation_srt execs or errors");
                },
                other => bail!(
                    "Sandbox backend '{other}' is not yet wired into activation.\nWired backends: 'libsandbox' (default), 'host-native', and 'srt'. Run 'flox sandbox backends' to see status, or unset FLOX_SANDBOX_BACKEND."
                ),
            }
        };

        // Services run outside the sandbox in this prototype (TH-003
        // deferred), so warn once when the environment defines any.
        if sandbox_mode != SandboxMode::Off
            && !manifest.as_latest_schema().services.inner().is_empty()
        {
            message::info(
                "Services run unsandboxed; --sandbox does not mediate their filesystem access.",
            );
        }

        // The `prompt`-mode activation banner: explain the deny-and-queue model
        // and point at the review surfaces, then surface any grant that was
        // added outside flox (the journal tamper diff). Printed before exec so
        // it lands on the user's terminal, not in the broker's nulled stdio.
        if sandbox_mode == SandboxMode::Prompt {
            print_prompt_banner(&concrete_environment.dot_flox_path());
        }

        if !self.trust
            && let Some(compose) = &lockfile.compose
        {
            for include in &compose.include {
                if let IncludeDescriptor::Remote { ref remote, .. } = include.descriptor {
                    ensure_environment_trust(
                        &mut config,
                        &flox,
                        remote,
                        true,
                        &render_composition_manifest(&include.manifest)?,
                    )
                    .await?;
                }
            }
        }

        // breadcrumb metric to estimate use of composition
        let has_includes = lockfile.compose.is_some();
        subcommand_metric!("activate", "has_includes" = has_includes);

        if let Err(err) = EventsHub::global().record_environment_activate_with(
            env_detail_from_concrete(&concrete_environment),
            |p| p.with_has_includes(has_includes),
        ) {
            debug!(error = %err, "Failed to record v2 event");
        }

        let rendered_env_path_result = concrete_environment.rendered_env_links(&flox);
        let rendered_env_path = match rendered_env_path_result {
            Err(EnvironmentError::Core(err)) if err.is_incompatible_system_error() => {
                let mut message = format!(
                    "This environment is not yet compatible with your system ({system}).",
                    system = flox.system
                );

                if let ConcreteEnvironment::Remote(remote) = &concrete_environment {
                    message.push_str("\n\n");
                    message.push_str(&format!(
                    "Use 'flox pull --force {}/{}' to update and verify this environment on your system.",
                    remote.owner(),
                    remote.name()));
                }

                bail!("{message}")
            },
            other => other?,
        };

        // Must not be evaluated inline with the macro or we'll leak TRACE logs
        // for reasons unknown.
        let lockfile_version = lockfile.version();
        subcommand_metric!("activate#version", lockfile_version = lockfile_version);

        // The new pipeline drops the legacy `activate#version` pseudo-
        // subcommand (per spec AC #4) and rides `lockfile_version` on a
        // real `cli.environment.activate` event instead.
        if let Err(err) = EventsHub::global().record_environment_activate_with(
            env_detail_from_concrete(&concrete_environment),
            |p| p.with_lockfile_version(lockfile_version.to_string()),
        ) {
            debug!(error = %err, "Failed to record v2 event");
        }

        let mode = self.mode.clone().unwrap_or(
            manifest
                .as_latest_schema()
                .options
                .activate
                .mode
                .clone()
                .unwrap_or_default(),
        );
        let mode_link_path = rendered_env_path.clone().for_mode(&mode);
        let store_path = fs::read_link(&mode_link_path).with_context(|| {
            format!(
                "a symlink at {} was just created and should still exist",
                mode_link_path.display()
            )
        })?;

        let interpreter_path = {
            let path = FLOX_INTERPRETER.clone();
            tracing::debug!(
                interpreter = "bundled",
                path = traceable_path(&path),
                "setting interpreter"
            );
            path
        };

        // read the currently active environments from the environment
        let mut flox_active_environments = activated_environments();

        // Detect if the current environment is already active
        let already_active = flox_active_environments.is_active(&now_active);
        if already_active {
            debug!(
                "Environment is already active: environment={}. Not adding to active environments",
                now_active.bare_description()
            );
            if invocation_type == InvocationType::Interactive {
                return Err(anyhow!(
                    "Environment {} is already active",
                    uninitialized_environment_description(&now_active)?
                ));
            }
        } else {
            // Add to _FLOX_ACTIVE_ENVIRONMENTS so we can detect what environments are active.
            flox_active_environments.set_last_active(
                now_active.clone(),
                self.generation,
                mode.clone(),
            );
        };

        // Determine values for `set_prompt` and `hide_default_prompt`, taking
        // deprecated `shell_prompt` into account
        let (set_prompt, hide_default_prompt) = match (
            config.flox.set_prompt,
            config.flox.hide_default_prompt,
            &config.flox.shell_prompt,
        ) {
            (None, None, Some(EnvironmentPromptConfig::ShowAll)) => (true, false),
            (None, None, Some(EnvironmentPromptConfig::HideDefault)) => (true, true),
            (None, None, Some(EnvironmentPromptConfig::HideAll)) => (false, false),
            (Some(_), _, Some(_)) | (_, Some(_), Some(_)) => bail!(indoc! {"
                'shell_prompt' has been deprecated and cannot be set when 'set_prompt' or
                'hide_default_prompt' is set.

                Remove 'shell_prompt' with 'flox config --delete shell_prompt'
            "}),
            (set_prompt, hide_default_prompt, _) => (
                set_prompt.unwrap_or(true),
                hide_default_prompt.unwrap_or(true),
            ),
        };

        // We don't have access to the current PS1 (it's not exported), so we
        // can't modify it. Instead set FLOX_PROMPT_ENVIRONMENTS and let the
        // activation script set PS1 based on that.
        let flox_prompt_environments =
            Self::make_prompt_environments(hide_default_prompt, &flox_active_environments);

        let prompt_color_1 = env::var("FLOX_PROMPT_COLOR_1")
            .unwrap_or(utils::colors::INDIGO_400.to_ansi256().to_string());
        let prompt_color_2 = env::var("FLOX_PROMPT_COLOR_2")
            .unwrap_or(utils::colors::INDIGO_300.to_ansi256().to_string());

        let socket_path = concrete_environment.services_socket_path(&flox)?;

        let flox_env_cuda_detection = match manifest.as_latest_schema().options.cuda_detection {
            Some(false) => "0", // manifest opts-out
            _ => "1",           // default to enabling CUDA
        };

        // Determine services to start with a new process-compose
        let services_to_start = if is_ephemeral {
            services_for_ephemeral_activation
        } else {
            self.services_to_start(manifest, &flox.system, &socket_path)
        };
        debug!(
            is_ephemeral,
            ?services_to_start,
            "setting service variables"
        );

        let shell = if invocation_type == InvocationType::InPlace {
            detect_shell_for_in_place()?
        } else {
            detect_shell_for_subshell()
        };
        subcommand_metric!("activate", "shell" = shell.to_string());

        // Runs before `command.exec()`, so the buffered event is flushed
        // synchronously by the pre-exec emit + flush block below
        // (spec AC #5).
        if let Err(err) = EventsHub::global().record_environment_activate_with(
            env_detail_from_concrete(&concrete_environment),
            |p| p.with_shell(shell.to_string()),
        ) {
            debug!(error = %err, "Failed to record v2 event");
        }

        let core = AttachCtx {
            // Don't rely on FLOX_ENV in the environment when we explicitly know
            // what it should be. This is necessary for nested activations where an
            // outer export of FLOX_ENV would be inherited by the inner activation.
            env: mode_link_path.to_string_lossy().to_string(),
            env_cache: concrete_environment.cache_path()?.into_inner(),
            env_description: now_active.bare_description(),
            flox_active_environments: flox_active_environments.to_string(),
            prompt_color_1,
            prompt_color_2,
            flox_prompt_environments,
            set_prompt,
            flox_env_cuda_detection: flox_env_cuda_detection.to_string(),
            interpreter_path,
            sandbox_mode,
            // Seeded into the sandbox network policy as a visible
            // default-seed grant; absent when the user disabled metrics, so
            // nothing is granted for traffic that will never happen.
            metrics_host: if config.flox.disable_metrics {
                None
            } else {
                metrics_events_host()
            },
        };

        let dot_flox_path = concrete_environment.dot_flox_path().to_path_buf();

        let project = AttachProjectCtx {
            env_project: concrete_environment.project_path()?,
            dot_flox_path: dot_flox_path.clone(),
            flox_env_log_dir: concrete_environment.log_path()?.to_path_buf(),
            flox_services_socket: socket_path,
            process_compose_bin: PathBuf::from(&*PROCESS_COMPOSE_BIN),
            services_to_start,
        };

        let activation_state_dir = activation_state_dir_path(&flox.runtime_dir, &dot_flox_path);

        let activate_data = ActivateCtx {
            flox_activate_store_path: store_path.to_string_lossy().to_string(),
            attach_ctx: core,
            project_ctx: Some(project),
            activation_state_dir,
            mode,
            shell,
            invocation_type: Some(invocation_type),
            remove_after_reading: true,
            metrics_uuid: flox.metrics_device_uuid,
            disable_hook: config.flox.disable_hook.unwrap_or(false),
            flox_bin: std::env::current_exe()
                .ok()
                .and_then(|p| p.to_str().map(String::from))
                .unwrap_or_else(|| "flox".to_string()),
            auto_activate_fish_mode: config.flox.auto_activate_fish_mode,
            sandbox_mode,
        };

        let tempfile = tempfile::NamedTempFile::new_in(flox.temp_dir)?;

        let writer = BufWriter::new(&tempfile);
        serde_json::to_writer_pretty(writer, &activate_data)?;
        let (_, tempfile) = tempfile.keep()?;

        // `flox-activations` doesn't really have a "quiet" mode, so it makes
        // more sense for 0 to be the default rather than 1.
        let verbosity_num = flox.verbosity.max(0) as u32;
        let mut command = std::process::Command::new(&*FLOX_ACTIVATIONS_BIN);
        command
            .env(FLOX_ACTIVATIONS_VERBOSITY_VAR, format!("{verbosity_num}"))
            .arg("activate")
            .arg("--activate-data")
            .arg(tempfile);

        if is_ephemeral {
            debug!("running ephemeral activation command: {:?}", command);
            let output = command
                .stderr(Stdio::piped())
                .stdout(Stdio::piped())
                .output()?;
            if !output.status.success() {
                // flox-activations formats its own errors
                // We might be able to just use Stdio::inherit above but I'm not
                // 100% flox-activations will only print in the error case
                eprint!("{}", String::from_utf8_lossy(&output.stderr));
                Err(Exit(1.into()))?;
            }
            trace!(
                "ephemeral activation stderr:\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
            Ok(())
        } else {
            debug!("running activation command: {:?}", command);
            // `command.exec()` replaces this process, so the dispatcher's
            // end-of-`cli_worker` `command_completed` emit will never run.
            // Record + flush the v2 event synchronously here so the
            // invocation is closed out before control passes to the user's
            // shell. The hub's idempotent flag turns the dispatcher's emit
            // into a no-op if `exec` returns an error and the failure
            // propagates back to `cli_worker`.
            let hub = flox_events::EventsHub::global();
            if let Err(err) = hub.record_command_completed("activate".to_string()) {
                debug!(
                    error = %err,
                    "Failed to record v2 cli.command_completed event before exec"
                );
            }
            if let Err(err) = hub.flush(true) {
                debug!(
                    error = %err,
                    "Failed to flush v2 events before exec"
                );
            }
            // exec should never return
            // TODO: did this break in-place metrics?
            Err(command.exec().into())
        }
    }

    /// Determine which services to start on activation.
    ///
    /// Services are started when `--start-services` is set or when the manifest
    /// has `[services] auto-start = true` and `--no-start-services` is not set.
    ///
    /// Returns an empty list (with a warning) if:
    /// - Neither flag nor manifest requests service startup
    /// - No services are defined in the manifest
    /// - No services are defined for the current system
    /// - Services are already running
    fn services_to_start(
        &self,
        manifest: &Manifest<MigratedTypedOnly>,
        system: &System,
        socket_path: &Path,
    ) -> Vec<String> {
        let manifest_services = &manifest.as_latest_schema().services;
        let auto_start = manifest_services.auto_start == Some(true);

        let should_start = self.start_services || (auto_start && !self.no_start_services);
        if !should_start {
            return Vec::new();
        }

        // Only emit warnings for conditions the user can act on when they
        // explicitly requested service startup via `--start-services`. When
        // auto-start triggers silently on every activation, these messages
        // would be noisy and surprising.
        let warn = self.start_services;

        if manifest_services.inner().is_empty() {
            message::warning(ServicesCommandsError::NoDefinedServices);
            return Vec::new();
        }

        let services_for_system = manifest_services.copy_for_system(system);
        if services_for_system.inner().is_empty() {
            if warn {
                message::warning(ServicesCommandsError::NoDefinedServicesForSystem {
                    system: system.clone(),
                });
            }
            return Vec::new();
        }

        let has_running_services = ProcessStates::read(socket_path)
            .map(|states| states.iter().any(|p| p.is_running))
            .unwrap_or(false);

        if has_running_services {
            if warn {
                message::warning("Skipped starting services, services are already running");
            }
            return Vec::new();
        }

        services_for_system.inner().keys().cloned().collect()
    }

    /// Construct the environment list for the shell prompt
    ///
    /// [`None`] if the prompt is disabled, or filters removed all components.
    fn make_prompt_environments(
        hide_default_prompt: bool,
        flox_active_environments: &super::ActiveEnvironments,
    ) -> String {
        let prompt_envs: Vec<_> = flox_active_environments
            .iter()
            .filter_map(|env| {
                if hide_default_prompt && env.name().as_ref() == DEFAULT_NAME {
                    return None;
                }
                Some(env.bare_description())
            })
            .collect();

        prompt_envs.join(" ")
    }
}

/// Notify the user of available upgrades
///
/// Upon activation flox will start a detached process to check for upgrades.
/// Future activations will be able to read the upgrade information from a file
/// and notify the user if there are any upgrades available using this function.
/// See [spawn_detached_check_for_upgrades_process] for more information
/// on the upgrade check process.
///
/// This function reads the upgrade information for a given environment,
/// and prints a message to the user if the upgrade information is still applicable
/// to the current environment -- based on the same lockfile
/// and indicating that upgrades are available -- and the environment isn't
/// already active, to prevent immediate duplications.
///
/// There is no refractory period for upgrade notifications,
/// i.e. we message _every_ time this function is called by `flox activate`.
/// The motivation for this is to provide deterministic behavior,
/// compared to comparatively random display of upgrade messages every hour or so.
/// For example, when a user activates an environment and sees a message,
/// but doesn't act on it, they should see the message again next time they activate,
/// so they are not wondering whether upgrades may have been applied automatically.
/// To make this less annoying, we tried to make the message as unobtrusive as possible.
fn notify_upgrades_if_available(
    flox: &Flox,
    environment: &mut ConcreteEnvironment,
    environment_select: &EnvironmentSelect,
) -> Result<()> {
    let current_environment = UninitializedEnvironment::from_concrete_environment(environment);
    let active_environments = activated_environments();
    if active_environments.is_active(&current_environment) {
        debug!("Not notifying user of upgrade, environment is already active");
        return Ok(());
    }

    // print a possible notification about environment upgrades first
    // (and avoid being skipped below because upgrade information is unavailable)
    notify_environment_upgrades(environment, environment_select)?;

    let upgrade_guard = UpgradeInformationGuard::read_in(environment.cache_path()?)?;

    let Some(info) = upgrade_guard.info() else {
        debug!("Not notifying user of upgrade, no upgrade information available");
        return Ok(());
    };

    notify_package_upgrades(flox, environment, &info.upgrade_result, environment_select)?;

    Ok(())
}

fn notify_package_upgrades(
    flox: &Flox,
    environment: &mut ConcreteEnvironment,
    upgrade_result: &UpgradeResult,
    environment_select: &EnvironmentSelect,
) -> Result<()> {
    let current_lockfile = environment.lockfile(flox)?.into();
    if Some(current_lockfile) != upgrade_result.old_lockfile {
        // todo: delete the info file?
        debug!("Not notifying user of upgrade, lockfile has changed since last check");
        return Ok(());
    }
    let diff = upgrade_result.diff();
    if diff.is_empty() {
        debug!("Not notifying user of upgrade, no changes in lockfile");
        return Ok(());
    }
    let description = environment_description(environment)?;
    let diff_for_system = upgrade_result.diff_for_system(&flox.system);
    if diff_for_system.is_empty() {
        message::verbose(formatdoc! {"
            Upgrades available for {description} on other systems.
            Use 'flox upgrade --dry-run' for details."});
        return Ok(());
    }
    // TODO: this doesn't capture the environment chosen by the user if we prompted
    let flags = environment_select
        .to_flags()
        .map(|flags| format!(" {}", flags.join(" ")))
        .unwrap_or("".to_string());
    let (version_changes, rebuilds) = count_upgrade_categories(&diff_for_system);
    let summary = format_upgrade_summary(version_changes, rebuilds);
    let message = formatdoc! {"
        {summary} available in {description}.
        Use 'flox upgrade --dry-run{flags}' for details.
    "};
    message::info(message);
    Ok(())
}

/// For remote environments only; check whether the environment state is equal
/// to our most recent view of the remote state on FloxHub.
/// This method itself **won't** query FloxHub but depends on side effects
/// of other operations (e.g. push, pull, * --upstream,
/// the async fetch of a previous activation) to avoid delays of activations,
/// or failures due to network disruptions.
fn notify_environment_upgrades(
    environment: &ConcreteEnvironment,
    environment_select: &EnvironmentSelect,
) -> Result<()> {
    if let ConcreteEnvironment::Path(_) = environment {
        debug!("Not notifying user of environment upgrades for local path environments");
        return Ok(());
    }

    let branch_ord = match environment {
        ConcreteEnvironment::Path(_) => unreachable!(),
        ConcreteEnvironment::Managed(managed_environment) => {
            managed_environment.compare_remote()?
        },
        ConcreteEnvironment::Remote(remote_environment) => remote_environment.compare_remote()?,
    };

    if branch_ord == BranchOrd::Equal {
        debug!("Not notifying user of environment upgrades, equal branches");
        return Ok(());
    }

    let (local_generations_metadata, remote_generations_metadata) = match environment {
        ConcreteEnvironment::Path(_) => unreachable!(),
        ConcreteEnvironment::Managed(managed_environment) => (
            managed_environment.generations_metadata(),
            managed_environment.remote_generations_metadata(),
        ),
        ConcreteEnvironment::Remote(remote_environment) => (
            remote_environment.generations_metadata(),
            remote_environment.remote_generations_metadata(),
        ),
    };

    let local_generations_metadata = match local_generations_metadata {
        Ok(metadata) => metadata.into_inner(),
        Err(error) => {
            warn!(%error, "Not notifying user of environment upgrades, could not get local state");
            return Ok(());
        },
    };

    let remote_generations_metadata = match remote_generations_metadata {
        Ok(metadata) => metadata.into_inner(),
        Err(error) => {
            warn!(%error, "Not notifying user of environment upgrades, could not get remote state");
            return Ok(());
        },
    };

    let branch_ord_description = match branch_ord {
        BranchOrd::Equal => unreachable!(),
        BranchOrd::Ahead => "ahead of",
        BranchOrd::Behind => "behind",
        BranchOrd::Diverged => "diverged from",
    };

    let history_peek = format_diverged_metadata(&DivergedMetadata {
        local: local_generations_metadata,
        remote: remote_generations_metadata.to_owned(),
    });

    // TODO: this doesn't capture the environment chosen by the user if we prompted
    let flags = environment_select
        .to_flags()
        .map(|flags| format!(" {}", flags.join(" ")))
        .unwrap_or("".to_string());

    let compensation_description = match branch_ord {
        BranchOrd::Equal => unreachable!(),
        BranchOrd::Ahead => format!("Use 'flox push{flags}' to update the environment on FloxHub."),
        BranchOrd::Behind => {
            format!("Use 'flox pull{flags}' to fetch updates from FloxHub.")
        },
        BranchOrd::Diverged => {
            format!(
                "Use 'flox pull|push --force{flags}' to fetch updates or update the environment on FloxHub."
            )
        },
    };

    let message = formatdoc! {"
        Local environment state is {branch_ord_description} FloxHub.

        {history_peek}

        {compensation_description}
    "};

    message::info(message);

    Ok(())
}

/// Allow auto-activation for an environment by updating the config.
///
/// Writes the allow preference to the config file for the environment's parent
/// path.
pub fn allow(config: &Config, concrete_environment: &ConcreteEnvironment) -> Result<()> {
    set_auto_activation_preference(
        config,
        concrete_environment,
        AutoActivationPreference::Allow,
    )
}

/// Deny auto-activation for an environment by updating the config.
///
/// Writes the deny preference to the config file for the environment's parent
/// path.
pub fn deny(config: &Config, concrete_environment: &ConcreteEnvironment) -> Result<()> {
    set_auto_activation_preference(config, concrete_environment, AutoActivationPreference::Deny)
}

/// Record the user's per-directory auto-activation preference under
/// `auto_activate_environments` in the config.
fn set_auto_activation_preference(
    config: &Config,
    concrete_environment: &ConcreteEnvironment,
    preference: AutoActivationPreference,
) -> Result<()> {
    let env_path = concrete_environment.parent_path()?;
    write_auto_activation_preference(&config.flox.config_dir, &env_path, preference)
}

/// Write an auto-activation preference for a project directory (the directory
/// containing `.flox`) to the config under `auto_activate_environments`.
///
/// The directory is written as a single literal TOML key rather than spliced
/// into a dot-separated key string: a path can contain `.` (macOS temp dirs
/// live under paths like `/var/folders/...`, and project directories may have
/// names like `my.app`), which dotted-key parsing would shatter into several
/// nested tables.
///
/// `env_path` must be canonical so it matches the directories the prompt hook
/// discovers. Subcommand callers get this from
/// [`Environment::parent_path`] (a popped `CanonicalPath`); the prompt hook
/// passes the already-canonical discovered directory.
pub fn write_auto_activation_preference(
    config_dir: &Path,
    env_path: &Path,
    preference: AutoActivationPreference,
) -> Result<()> {
    let query = [
        Key::new("auto_activate_environments"),
        Key::new(env_path.to_string_lossy().into_owned()),
    ];
    update_config_with_query(config_dir, &query, Some(preference))?;
    Ok(())
}

/// Environment marker set on the re-exec'd inner activation so it neither wraps
/// again nor also applies libsandbox while running inside an OS-sandbox wrap.
const WRAPPED_MARKER_VAR: &str = "_FLOX_SANDBOX_WRAPPED";

/// Resolve the sandbox backend, in precedence order: the `--sandbox-backend`
/// flag, the `FLOX_SANDBOX_BACKEND` environment variable, the manifest
/// `options.sandbox-backend`, then the default (`libsandbox`).
fn resolve_sandbox_backend(
    flag: Option<SandboxBackend>,
    manifest: Option<SandboxBackend>,
) -> Result<SandboxBackend> {
    if let Some(backend) = flag {
        return Ok(backend);
    }
    if let Some(value) = std::env::var_os(FLOX_SANDBOX_BACKEND_VAR) {
        let value = value.to_string_lossy();
        if !value.is_empty() {
            return value.parse().map_err(|err| anyhow::anyhow!("{err}"));
        }
    }
    Ok(manifest.unwrap_or_default())
}

/// `warn` and `prompt` are advisory semantics that only `libsandbox` implements
/// — observe-but-allow, and deny-then-live-redeem through the broker. A
/// kernel/container/hypervisor backend can only allow or deny, so reject those
/// modes with a clear message rather than silently enforcing a mode the user
/// did not ask for (the failure mode where `--sandbox warn` locks down the
/// home directory anyway).
fn ensure_advisory_mode_supported(backend: SandboxBackend, mode: SandboxMode) -> Result<()> {
    if backend.capabilities().enforces && matches!(mode, SandboxMode::Warn | SandboxMode::Prompt) {
        bail!(
            "Sandbox backend '{backend}' enforces; it has no advisory '{mode}' mode.\nUse '--sandbox enforce' with this backend, or '--sandbox-backend libsandbox' for advisory '{mode}'."
        );
    }
    Ok(())
}

/// Re-exec the current `flox activate` invocation under the host-native OS
/// sandbox, then never return (the inner activation runs confined). Returns the
/// error on a failure to launch, or on an unsupported platform.
///
/// macOS uses `sandbox-exec` with a generated SBPL profile that is permissive
/// at the base (`allow default`, so flox and the user's tools run) but denies
/// reading the *contents* of, and writing to, the user's entire home directory
/// — the agent's primary secret/data surface — except the project being
/// activated and Flox's own state. Metadata (stat/traverse) stays allowed so
/// paths into the re-allowed subdirs resolve; reads outside `$HOME` (system,
/// Nix) are untouched. Enforcement holds even against SIP-protected system
/// binaries that bypass advisory libsandbox. The Linux (bubblewrap + Landlock)
/// path is not yet wired.
fn wrap_activation_host_native(dot_flox_path: &Path) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::process::CommandExt;

        let home = std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
            anyhow::anyhow!("HOME is not set; cannot build the host-native sandbox profile.")
        })?;
        // SBPL matches the realpath, so canonicalize (/var -> /private/var).
        let home = std::fs::canonicalize(&home).unwrap_or(home);
        // Re-allow the project root (the parent of its `.flox`) so the agent can
        // read and write the code it is working on.
        let dot_flox =
            std::fs::canonicalize(dot_flox_path).unwrap_or_else(|_| dot_flox_path.to_path_buf());
        let project = dot_flox.parent().unwrap_or(&dot_flox).to_path_buf();
        let profile = host_native_profile(&home, &project);

        let flox_exe = std::env::current_exe()
            .map_err(|err| anyhow::anyhow!("Cannot locate the flox binary to re-exec: {err}"))?;
        let inner_args: Vec<String> = std::env::args().skip(1).collect();

        // `exec` only returns on failure.
        let err = std::process::Command::new("sandbox-exec")
            .arg("-p")
            .arg(&profile)
            .arg(&flox_exe)
            .args(&inner_args)
            .env(WRAPPED_MARKER_VAR, "1")
            .exec();
        Err(anyhow::anyhow!(
            "Failed to launch the host-native sandbox: {err}. 'sandbox-exec' must be available on macOS."
        ))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = dot_flox_path;
        bail!(
            "The 'host-native' sandbox backend is only wired on macOS (sandbox-exec).\nThe Linux backend (bubblewrap + Landlock) is not yet implemented. Unset FLOX_SANDBOX_BACKEND or use '--sandbox-backend libsandbox'."
        );
    }
}

/// The macOS `sandbox-exec` (SBPL) profile for a host-native activation. Paths
/// are canonical because SBPL matches the realpath.
///
/// Deny-by-default for the user's home: the contents of every file under
/// `$HOME` are unreadable and unwritable except the project and Flox's own
/// state. `file-read-metadata` is left to `allow default` so path resolution
/// into the allowed subdirs works. `.env` files stay secret even inside the
/// project (the last matching rule wins).
#[cfg(target_os = "macos")]
fn host_native_profile(home: &Path, project: &Path) -> String {
    let h = home.display();
    let p = project.display();
    format!(
        r##"(version 1)
(allow default)
; Deny reading the contents of, and writing to, the user's home — the agent's
; primary secret/data surface. Metadata (stat/traverse) stays allowed via
; `allow default`, and reads outside $HOME (system, Nix) are untouched, so flox
; and tools still run.
(deny file-read-data file-write* (subpath "{h}"))
(allow file-read-data file-write*
  (subpath "{p}")
  (subpath "{h}/.cache")
  (subpath "{h}/.config")
  (subpath "{h}/.local")
  (subpath "{h}/Library/Application Support/flox")
  (subpath "{h}/Library/Caches/flox"))
; Keep .env files secret even inside the project.
(deny file-read-data file-write* (regex #"/\.env(\.[^/]*)?$"))
"##
    )
}

/// `true` if an executable named `name` is on `PATH`.
fn binary_on_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(name).is_file()))
}

/// Re-exec the current `flox activate` under Anthropic's sandbox-runtime
/// (`srt`), then never return. `srt` drives `sandbox-exec` on macOS and
/// `bubblewrap` on Linux and adds proxy-based network egress control, so it
/// brings the same kernel boundary as `host-native` packaged as an installable
/// tool — and, unlike host-native's bespoke profile, it is wired on both
/// platforms.
///
/// Generates an srt settings file mirroring the host-native deny-`$HOME` policy
/// (srt reads are deny-then-allow, so deny `$HOME` and re-allow only the
/// project and Flox's state) and runs `srt -s <settings> <flox> activate
/// <args>` with `_FLOX_SANDBOX_WRAPPED=1` (srt preserves the environment).
/// Network egress is default-deny — srt's secure-by-default — which
/// `host-native` does not enforce.
fn wrap_activation_srt(dot_flox_path: &Path) -> Result<()> {
    use std::os::unix::process::CommandExt;

    if !binary_on_path("srt") {
        bail!(
            "The 'srt' sandbox backend requires Anthropic's sandbox-runtime, which was not found on PATH.\nInstall it (e.g. 'flox install sandbox-runtime' or 'npm install -g @anthropic-ai/sandbox-runtime'), or use '--sandbox-backend host-native'."
        );
    }

    let home = std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
        anyhow::anyhow!("HOME is not set; cannot build the srt sandbox settings.")
    })?;
    let home = std::fs::canonicalize(&home).unwrap_or(home);
    let dot_flox =
        std::fs::canonicalize(dot_flox_path).unwrap_or_else(|_| dot_flox_path.to_path_buf());
    let project = dot_flox.parent().unwrap_or(&dot_flox).to_path_buf();

    let settings = srt_settings_json(&home, &project);
    // The settings file must outlive this process (exec replaces us), so it is
    // written to a temp file that is intentionally not cleaned up.
    let settings_path = std::env::temp_dir().join(format!("flox-srt-{}.json", std::process::id()));
    std::fs::write(&settings_path, settings)
        .map_err(|err| anyhow::anyhow!("Failed to write the srt settings file: {err}"))?;

    let flox_exe = std::env::current_exe()
        .map_err(|err| anyhow::anyhow!("Cannot locate the flox binary to re-exec: {err}"))?;
    let inner_args: Vec<String> = std::env::args().skip(1).collect();

    // `exec` only returns on failure.
    let err = std::process::Command::new("srt")
        .arg("-s")
        .arg(&settings_path)
        .arg(&flox_exe)
        .args(&inner_args)
        .env(WRAPPED_MARKER_VAR, "1")
        .exec();
    Err(anyhow::anyhow!("Failed to launch srt: {err}."))
}

/// The srt settings JSON for a host-native-equivalent activation policy.
///
/// srt reads are deny-then-allow (`allowRead` wins), so denying `$HOME` and
/// re-allowing only the project and Flox's dirs leaves arbitrary home files
/// (e.g. `~/.ssh`) unreadable. Writes are allow-only. Network is default-deny
/// (empty allowlist) — the egress control `host-native` lacks.
fn srt_settings_json(home: &Path, project: &Path) -> String {
    let h = home.display().to_string();
    let p = project.display().to_string();
    let cache = format!("{h}/.cache");
    let config = format!("{h}/.config");
    let local = format!("{h}/.local");
    let settings = serde_json::json!({
        "filesystem": {
            "denyRead": [&h],
            "allowRead": [&p, &cache, &config, &local],
            "allowWrite": [&p, &cache, &config, &local, "/tmp", "/private/tmp"],
            "denyWrite": [],
        },
        "network": {
            "allowedDomains": [],
            "deniedDomains": [],
            // The activation talks to the Nix daemon and binds Flox's own run
            // sockets; allow those unix-socket dirs (subpath-matched) while
            // keeping TCP egress default-deny.
            "allowUnixSockets": ["/nix/var/nix/daemon-socket", &cache],
        },
    });
    serde_json::to_string_pretty(&settings).expect("serializing a literal JSON value cannot fail")
}

/// Resolve the effective sandbox mode for an activation.
///
/// Precedence: CLI flag > manifest `options.sandbox` > off.
///
/// An explicit CLI flag without the sandbox_activate feature flag is a hard
/// error so the user knows the flag did not take effect. A manifest-sourced
/// mode is gentler — shared manifests must not hard-fail consumers — so it
/// is ignored for ephemeral (service) activations with a debug note, and
/// downgraded to off with a warning when the feature flag is absent.
fn resolve_sandbox_mode(
    cli_mode: Option<SandboxMode>,
    manifest_mode: Option<SandboxMode>,
    sandbox_feature_enabled: bool,
    is_ephemeral: bool,
) -> Result<SandboxMode> {
    if let Some(mode) = cli_mode {
        // Normally already rejected by the gate at the top of
        // `ActivateOptions::activate`; repeated here so resolution is
        // self-contained.
        if !sandbox_feature_enabled {
            bail!(
                "'--sandbox' requires the sandbox_activate feature flag. Set FLOX_FEATURES_SANDBOX_ACTIVATE=true."
            );
        }
        return Ok(mode);
    }

    let Some(mode) = manifest_mode else {
        return Ok(SandboxMode::Off);
    };

    if is_ephemeral {
        debug!("ignoring manifest 'options.sandbox' for an ephemeral (service) activation");
        return Ok(SandboxMode::Off);
    }

    if !sandbox_feature_enabled {
        message::warning(
            "Ignoring 'options.sandbox' from the manifest; sandboxing requires the sandbox_activate feature flag. Set FLOX_FEATURES_SANDBOX_ACTIVATE=true.",
        );
        return Ok(SandboxMode::Off);
    }

    Ok(mode)
}

/// Reject a sandboxed in-place activation.
///
/// A non-TTY stdout silently selects [InvocationType::InPlace], so without
/// this guard `flox activate --sandbox prompt | tee` (or a manifest-set mode)
/// would activate unsandboxed without any indication.
fn ensure_sandbox_not_in_place(
    sandbox_mode: SandboxMode,
    invocation_type: &InvocationType,
) -> Result<()> {
    if sandbox_mode != SandboxMode::Off && *invocation_type == InvocationType::InPlace {
        bail!(
            "--sandbox requires an interactive shell or a command ('flox activate --sandbox prompt -- <cmd>'); in-place activation cannot be sandboxed."
        );
    }
    Ok(())
}

/// Print the `prompt`-mode activation banner and any journal tamper warning.
///
/// The banner explains the deny-and-queue model and names the review surfaces.
/// The tamper check reads grants.toml and the journal, and warns when a grant
/// is present in the file but absent from the journal — it was added outside
/// flox (a hand-edit or a self-approving agent), which is friction-plus-audit,
/// not enforcement (the journal is provenance, never policy).
fn print_prompt_banner(dot_flox_path: &Path) {
    message::info(
        "Sandbox 'prompt' enabled (advisory; mediates file reads/writes).\n  \
         Out-of-policy access is denied and queued for approval.\n    \
         review queue:   flox sandbox\n    \
         approve a path: flox sandbox allow '<glob>'   (second terminal)",
    );

    let grants_dir = dot_flox_path.join("cache").join("sandbox");
    let unjournaled = grants::unjournaled_patterns(&grants_dir);
    if unjournaled.is_empty() {
        return;
    }
    let count = unjournaled.len();
    let listed = unjournaled
        .iter()
        .map(|pattern| format!("    {pattern}"))
        .collect::<Vec<_>>()
        .join("\n");
    message::warning(format!(
        "{count} grant(s) added outside flox — possibly self-approved:\n{listed}\n  \
         Keep them if intentional, or remove with: flox sandbox revoke '<glob>'"
    ));
}

#[cfg(test)]
mod tests {
    use std::sync::LazyLock;

    use flox_rust_sdk::models::environment::{DotFlox, EnvironmentPointer, PathPointer};
    use flox_rust_sdk::utils::logging::test_helpers::test_subscriber_message_only;

    use super::*;
    use crate::commands::ActiveEnvironments;

    static DEFAULT_ENV: LazyLock<UninitializedEnvironment> = LazyLock::new(|| {
        UninitializedEnvironment::DotFlox(DotFlox {
            path: PathBuf::from(""),
            pointer: EnvironmentPointer::Path(PathPointer::new("default".parse().unwrap())),
        })
    });

    static NON_DEFAULT_ENV: LazyLock<UninitializedEnvironment> = LazyLock::new(|| {
        UninitializedEnvironment::DotFlox(DotFlox {
            path: PathBuf::from(""),
            pointer: EnvironmentPointer::Path(PathPointer::new("wichtig".parse().unwrap())),
        })
    });

    #[test]
    fn test_shell_prompt_empty_without_active_environments() {
        let active_environments = ActiveEnvironments::default();
        let prompt = ActivateOptions::make_prompt_environments(false, &active_environments);

        assert_eq!(prompt, "");
    }

    #[test]
    fn test_shell_prompt_default() {
        let mut active_environments = ActiveEnvironments::default();
        active_environments.set_last_active(DEFAULT_ENV.clone(), None, ActivateMode::Dev);

        // with `hide_default_prompt = false` we should see the default environment
        let prompt = ActivateOptions::make_prompt_environments(false, &active_environments);
        assert_eq!(prompt, "default".to_string());

        // with `hide_default_prompt = true` we should not see the default environment
        let prompt = ActivateOptions::make_prompt_environments(true, &active_environments);
        assert_eq!(prompt, "");
    }

    #[test]
    fn test_shell_prompt_mixed() {
        let mut active_environments = ActiveEnvironments::default();
        active_environments.set_last_active(DEFAULT_ENV.clone(), None, ActivateMode::Dev);
        active_environments.set_last_active(NON_DEFAULT_ENV.clone(), None, ActivateMode::Dev);

        // with `hide_default_prompt = false` we should see the default environment
        let prompt = ActivateOptions::make_prompt_environments(false, &active_environments);
        assert_eq!(prompt, "wichtig default".to_string());

        // with `hide_default_prompt = true` we should not see the default environment
        let prompt = ActivateOptions::make_prompt_environments(true, &active_environments);
        assert_eq!(prompt, "wichtig".to_string());
    }

    /// Build minimal ActivateOptions with only the service-related flags set.
    fn activate_options_with_flags(
        start_services: bool,
        no_start_services: bool,
    ) -> ActivateOptions {
        ActivateOptions {
            trust: false,
            print_script: false,
            start_services,
            no_start_services,
            mode: None,
            sandbox: None,
            sandbox_backend: None,
            generation: None,
            command: None,
        }
    }

    #[test]
    fn test_conflicting_service_flags_are_rejected() {
        let options = activate_options_with_flags(true, true);
        assert!(options.validate_service_flags().is_err());
    }

    /// Run the [ActivateOptions] parser against a synthetic command line.
    fn parse_activate_options(args: &[&str]) -> Result<ActivateOptions, bpaf::ParseFailure> {
        activate_options().to_options().run_inner(args)
    }

    #[test]
    fn sandbox_flag_absent_parses_as_none() {
        let options = parse_activate_options(&[]).unwrap();
        assert_eq!(options.sandbox, None);
    }

    #[test]
    fn bare_sandbox_flag_parses_as_prompt() {
        let options = parse_activate_options(&["--sandbox"]).unwrap();
        assert_eq!(options.sandbox, Some(SandboxMode::Prompt));
    }

    #[test]
    fn sandbox_flag_with_value_parses_explicit_mode() {
        let options = parse_activate_options(&["--sandbox", "warn"]).unwrap();
        assert_eq!(options.sandbox, Some(SandboxMode::Warn));

        let options = parse_activate_options(&["--sandbox=enforce"]).unwrap();
        assert_eq!(options.sandbox, Some(SandboxMode::Enforce));

        let options = parse_activate_options(&["--sandbox", "off"]).unwrap();
        assert_eq!(options.sandbox, Some(SandboxMode::Off));
    }

    #[test]
    fn bare_sandbox_flag_parses_before_trailing_command() {
        let options = parse_activate_options(&["--sandbox", "--", "true"]).unwrap();
        assert_eq!(options.sandbox, Some(SandboxMode::Prompt));
        let Some(CommandSelect::ExecCommand { command, args }) = options.command else {
            panic!("expected an exec command");
        };
        assert_eq!(command, "true");
        assert_eq!(args, Vec::<String>::new());
    }

    #[test]
    fn valued_sandbox_flag_parses_before_trailing_command() {
        let options = parse_activate_options(&["--sandbox", "enforce", "--", "true"]).unwrap();
        assert_eq!(options.sandbox, Some(SandboxMode::Enforce));
        let Some(CommandSelect::ExecCommand { command, .. }) = options.command else {
            panic!("expected an exec command");
        };
        assert_eq!(command, "true");
    }

    #[test]
    fn sandbox_flag_rejects_invalid_mode() {
        assert!(parse_activate_options(&["--sandbox", "bogus"]).is_err());
    }

    #[test]
    fn resolve_sandbox_mode_cli_flag_wins_over_manifest() {
        let mode = resolve_sandbox_mode(
            Some(SandboxMode::Warn),
            Some(SandboxMode::Enforce),
            true,
            false,
        )
        .unwrap();
        assert_eq!(mode, SandboxMode::Warn);

        // An explicit `--sandbox off` overrides a manifest-set mode.
        let mode = resolve_sandbox_mode(
            Some(SandboxMode::Off),
            Some(SandboxMode::Prompt),
            true,
            false,
        )
        .unwrap();
        assert_eq!(mode, SandboxMode::Off);
    }

    #[test]
    fn resolve_sandbox_mode_manifest_applies_without_cli_flag() {
        let mode = resolve_sandbox_mode(None, Some(SandboxMode::Warn), true, false).unwrap();
        assert_eq!(mode, SandboxMode::Warn);
    }

    #[test]
    fn resolve_sandbox_mode_defaults_to_off() {
        let mode = resolve_sandbox_mode(None, None, true, false).unwrap();
        assert_eq!(mode, SandboxMode::Off);

        let mode = resolve_sandbox_mode(None, None, false, false).unwrap();
        assert_eq!(mode, SandboxMode::Off);
    }

    #[test]
    fn resolve_sandbox_mode_ignores_manifest_for_ephemeral_activation() {
        let mode = resolve_sandbox_mode(None, Some(SandboxMode::Enforce), true, true).unwrap();
        assert_eq!(mode, SandboxMode::Off);
    }

    #[test]
    fn advisory_modes_rejected_on_enforcing_backends() {
        // warn/prompt are libsandbox-only; a kernel/container/hypervisor backend
        // must reject them rather than silently enforce.
        let enforcing = [
            SandboxBackend::Nix,
            SandboxBackend::HostNative,
            SandboxBackend::Srt,
            SandboxBackend::Oci,
            SandboxBackend::Libkrun,
        ];
        for backend in enforcing {
            for mode in [SandboxMode::Warn, SandboxMode::Prompt] {
                assert!(
                    ensure_advisory_mode_supported(backend, mode).is_err(),
                    "{backend} should reject advisory mode '{mode}'",
                );
            }
            // enforce and off are always fine.
            assert!(ensure_advisory_mode_supported(backend, SandboxMode::Enforce).is_ok());
            assert!(ensure_advisory_mode_supported(backend, SandboxMode::Off).is_ok());
        }
    }

    #[test]
    fn advisory_modes_allowed_on_libsandbox() {
        for mode in [
            SandboxMode::Off,
            SandboxMode::Warn,
            SandboxMode::Enforce,
            SandboxMode::Prompt,
        ] {
            assert!(ensure_advisory_mode_supported(SandboxBackend::Libsandbox, mode).is_ok());
        }
    }

    #[test]
    fn advisory_mode_rejection_names_mode_and_points_at_libsandbox() {
        let err = ensure_advisory_mode_supported(SandboxBackend::HostNative, SandboxMode::Warn)
            .unwrap_err()
            .to_string();
        assert!(err.contains("host-native"), "message: {err}");
        assert!(err.contains("'warn'"), "message: {err}");
        assert!(err.contains("--sandbox enforce"), "message: {err}");
        assert!(err.contains("libsandbox"), "message: {err}");
    }

    #[test]
    fn resolve_sandbox_mode_manifest_downgraded_without_feature_flag() {
        let (subscriber, writer) = test_subscriber_message_only();

        let mode = tracing::subscriber::with_default(subscriber, || {
            resolve_sandbox_mode(None, Some(SandboxMode::Prompt), false, false).unwrap()
        });

        assert_eq!(mode, SandboxMode::Off);
        let printed = writer.to_string();
        assert!(
            printed.contains("Ignoring 'options.sandbox' from the manifest"),
            "printed: {printed}"
        );
    }

    #[test]
    fn resolve_sandbox_mode_cli_flag_without_feature_flag_errors() {
        let err = resolve_sandbox_mode(Some(SandboxMode::Warn), None, false, false).unwrap_err();
        assert!(
            err.to_string().contains("sandbox_activate feature flag"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn in_place_guard_rejects_any_active_sandbox_mode() {
        assert!(
            ensure_sandbox_not_in_place(SandboxMode::Prompt, &InvocationType::InPlace).is_err()
        );
        assert!(ensure_sandbox_not_in_place(SandboxMode::Warn, &InvocationType::InPlace).is_err());
        assert!(
            ensure_sandbox_not_in_place(SandboxMode::Enforce, &InvocationType::InPlace).is_err()
        );
        assert!(ensure_sandbox_not_in_place(SandboxMode::Off, &InvocationType::InPlace).is_ok());
    }

    #[test]
    fn in_place_guard_allows_other_invocation_types() {
        assert!(
            ensure_sandbox_not_in_place(SandboxMode::Prompt, &InvocationType::Interactive).is_ok()
        );
        assert!(
            ensure_sandbox_not_in_place(
                SandboxMode::Enforce,
                &InvocationType::ExecCommand(vec!["true".to_string()])
            )
            .is_ok()
        );
    }
}

#[cfg(test)]
mod upgrade_notification_tests {
    use flox_core::activate::vars::FLOX_ACTIVE_ENVIRONMENTS_VAR;
    use flox_manifest::lockfile::{LockedPackage, Lockfile};
    use flox_rust_sdk::flox::test_helpers::flox_instance;
    use flox_rust_sdk::models::environment::UpgradeResult;
    use flox_rust_sdk::models::environment::path_environment::test_helpers::{
        new_named_path_environment_from_env_files,
        new_path_environment_from_env_files,
    };
    use flox_rust_sdk::providers::upgrade_checks::UpgradeInformation;
    use flox_rust_sdk::utils::logging::test_helpers::test_subscriber_message_only;
    use flox_test_utils::GENERATED_DATA;
    use time::OffsetDateTime;

    use super::*;
    use crate::commands::ActiveEnvironments;

    #[test]
    fn no_notification_printed_if_absent() {
        let (flox, _tempdir) = flox_instance();

        let (subscriber, writer) = test_subscriber_message_only();

        let environment =
            new_path_environment_from_env_files(&flox, GENERATED_DATA.join("envs/hello"));
        let mut environment = ConcreteEnvironment::Path(environment);

        tracing::subscriber::with_default(subscriber, || {
            notify_upgrades_if_available(&flox, &mut environment, &EnvironmentSelect::Unspecified)
                .unwrap();
        });

        let printed = writer.to_string();

        assert!(printed.is_empty(), "printed: {printed}");
    }

    fn write_upgrade_available(flox: &Flox, environment: &mut ConcreteEnvironment) {
        let upgrade_information =
            UpgradeInformationGuard::read_in(environment.cache_path().unwrap()).unwrap();
        let mut locked = upgrade_information.lock_if_unlocked().unwrap().unwrap();

        let mut new_lockfile: Lockfile = environment.lockfile(flox).unwrap().into();
        for locked_package in new_lockfile.packages.iter_mut() {
            match locked_package {
                LockedPackage::Catalog(locked_package_catalog) => {
                    locked_package_catalog.derivation = "upgraded".to_string()
                },
                LockedPackage::Flake(locked_package_flake) => {
                    locked_package_flake.locked_installable.derivation = "upgraded".to_string()
                },
                LockedPackage::StorePath(_) => {},
            }
        }

        let _ = locked.info_mut().insert(UpgradeInformation {
            last_checked: OffsetDateTime::now_utc(),
            upgrade_result: UpgradeResult {
                old_lockfile: Some(environment.lockfile(flox).unwrap().into()),
                new_lockfile,

                store_path: None,
            },
        });

        locked.commit().unwrap();
    }

    #[test]
    fn no_notification_printed_if_already_active() {
        let (flox, _tempdir) = flox_instance();
        let (subscriber, writer) = test_subscriber_message_only();

        let environment =
            new_path_environment_from_env_files(&flox, GENERATED_DATA.join("envs/hello"));
        let mut environment = ConcreteEnvironment::Path(environment);

        let mut active = ActiveEnvironments::default();
        active.set_last_active(
            UninitializedEnvironment::from_concrete_environment(&environment),
            None,
            ActivateMode::Dev,
        );

        write_upgrade_available(&flox, &mut environment);

        temp_env::with_var(
            FLOX_ACTIVE_ENVIRONMENTS_VAR,
            Some(active.to_string()),
            || {
                tracing::subscriber::with_default(subscriber, || {
                    notify_upgrades_if_available(
                        &flox,
                        &mut environment,
                        &EnvironmentSelect::Unspecified,
                    )
                    .unwrap();
                });
            },
        );

        let printed = writer.to_string();

        assert!(printed.is_empty(), "printed: {printed}");
    }

    #[test]
    fn notification_printed_if_present() {
        let (flox, _tempdir) = flox_instance();
        let (subscriber, writer) = test_subscriber_message_only();

        let environment = new_named_path_environment_from_env_files(
            &flox,
            GENERATED_DATA.join("envs/hello"),
            "name",
        );
        let mut environment = ConcreteEnvironment::Path(environment);

        write_upgrade_available(&flox, &mut environment);

        tracing::subscriber::with_default(subscriber, || {
            notify_upgrades_if_available(&flox, &mut environment, &EnvironmentSelect::Unspecified)
                .unwrap();
        });

        let printed = writer.to_string();

        assert_eq!(printed, formatdoc! {"
            ℹ 1 rebuild available in 'name'.
            Use 'flox upgrade --dry-run' for details.

        "});
    }

    /// When the user specifies an environment via flags (e.g. `-d <path>` or
    /// `-r <env>`), the upgrade hint must include those flags so the suggested
    /// command actually targets the right environment.
    #[test]
    fn notification_printed_with_dir_flags() {
        let (flox, _tempdir) = flox_instance();
        let (subscriber, writer) = test_subscriber_message_only();

        let path_env = new_named_path_environment_from_env_files(
            &flox,
            GENERATED_DATA.join("envs/hello"),
            "name",
        );
        // Capture the parent of the .flox directory so we can construct a
        // matching EnvironmentSelect::Dir value.
        let dot_flox_parent = path_env.path.parent().unwrap().to_path_buf();
        let mut environment = ConcreteEnvironment::Path(path_env);

        write_upgrade_available(&flox, &mut environment);

        let env_select = EnvironmentSelect::Dir(dot_flox_parent.clone());

        tracing::subscriber::with_default(subscriber, || {
            notify_upgrades_if_available(&flox, &mut environment, &env_select).unwrap();
        });

        let printed = writer.to_string();
        let expected_flags = format!("-d {}", dot_flox_parent.display());

        assert!(
            printed.contains(&format!("'flox upgrade --dry-run {expected_flags}'")),
            "expected upgrade hint to include env flags, got: {printed}"
        );
    }

    #[test]
    fn no_notification_printed_if_outdated() {
        let (flox, _tempdir) = flox_instance();
        let (subscriber, writer) = test_subscriber_message_only();

        let environment =
            new_path_environment_from_env_files(&flox, GENERATED_DATA.join("envs/hello"));
        let mut environment = ConcreteEnvironment::Path(environment);

        {
            let upgrade_information =
                UpgradeInformationGuard::read_in(environment.cache_path().unwrap()).unwrap();
            let mut locked = upgrade_information.lock_if_unlocked().unwrap().unwrap();

            // cause old_lockfile to evaluate as non-equal to the current lockfile
            let mut old_lockfile: Lockfile = environment.lockfile(&flox).unwrap().into();
            old_lockfile.packages.clear();

            let _ = locked.info_mut().insert(UpgradeInformation {
                last_checked: OffsetDateTime::now_utc(),
                upgrade_result: UpgradeResult {
                    old_lockfile: Some(old_lockfile),
                    new_lockfile: environment.lockfile(&flox).unwrap().into(),

                    store_path: None,
                },
            });

            locked.commit().unwrap();
        }

        tracing::subscriber::with_default(subscriber, || {
            notify_upgrades_if_available(&flox, &mut environment, &EnvironmentSelect::Unspecified)
                .unwrap();
        });

        let printed = writer.to_string();
        assert!(printed.is_empty(), "printed: {printed}");
    }

    #[test]
    fn no_notification_printed_if_no_diff() {
        let (flox, _tempdir) = flox_instance();
        let (subscriber, writer) = test_subscriber_message_only();

        let environment =
            new_path_environment_from_env_files(&flox, GENERATED_DATA.join("envs/hello"));
        let mut environment = ConcreteEnvironment::Path(environment);

        {
            let upgrade_information =
                UpgradeInformationGuard::read_in(environment.cache_path().unwrap()).unwrap();

            let upgrade_result = UpgradeResult {
                old_lockfile: Some(environment.lockfile(&flox).unwrap().into()),
                new_lockfile: environment.lockfile(&flox).unwrap().into(),

                store_path: None,
            };

            assert!(upgrade_result.diff().is_empty());

            let mut locked = upgrade_information.lock_if_unlocked().unwrap().unwrap();

            let _ = locked.info_mut().insert(UpgradeInformation {
                last_checked: OffsetDateTime::now_utc(),
                upgrade_result,
            });

            locked.commit().unwrap();
        }

        tracing::subscriber::with_default(subscriber, || {
            notify_upgrades_if_available(&flox, &mut environment, &EnvironmentSelect::Unspecified)
                .unwrap();
        });

        let printed = writer.to_string();
        assert!(printed.is_empty(), "printed: {printed}");
    }
}
