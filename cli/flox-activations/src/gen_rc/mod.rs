use std::path::PathBuf;

use flox_core::activate::context::ActivateCtx;

/// Absolute path to coreutils `rm`, avoiding user alias expansion (e.g. `alias rm='rm -i'`).
const RM: &str = concat!(env!("COREUTILS"), "/bin/rm");

use crate::attach_diff::AttachDiff;
use crate::gen_rc::bash::BashStartupArgs;
use crate::gen_rc::fish::FishStartupArgs;
use crate::gen_rc::tcsh::TcshStartupArgs;
use crate::gen_rc::zsh::ZshStartupArgs;

pub mod bash;
pub mod fish;
pub mod tcsh;
pub mod zsh;

/// Struct to container arguments needed by generate_*_profile_commands
#[derive(Debug, Clone)]
pub enum Action<A> {
    Activate { args: A, attach_diff: AttachDiff },
    Deactivate { activate_d: PathBuf },
}

#[derive(Debug, Clone)]
pub enum ShellStartupArgs {
    Bash(BashStartupArgs),
    Fish(FishStartupArgs),
    Tcsh(TcshStartupArgs),
    Zsh(ZshStartupArgs),
}

/// Context for shell startup, shared between normal and container activations.
#[derive(Debug)]
pub struct StartupCtx {
    pub args: ShellStartupArgs,
    pub rc_path: Option<PathBuf>,
    pub act_ctx: ActivateCtx,
    pub attach_diff: AttachDiff,
}

#[cfg(test)]
pub(crate) mod test_helpers {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use flox_core::activate::context::{ActivateCtx, AttachCtx, AttachProjectCtx, InvocationType};
    use flox_core::activate::mode::ActivateMode;
    use flox_core::activate::vars::FLOX_ACTIVATIONS_BIN;
    use shell_gen::ShellWithPath;

    use super::StartupCtx;
    use crate::attach::{startup_ctx, write_to_writer};
    use crate::start_diff::StartDiff;
    use crate::vars_from_env::VarsFromEnvironment;

    /// Build a deterministic `StartupCtx` for tests of `gen_rc/*`.
    /// Fills every `AttachCtx` / `AttachProjectCtx` / `ActivateCtx`
    /// field with stable test values so snapshot output is
    /// reproducible across shells.
    pub fn test_startup_ctx(shell: ShellWithPath, is_in_place: bool) -> StartupCtx {
        let invocation_type = if is_in_place {
            InvocationType::InPlace
        } else {
            InvocationType::Interactive
        };
        let attach_ctx = AttachCtx {
            env: "/flox_env".to_string(),
            env_cache: PathBuf::from("/flox_env_cache"),
            env_description: "env_description".to_string(),
            flox_active_environments: "active_envs".to_string(),
            prompt_color_1: "1".to_string(),
            prompt_color_2: "2".to_string(),
            flox_prompt_environments: "prompt_envs".to_string(),
            set_prompt: true,
            flox_env_cuda_detection: "0".to_string(),
            interpreter_path: PathBuf::from("/interpreter"),
        };
        let project_ctx = Some(AttachProjectCtx {
            env_project: PathBuf::from("/flox_env_project"),
            dot_flox_path: PathBuf::from("/dot_flox"),
            flox_env_log_dir: PathBuf::from("/flox_env_log_dir"),
            process_compose_bin: PathBuf::from("/process_compose"),
            flox_services_socket: PathBuf::from("/flox_services_socket"),
            services_to_start: Vec::new(),
        });
        let bashrc_path =
            matches!(shell, ShellWithPath::Bash(_)).then(|| PathBuf::from("/home/user/.bashrc"));
        let act_ctx = ActivateCtx {
            flox_activate_store_path: "/store_path".to_string(),
            attach_ctx,
            project_ctx,
            activation_state_dir: PathBuf::from("/activation_state_dir"),
            mode: ActivateMode::Dev,
            shell,
            invocation_type: Some(invocation_type.clone()),
            remove_after_reading: false,
            metrics_uuid: None,
            auto_activate: false,
            flox_bin: "/flox".to_string(),
            auto_activate_fish_mode: None,
        };
        let start_diff = StartDiff::from_parts(
            HashMap::from([
                ("ADDED_VAR".to_string(), "ADDED_VALUE".to_string()),
                ("QUOTED_VAR".to_string(), "QUOTED'VALUE".to_string()),
            ]),
            vec!["DELETED_VAR".to_string()],
        );
        let vars_from_env = VarsFromEnvironment {
            flox_env_dirs: None,
            path: None,
            manpath: None,
            full_env: None,
        };
        let rc_path = Some(PathBuf::from("/path/to/rc/file"));
        startup_ctx(
            act_ctx,
            invocation_type,
            rc_path,
            start_diff,
            "TRACER",
            3,
            vars_from_env,
            false,
            true,
            bashrc_path,
        )
        .expect("test fixture should build")
    }

    /// Render `ctx` via `write_to_writer`, stripping platform specific lines
    /// and replacing /nix/store hashes
    pub fn render_normalized(ctx: &StartupCtx) -> String {
        const PREFIX: &str = "/nix/store/";
        const HASH_LEN: usize = 32;
        let mut buf = Vec::new();
        write_to_writer(ctx, &mut buf).expect("write_to_writer should succeed");
        // flox-activations could be cargo compiled rather than a /nix/store path
        let flox_activations_bin = FLOX_ACTIVATIONS_BIN.display().to_string();
        let mut normalized =
            String::from_utf8_lossy(&buf).replace(&flox_activations_bin, "/flox_activations");

        // Vibed but seems to be working
        // Replace store path hashes
        let mut start = 0;
        while let Some(i) = normalized[start..].find(PREFIX) {
            let hash_start = start + i + PREFIX.len();
            let hash_end = hash_start + HASH_LEN;
            if hash_end <= normalized.len()
                && normalized.as_bytes()[hash_start..hash_end]
                    .iter()
                    .all(|b| b.is_ascii_alphanumeric())
            {
                normalized.replace_range(hash_start..hash_end, &"X".repeat(HASH_LEN));
            }
            start = hash_end;
        }

        // Drop platform specific or environment variable dependent variables
        // TODO: should we move those to vars_from_environment?
        const VOLATILE: &[&str] = &[
            "LOCALE_ARCHIVE",
            "NIX_SSL_CERT_FILE",
            "PATH_LOCALE",
            "SSL_CERT_FILE",
        ];
        let trailing_newline = normalized.ends_with('\n');
        let mut filtered = normalized
            .lines()
            .filter(|l| !VOLATILE.iter().any(|v| l.contains(v)))
            .collect::<Vec<_>>()
            .join("\n");
        if trailing_newline {
            filtered.push('\n');
        }
        filtered
    }
}
