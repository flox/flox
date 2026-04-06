use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Returns true if profiling is enabled via the `FLOX_PROFILE` env var.
pub fn is_profiling() -> bool {
    std::env::var("FLOX_PROFILE").is_ok_and(|v| !v.is_empty())
}

/// Returns the profile output directory.
///
/// If `_FLOX_PROFILE_DIR` is set, uses that. Otherwise creates a new
/// `/tmp/flox-profile-<pid>/` directory and sets `_FLOX_PROFILE_DIR` for
/// child processes.
pub fn profile_output_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("_FLOX_PROFILE_DIR") {
        return PathBuf::from(dir);
    }

    let dir = PathBuf::from(format!("/tmp/flox-profile-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("failed to create profile output directory");

    // Set for child processes
    unsafe {
        std::env::set_var("_FLOX_PROFILE_DIR", &dir);
    }

    dir
}

/// Creates a Chrome tracing layer that writes to `<profile_dir>/<name>-<pid>.json`.
///
/// Returns `(Option<ChromeLayer>, Option<FlushGuard>)`.
/// Only available when the `profiling` feature is enabled.
#[cfg(feature = "profiling")]
pub fn create_chrome_layer<S>(
    name: &str,
) -> (
    Option<tracing_chrome::ChromeLayer<S>>,
    Option<tracing_chrome::FlushGuard>,
)
where
    S: tracing::Subscriber
        + for<'span> tracing_subscriber::registry::LookupSpan<'span>
        + Send
        + Sync,
{
    if !is_profiling() {
        return (None, None);
    }

    let dir = profile_output_dir();
    let filename = format!("{}-{}.json", name, std::process::id());
    let path = dir.join(&filename);

    // Record wall-clock epoch so the merge tool can align traces across processes.
    // tracing-chrome uses Instant::now() (monotonic) internally, so timestamps
    // are relative to layer creation. We save the wall-clock time at this moment
    // as a reference point.
    let wall_clock_us = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before UNIX epoch")
        .as_micros();
    let epoch_path = dir.join(format!("{}-{}.epoch", name, std::process::id()));
    let _ = std::fs::write(&epoch_path, wall_clock_us.to_string());

    eprintln!(
        "flox profiling: writing trace to {}",
        dir.to_string_lossy()
    );

    let (chrome_layer, guard) = tracing_chrome::ChromeLayerBuilder::new()
        .file(path)
        .include_args(true)
        .build();

    (Some(chrome_layer), Some(guard))
}
