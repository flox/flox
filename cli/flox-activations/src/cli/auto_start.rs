use std::fs::{self, DirBuilder};
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Result;
use clap::Args;
use flox_core::activate::context::AutoStartCtx;
use flox_core::activations::{
    ActivationState,
    StartIdentifier,
    StartOrAttachResult,
    read_activations_json,
    state_json_path,
    write_activations_json,
};
use signal_hook::consts::{SIGCHLD, SIGUSR1};
use signal_hook::iterator::Signals;
use tracing::debug;

use crate::start::{spawn_executive, wait_for_executive};

/// Result reported on stdout as JSON.
#[derive(Debug, serde::Serialize)]
pub struct AutoStartResult {
    pub status: String,
    pub start_id: String,
    pub is_new: bool,
}

#[derive(Debug, Args)]
pub struct AutoStartArgs {
    /// Shell PID to register with the activation
    #[arg(long)]
    pub pid: i32,

    /// Path to JSON file containing AutoStartCtx
    #[arg(long)]
    pub activate_data: PathBuf,
}

impl AutoStartArgs {
    pub fn handle(self) -> Result<(), anyhow::Error> {
        let contents = fs::read_to_string(&self.activate_data)?;
        let ctx: AutoStartCtx = serde_json::from_str(&contents)?;

        // Clean up the context file
        let _ = fs::remove_file(&self.activate_data);

        let start_id = self.start_or_attach(&ctx)?;

        let is_new = start_id.1;
        let result = AutoStartResult {
            status: "ok".to_string(),
            start_id: serde_json::to_string(&start_id.0)?,
            is_new,
        };
        println!("{}", serde_json::to_string(&result)?);

        Ok(())
    }

    fn start_or_attach(
        &self,
        ctx: &AutoStartCtx,
    ) -> Result<(StartIdentifier, bool), anyhow::Error> {
        let retry_delay = Duration::from_millis(200);
        let warning_interval = Duration::from_secs(5);
        let mut last_warning: Option<Instant> = None;

        loop {
            match self.try_start_or_attach(ctx)? {
                StartOrAttachResult::Start { start_id, .. } => {
                    return Ok((start_id, true));
                },
                StartOrAttachResult::Attach { start_id, .. } => {
                    return Ok((start_id, false));
                },
                StartOrAttachResult::AlreadyStarting {
                    pid: blocking_pid, ..
                } => {
                    let now = Instant::now();
                    let should_warn =
                        last_warning.is_none_or(|t| now.duration_since(t) >= warning_interval);

                    if should_warn {
                        eprintln!(
                            "Waiting for another activation to complete (blocked by PID {})...",
                            blocking_pid
                        );
                        last_warning = Some(now);
                    }

                    std::thread::sleep(retry_delay);
                },
            }
        }
    }

    fn try_start_or_attach(
        &self,
        ctx: &AutoStartCtx,
    ) -> Result<StartOrAttachResult, anyhow::Error> {
        let activations_json_path = state_json_path(&ctx.activation_state_dir);
        let (activations_opt, lock) = read_activations_json(&activations_json_path)?;

        let mut activations = activations_opt.unwrap_or_else(|| {
            debug!("no existing activation state, creating new one");
            ActivationState::new(&ctx.mode, Some(&ctx.dot_flox_path), &ctx.flox_env)
        });

        // Reset state if executive is not running
        if !activations.executive_running() {
            debug!("discarding activation state due to executive not running");
            activations = ActivationState::new(&ctx.mode, Some(&ctx.dot_flox_path), &ctx.flox_env);
        }

        match activations.start_or_attach(self.pid, &ctx.store_path) {
            StartOrAttachResult::Start { start_id } => {
                let result = self.do_start(
                    ctx,
                    start_id,
                    &mut activations,
                    &activations_json_path,
                    lock,
                )?;
                Ok(result)
            },
            StartOrAttachResult::Attach { start_id } => {
                write_activations_json(&activations, &activations_json_path, lock)?;
                Ok(StartOrAttachResult::Attach { start_id })
            },
            StartOrAttachResult::AlreadyStarting { pid, start_id } => {
                drop(lock);
                Ok(StartOrAttachResult::AlreadyStarting { pid, start_id })
            },
        }
    }

    fn do_start(
        &self,
        ctx: &AutoStartCtx,
        start_id: StartIdentifier,
        activations: &mut ActivationState,
        activations_json_path: &std::path::Path,
        lock: fslock::LockFile,
    ) -> Result<StartOrAttachResult, anyhow::Error> {
        let start_state_dir = start_id.start_state_dir(&ctx.activation_state_dir)?;
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&start_state_dir)?;

        // Build context structs needed by spawn_executive
        let attach_ctx = flox_core::activate::context::AttachCtx {
            env: ctx.flox_env.clone(),
            env_cache: ctx.env_cache.clone(),
            env_description: ctx.env_description.clone(),
            flox_active_environments: String::new(),
            prompt_color_1: String::new(),
            prompt_color_2: String::new(),
            flox_prompt_environments: String::new(),
            set_prompt: false,
            flox_env_cuda_detection: String::new(),
            interpreter_path: ctx.interpreter_path.clone(),
        };

        let project_ctx = flox_core::activate::context::AttachProjectCtx {
            env_project: ctx.env_project.clone(),
            dot_flox_path: ctx.dot_flox_path.clone(),
            flox_env_log_dir: ctx.flox_env_log_dir.clone(),
            process_compose_bin: ctx.process_compose_bin.clone(),
            flox_services_socket: ctx.flox_services_socket.clone(),
            services_to_start: Vec::new(),
        };

        // Spawn executive if not already running
        let new_executive = if !activations.executive_started() {
            let signals = Signals::new([SIGCHLD, SIGUSR1])?;
            let exec_pid = spawn_executive(
                &attach_ctx,
                &project_ctx,
                &ctx.activation_state_dir,
                &start_state_dir,
                ctx.metrics_uuid,
            )?;
            activations.set_executive_pid(exec_pid.as_raw());
            Some((exec_pid, signals))
        } else {
            None
        };

        write_activations_json(activations, activations_json_path, lock)?;

        if let Some((exec_pid, signals)) = new_executive {
            wait_for_executive(exec_pid, signals)?;
        }

        // Phase 1: Skip on-activate hooks - just mark as ready
        // Phase 2 will add hook execution here

        // Re-acquire lock to mark ready
        let (activations_opt, lock) = read_activations_json(activations_json_path)?;
        let mut activations = activations_opt.expect("state.json should exist");
        activations.set_ready(&start_id);
        write_activations_json(&activations, activations_json_path, lock)?;

        Ok(StartOrAttachResult::Start { start_id })
    }
}
