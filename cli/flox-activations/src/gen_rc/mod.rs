use std::path::PathBuf;

use flox_core::activate::context::ActivateCtx;

/// Absolute path to coreutils `rm`, avoiding user alias expansion (e.g. `alias rm='rm -i'`).
const RM: &str = concat!(env!("COREUTILS"), "/bin/rm");

use crate::attach_diff::AttachDiff;
use crate::attach_diff::diff_serializer::DiffSerializer;
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
    Deactivate(DeactivateCtx),
}

/// Context for shell deactivation, shared across all shell flavors.
#[derive(Debug, Clone)]
pub struct DeactivateCtx {
    pub activate_d: PathBuf,
    /// The env being torn down. Supplied by the caller (derived from the
    /// active-environment stack and that env's `ConcreteEnvironment`) so the
    /// emitted script does not depend on runtime `$FLOX_ENV` — which is the
    /// most-recently activated env, not necessarily the one being torn down.
    pub flox_env: PathBuf,
    /// Verbosity passed through from `_FLOX_SUBSYSTEM_VERBOSITY`; when
    /// `>= 2` each shell wraps its deactivate output in a trace mode
    /// (`set -x`, `set verbose`, `set -gx fish_trace 1`).
    pub flox_activate_tracelevel: u32,
    /// Decoded from `_FLOX_HOOK_DIFF`
    pub restore_diff: DiffSerializer,
    /// Path to the `flox-activations` binary, embedded in generated shell
    /// for inner-deactivation `fix-fpath` calls.
    pub flox_activations: PathBuf,
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
    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;

    use flox_core::activate::context::{ActivateCtx, AttachCtx, AttachProjectCtx, InvocationType};
    use flox_core::activate::mode::ActivateMode;
    use flox_core::activate::vars::{FLOX_ACTIVATIONS_BIN, FLOX_ACTIVE_ENVIRONMENTS_VAR};
    use shell_gen::ShellWithPath;

    use super::{DeactivateCtx, StartupCtx};
    use crate::attach::{startup_ctx, write_to_writer};
    use crate::attach_diff::diff_serializer::{DiffSerializer, FLOX_HOOK_DIFF_VAR};
    use crate::start_diff::StartDiff;
    use crate::vars_from_env::VarsFromEnvironment;

    /// Build a deterministic `StartupCtx` for tests of `gen_rc/*`.
    /// Fills every `AttachCtx` / `AttachProjectCtx` / `ActivateCtx`
    /// field with stable test values so snapshot output is
    /// reproducible across shells.
    ///
    /// Auto-activation is off, so no prompt hook is registered. Use
    /// [`test_startup_ctx_hook`] to control that.
    pub fn test_startup_ctx(shell: ShellWithPath, is_in_place: bool) -> StartupCtx {
        test_startup_ctx_hook(shell, is_in_place, false, false)
    }

