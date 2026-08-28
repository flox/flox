//! Shared protocol for invoking `[plugin-hooks]` executables.
//!
//! The env-var contract is identical for every hook kind: the invoker
//! writes a versioned JSON context to a `0600` file and passes its path
//! via [`FLOX_HOOK_CTX_VAR`], names the hook kind and plugin, and points
//! at a jq the hook may rely on for parsing. `session-wrap` hooks are
//! invoked by the `flox` CLI; `env` and `sidecar` hooks by
//! `flox-activations`/the executive — hence these constants live here
//! rather than in either binary. Design: docs/plugin-lifecycle-hooks.md.

use std::sync::LazyLock;

/// Environment variable holding the path to the serialized hook context.
pub const FLOX_HOOK_CTX_VAR: &str = "FLOX_HOOK_CTX";
/// Environment variable naming the hook being invoked
/// (`session-wrap`, `env`, `sidecar`).
pub const FLOX_HOOK_VAR: &str = "FLOX_HOOK";
/// Environment variable naming the plugin whose hook is invoked.
pub const FLOX_PLUGIN_NAME_VAR: &str = "FLOX_PLUGIN_NAME";
/// Environment variable pointing at the invoking flox binary.
pub const FLOX_BIN_VAR: &str = "FLOX_BIN";
/// Environment variable pointing at a jq the hook may rely on for parsing
/// its ctx, so shell-scripted hooks need not depend on one themselves.
pub const FLOX_HOOK_JQ_VAR: &str = "FLOX_HOOK_JQ";

/// jq bundled at build time for hook consumption (`FLOX_HOOK_JQ`), following
/// the `PROCESS_COMPOSE_BIN` pattern of using our own binaries by absolute
/// path rather than relying on the user's `PATH`. The bake is optional
/// because flox-core is also compiled in nix contexts (package-builder)
/// that neither set `JQ_BIN` nor invoke hooks; binaries that do invoke
/// hooks are built with it set.
pub static JQ_BIN: LazyLock<String> = LazyLock::new(|| {
    std::env::var("JQ_BIN")
        .ok()
        .or_else(|| option_env!("JQ_BIN").map(String::from))
        .unwrap_or_else(|| "jq".to_string())
});
