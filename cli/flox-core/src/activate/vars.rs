// The FLOX_* variables which follow are currently updated by the CLI as it
// activates new environments, and they are consequently *not* updated with
// manual invocations of the activation script. We want the activation script
// to eventually have feature parity with the CLI, so in future we will need
// to migrate this logic to the activation script itself.

use std::env;
use std::path::PathBuf;
use std::sync::LazyLock;

pub const FLOX_ACTIVE_ENVIRONMENTS_VAR: &str = "_FLOX_ACTIVE_ENVIRONMENTS";
pub const FLOX_RUNTIME_DIR_VAR: &str = "FLOX_RUNTIME_DIR";
pub const FLOX_ACTIVATIONS_VERBOSITY_VAR: &str = "_FLOX_ACTIVATIONS_VERBOSITY";
pub const FLOX_EXECUTIVE_VERBOSITY_VAR: &str = "_FLOX_EXECUTIVE_VERBOSITY";

pub static FLOX_ACTIVATIONS_BIN: LazyLock<PathBuf> = LazyLock::new(|| {
    PathBuf::from(
        env::var("FLOX_ACTIVATIONS_BIN").unwrap_or(env!("FLOX_ACTIVATIONS_BIN").to_string()),
    )
});