    /// Like [`test_startup_ctx`] but with control over the auto-activation
    /// inputs, for tests that exercise prompt-hook registration. The hook is
    /// emitted when `auto_activate` is set and `disable_hook` is not.
    pub fn test_startup_ctx_hook(
        shell: ShellWithPath,
        is_in_place: bool,
        auto_activate: bool,
        disable_hook: bool,
    ) -> StartupCtx {
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
            auto_activate,
            disable_hook,
            flox_bin: "/flox".to_string(),
            auto_activate_fish_mode: None,
        };
        let deleted_var = "DELETED_VAR".to_string();
        let modified_var = "MODIFIED_VAR".to_string();
        let start_diff = StartDiff::from_parts(
            HashMap::from([
                ("ADDED_VAR".to_string(), "ADDED_VALUE".to_string()),
                (modified_var.clone(), "MODIFIED_VALUE".to_string()),
                ("QUOTED_VAR".to_string(), "QUOTED'VALUE".to_string()),
            ]),
            vec![deleted_var.clone()],
        );
        let full_env = HashMap::from([
            (deleted_var.clone(), "DELETED_ORIGINAL".to_string()),
            (modified_var.clone(), "MODIFIED_ORIGINAL".to_string()),
        ]);
        let vars_from_env = VarsFromEnvironment {
            flox_env_dirs: None,
            path: None,
            manpath: None,
            full_env: Some(full_env),
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

    /// Build a `DeactivateCtx` for tests of `gen_rc/*`.
    ///
    /// Reuses the encoded `_FLOX_HOOK_DIFF` produced by `test_startup_ctx`
    /// so the deactivate snapshot reflects exactly what activation would
    /// have captured.
    pub fn test_deactivate_ctx(shell: ShellWithPath, is_in_place: bool) -> DeactivateCtx {
        let startup = test_startup_ctx(shell, is_in_place);
        let encoded_diff = startup
            .attach_diff
            .encoded_diff()
            .expect("test_startup_ctx should produce an encoded diff")
            .to_string();
        let restore_diff =
            DiffSerializer::decode(&encoded_diff).expect("encoded diff should decode successfully");
        DeactivateCtx {
            activate_d: PathBuf::from("/interpreter/activate.d"),
            flox_env: PathBuf::from("/flox_env"),
            flox_activate_tracelevel: 0,
            restore_diff,
            flox_activations: PathBuf::from("/flox-activations"),
        }
    }

    /// Build a `DeactivateCtx` that represents deactivating an *inner*
    /// (non-outermost) activation for tests of `gen_rc/*`.
    ///
    /// Constructs a diff where `_FLOX_ACTIVE_ENVIRONMENTS` is in `modified`
    /// with a non-empty prior value so that `is_outermost_deactivate()`
    /// returns `false`, triggering the inner-deactivation FPATH path.
    pub fn test_deactivate_ctx_inner(shell: ShellWithPath) -> DeactivateCtx {
        let restore_diff = DiffSerializer {
            added: HashSet::from(["ADDED_VAR".to_string()]),
            modified: HashMap::from([
                ("MODIFIED_VAR".to_string(), "MODIFIED_ORIGINAL".to_string()),
                // Non-empty outer value → inner activation → not outermost.
                (
                    FLOX_ACTIVE_ENVIRONMENTS_VAR.to_string(),
                    "/outer/env".to_string(),
                ),
                // Outer diff value to restore when deactivating the inner env.
                (
                    FLOX_HOOK_DIFF_VAR.to_string(),
                    "outer_encoded_diff_placeholder".to_string(),
                ),
                // Outer invocation type to restore when deactivating the inner env.
                ("_FLOX_INVOCATION_TYPE".to_string(), "inplace".to_string()),
            ]),
            removed: HashMap::from([("DELETED_VAR".to_string(), "DELETED_ORIGINAL".to_string())]),
        };
        let _ = shell; // shell type doesn't affect deactivation output
        DeactivateCtx {
            activate_d: PathBuf::from("/interpreter/activate.d"),
            flox_env: PathBuf::from("/flox_env"),
            flox_activate_tracelevel: 0,
            restore_diff,
            flox_activations: PathBuf::from("/flox-activations"),
        }
    }

    /// Strip lines referencing platform-specific or
    /// environment-dependent variables from rendered deactivate
    /// output. Also normalizes the absolute `FLOX_ACTIVATIONS_BIN`
    /// path to `/flox_activations` so snapshots are portable.
    pub fn strip_volatile_deactivate(output: &str) -> String {
        const VOLATILE: &[&str] = &[
            "LOCALE_ARCHIVE",
            "NIX_SSL_CERT_FILE",
            "PATH_LOCALE",
            "SSL_CERT_FILE",
        ];
        let bin = FLOX_ACTIVATIONS_BIN.display().to_string();
        let normalized = output.replace(&bin, "/flox_activations");
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
        // and the opaque `_FLOX_HOOK_DIFF` blob
        // TODO: should we move those to vars_from_environment?
        const VOLATILE: &[&str] = &[
            "LOCALE_ARCHIVE",
            "NIX_SSL_CERT_FILE",
            "PATH_LOCALE",
            "SSL_CERT_FILE",
            "_FLOX_HOOK_DIFF",
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
