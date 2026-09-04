use std::path::PathBuf;
use std::sync::LazyLock;

/// Path of the running `flox-activations` binary.
///
/// The binary re-invokes itself (`start`, `attach`) and writes its own path
/// into the rc scripts it generates, so it resolves the path at runtime rather
/// than having it compiled in. `FLOX_ACTIVATIONS_BIN` overrides it: the dev
/// shell points it at the cargo-built binary, and the integration tests use it
/// to pin a stable path.
pub static FLOX_ACTIVATIONS_BIN: LazyLock<PathBuf> = LazyLock::new(|| {
    std::env::var("FLOX_ACTIVATIONS_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::current_exe().expect("could not determine the path of the running binary")
        })
});
