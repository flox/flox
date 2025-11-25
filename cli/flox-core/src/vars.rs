use std::sync::LazyLock;

pub const FLOX_DISABLE_METRICS_VAR: &str = "FLOX_DISABLE_METRICS";

pub const FLOX_VERSION_VAR: &str = "FLOX_VERSION";

/// The Flox version string.
/// This is _also_ used to determine the version of the CLI.
/// The version is determined by the following rules:
/// 1. `github:flox/flox#flox`, provides a wrapper that sets `FLOX_VERSION`.
///    This is the main production artifact and canonical version.
/// 2. Our `just` targets will set `FLOX_VERSION` using the current git tag,
///    so `just` builds will have the correct updated version _with_ git metadata.
/// 3. `cargo build` when run outside of `just` will fallback to `0.0.0-dirty`.
///    This is the version that also local IDEs / rust-analyzer will use.
///    However, binaries built this way may fail to run in some cases,
///    e.g. `containerize` on macos which relies on the flox version.
pub static FLOX_VERSION_STRING: LazyLock<String> = LazyLock::new(|| {
    // Runtime provided version,
    // i.e. the flox cli wrapper of the nix built production flox package.
    if let Ok(version) = std::env::var(FLOX_VERSION_VAR) {
        return version;
    };

    // Buildtime provided version, i.e. `just build-flox`.
    // Macro requires string literal rather than const.
    if let Some(version) = option_env!("FLOX_VERSION") {
        return version.to_string();
    }

    // Fallback to dev version, to allow building without just,
    // and default configurations in IDEs.
    "0.0.0-dirty".to_string()
});

pub static FLOX_SENTRY_ENV: LazyLock<Option<String>> =
    LazyLock::new(|| std::env::var("FLOX_SENTRY_ENV").ok());
