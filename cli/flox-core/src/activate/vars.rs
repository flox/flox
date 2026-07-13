// The FLOX_* variables which follow are currently updated by the CLI as it
// activates new environments, and they are consequently *not* updated with
// manual invocations of the activation script. We want the activation script
// to eventually have feature parity with the CLI, so in future we will need
// to migrate this logic to the activation script itself.

use std::env;
use std::path::PathBuf;
use std::sync::LazyLock;

/// The environments active in this shell, as a JSON array of serialized
/// environment metadata (`UninitializedEnvironment` in `flox-rust-sdk`), most
/// recently activated first. Set by `flox activate`; read wherever an active
/// environment must be reopened (e.g. `flox deactivate`, the prompt hook) and
/// printed to users by `flox envs`.
pub const FLOX_ACTIVE_ENVIRONMENTS_VAR: &str = "_FLOX_ACTIVE_ENVIRONMENTS";

/// Numeric log verbosity for the `flox-activations` binary, exported by the
/// CLI from its own verbosity so subprocess logging matches `flox -v` levels.
/// Overridden by `RUST_LOG` when both are set.
pub const FLOX_ACTIVATIONS_VERBOSITY_VAR: &str = "_FLOX_ACTIVATIONS_VERBOSITY";

/// Numeric log verbosity for the executive subsystem's log file, deliberately
/// separate from [`FLOX_ACTIVATIONS_VERBOSITY_VAR`] so that `flox activate -v`
/// does not change what the long-lived executive process records.
pub const FLOX_EXECUTIVE_VERBOSITY_VAR: &str = "_FLOX_EXECUTIVE_VERBOSITY";

/// Project directories whose environments the prompt hook auto-activated in
/// this shell, as a JSON array of absolute paths, outermost-first. Maintained
/// by the script `flox hook-env` emits; used to decide which environments to
/// auto-deactivate when the shell leaves their directory.
pub const FLOX_AUTO_ACTIVATED_ENVIRONMENTS_VAR: &str = "_FLOX_AUTO_ACTIVATED_ENVIRONMENTS";

/// The invocation types of the activations performed by *this* shell, as a
/// JSON array of `{env, invocation_type}` entries keyed by the environment
/// pointer as it appears in `_FLOX_ACTIVE_ENVIRONMENTS` (see
/// [`super::context::InvocationTypes`]).
/// Deliberately a shell variable rather than an exported one: a subshell
/// inherits the activation's exported environment but did not attach to the
/// activation itself, and an absent/empty value is how deactivation knows
/// not to emit a `flox-activations detach` for it. Updated by
/// `flox-activations push-invocation-type` from each activation's startup
/// script (only the eval'ing shell can see the current value); passed to
/// `flox hook-env`/`flox deactivate --print-script` by shell code expanding
/// it (a subprocess can't read it from the environment), which take the
/// entry for each layer they deactivate and emit an update writing the
/// remainder back.
pub const FLOX_INVOCATION_TYPES_VAR: &str = "_FLOX_INVOCATION_TYPES";

/// Short-lived exported variable through which tcsh passes the current
/// [`FLOX_INVOCATION_TYPES_VAR`] value to `flox hook-env`,
/// `flox deactivate --print-script-from-env` and
/// `flox-activations push-invocation-type`: raw JSON cannot ride a tcsh
/// backtick command line (globbing and quote-stripping mangle it), so the
/// caller `setenv`s this immediately before the call and `unsetenv`s it
/// immediately after. The other shells pass the value as a quoted argument.
pub const FLOX_INVOCATION_TYPES_WIRE_VAR: &str = "_FLOX_INVOCATION_TYPES_WIRE";

/// Short-lived exported variable through which tcsh passes the environment
/// pointer of the environment being activated to
/// `flox-activations push-invocation-type` (same tcsh constraint and
/// setenv/unsetenv lifetime as [`FLOX_INVOCATION_TYPES_WIRE_VAR`]).
pub const FLOX_INVOCATION_TYPES_PUSH_ENV_VAR: &str = "_FLOX_INVOCATION_TYPES_PUSH_ENV";

/// Project directories the prompt hook must not auto-(re)activate while the
/// shell remains inside them, as a JSON array of absolute paths. An entry is
/// added when an environment is deactivated while the shell is still inside
/// its directory (e.g. by 'flox deactivate') and removed once the shell
/// leaves that directory, so a later re-entry auto-activates again.
pub const FLOX_SUPPRESSED_ENVIRONMENTS_VAR: &str = "_FLOX_SUPPRESSED_ENVIRONMENTS";

pub static FLOX_ACTIVATIONS_BIN: LazyLock<PathBuf> = LazyLock::new(|| {
    PathBuf::from(
        env::var("FLOX_ACTIVATIONS_BIN").unwrap_or(env!("FLOX_ACTIVATIONS_BIN").to_string()),
    )
});
