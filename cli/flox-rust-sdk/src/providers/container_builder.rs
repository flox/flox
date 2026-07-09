use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use flox_core::activate::mode::ActivateMode;
use flox_manifest::parsed::common::ContainerizeConfig;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use thiserror::Error;
use tracing::{Span, debug, info, instrument};
use tracing_indicatif::span_ext::IndicatifSpanExt;

use super::buildenv::BuiltStorePath;
use crate::flox::Flox;
use crate::providers::build::COMMON_NIXPKGS_URL;
use crate::providers::nix::nix_base_command;
use crate::utils::gomap::GoMap;
use crate::utils::{CommandExt, FLOX_INTERPRETER, ReaderExt};

static MK_CONTAINER_NIX: LazyLock<PathBuf> = LazyLock::new(|| {
    std::env::var("FLOX_MK_CONTAINER_NIX")
        .unwrap_or_else(|_| env!("FLOX_MK_CONTAINER_NIX").to_string())
        .into()
});

pub trait ContainerBuilder {
    type Error: std::error::Error;
    fn create_container_source(
        &self,
        flox: &Flox,
        name: impl AsRef<str>,
        tag: impl AsRef<str>,
    ) -> Result<ContainerSource, Self::Error>;
}

/// OCI representation of our more user-friendly `ManifestContainerizeConfig`.
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "PascalCase")]
pub struct OCIConfig {
    user: Option<String>,
    exposed_ports: Option<GoMap>,
    cmd: Option<Vec<String>>,
    volumes: Option<GoMap>,
    working_dir: Option<String>,
    labels: Option<BTreeMap<String, String>>,
    stop_signal: Option<String>,
}

impl From<ContainerizeConfig> for OCIConfig {
    fn from(config: ContainerizeConfig) -> Self {
        Self {
            user: config.user,
            exposed_ports: config.exposed_ports.map(GoMap::from),
            cmd: config.cmd,
            volumes: config.volumes.map(GoMap::from),
            working_dir: config.working_dir,
            labels: config.labels,
            stop_signal: config.stop_signal,
        }
    }
}

/// The three store paths the store-volume refresh produces and reports.
///
/// The builder-side `flox containerize --store-volume-refresh` prints this as
/// a single line of JSON on stdout; the macOS host parses it back to learn
/// (a) which env bundle and activations-context to run from the volume and
/// (b) which Linux `flox` binary served the request, so the next refresh can
/// exec that binary directly instead of paying another flake unpack.
///
/// All three fields are absolute `/nix/store/…` paths inside the builder's
/// store (the `flox-nix` volume). [`StoreVolumeRefresh::is_valid`] enforces
/// that invariant; the host rejects any line that fails it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreVolumeRefresh {
    /// The `environment-run` / `environment-dev` bundle built from the lock.
    pub env_run: String,
    /// The activations-context store path (mkContainer's `passthru.activateCtx`).
    pub activate_ctx: String,
    /// The Linux `flox` binary that served this refresh (from
    /// `readlink /proc/self/exe`), used to key and populate the host's
    /// binary-resolution cache.
    pub flox_bin: String,
}

impl StoreVolumeRefresh {
    /// Serialize to the single JSON line the builder prints on stdout.
    pub fn to_json_line(&self) -> String {
        serde_json::to_string(self).expect("serializing StoreVolumeRefresh cannot fail")
    }

    /// Parse a builder stdout line, accepting it only when all three fields
    /// are absolute store paths. Extra whitespace around the line is trimmed.
    ///
    /// Returns `None` for malformed JSON or any field that is not a
    /// `/nix/store/…` path, mirroring the rejection-shape discipline the
    /// baked-entrypoint parser uses on the host.
    pub fn parse_line(line: &str) -> Option<Self> {
        let parsed: Self = serde_json::from_str(line.trim()).ok()?;
        parsed.is_valid().then_some(parsed)
    }

    /// True when every field is an absolute `/nix/store/…` path.
    fn is_valid(&self) -> bool {
        [&self.env_run, &self.activate_ctx, &self.flox_bin]
            .iter()
            .all(|p| p.starts_with("/nix/store/"))
    }
}

/// An implementation of [ContainerBuilder] that uses a Nix script
/// to build a native [ContainerSource] from a [BuiltStorePath].
///
/// [MK_CONTAINER_NIX] is a Nix script that uses nixpkgs' `dockerTools.streamLayeredImage`
/// which produces a script which that composes store paths
/// as layers of a container image, and writes the resulting image to `stdout`.
/// [MkContainerNix::create_container_source] performs the build of that script,
/// which in turn builds all the required layers.
/// The resulting [ContainerSource] will execute the script
/// and stream the container image to a given sink via [ContainerSource::stream_container].
///
/// Note: that this implementation is not suitable for building containers on macOS.
///
/// 1. Containers are native _linux_ systems, for which packages can't be built on macOS.
/// 2. The script generated by `dockerTools.streamLayeredImage` is not compatible with macOS.
#[derive(Debug)]
pub struct MkContainerNix {
    store_path: BuiltStorePath,
    activation_mode: ActivateMode,
    container_config: Option<OCIConfig>,
}

#[derive(Debug, Error)]
pub enum MkContainerNixError {
    #[error("failed to call nix")]
    CallNixError(#[source] std::io::Error),

    #[error("failed to build container: {0}")]
    BuildContainerError(String),

    #[error("failed to parse nix build output")]
    ParseBuildOutput(#[source] serde_json::Error),

    #[error("couldn't serialize container config")]
    SerializeContainerConfig(#[source] serde_json::Error),

    #[error("activations-context build produced no store path")]
    EmptyActivateCtxPath,
}

impl MkContainerNix {
    /// Create a new [MkContainerNix] instance that will build a container
    /// from the given [BuiltStorePath].
    /// Generally, this should be the output of a [crate::providers::buildenv::BuildEnv::build].
    ///
    /// Note: this constructor is only available on Linux.
    /// On macOS, use a macOS-specific implementation of [ContainerBuilder].
    #[cfg_attr(
        not(target_os = "linux"),
        deprecated(note = "MkContainerNix is not supported on this platform")
    )]
    pub fn new(
        store_path: BuiltStorePath,
        activation_mode: ActivateMode,
        container_config: Option<OCIConfig>,
    ) -> Self {
        Self {
            store_path,
            activation_mode,
            container_config,
        }
    }

    /// Wire up the argstrs shared by every build against `mkContainer.nix`:
    /// the nixpkgs ref, systems, environment path, activation mode,
    /// interpreter path, container name, and (optional) container config.
    ///
    /// Both the full image build and the activations-context build call this
    /// so the context is guaranteed byte-identical to the one embedded in a
    /// baked image — the shared wiring is the freshness contract that lets a
    /// store-volume run reuse the closure without re-baking.
    ///
    /// The build subcommand, output flags, container tag, and build target
    /// are left to the caller, since those differ between the image and the
    /// context.
    fn add_common_argstrs(
        &self,
        command: &mut Command,
        name: &str,
    ) -> Result<(), MkContainerNixError> {
        command.args(["--argstr", "nixpkgsFlakeRef", COMMON_NIXPKGS_URL.as_str()]);
        command.args(["--argstr", "containerSystem", env!("NIX_TARGET_SYSTEM")]);
        command.args(["--argstr", "system", env!("NIX_TARGET_SYSTEM")]);
        command.args([
            "--argstr",
            "environmentOutPath",
            self.store_path.to_string_lossy().as_ref(),
        ]);
        command.args([
            "--argstr",
            "activationMode",
            &self.activation_mode.to_string(),
        ]);
        command.args([
            "--argstr",
            "interpreterPath",
            (*FLOX_INTERPRETER).to_string_lossy().as_ref(),
        ]);
        command.args(["--argstr", "containerName", name]);
        if let Some(container_config) = &self.container_config {
            command.args([
                "--argstr",
                "containerConfigJSON",
                &serde_json::to_string(container_config)
                    .map_err(MkContainerNixError::SerializeContainerConfig)?,
            ]);
        }
        Ok(())
    }

    /// Build only `passthru.activateCtx` from `mkContainer.nix` and return the
    /// realised store path of the activations-context.
    ///
    /// This is the store-volume refresh's activation-context step: it produces
    /// the same context an image bake would embed, but skips all image
    /// assembly (`streamLayeredImage`, layer packing, skopeo). Because it
    /// reuses [`Self::add_common_argstrs`] — the identical wiring the image
    /// build uses — the resulting context matches the baked one exactly.
    ///
    /// Note: this method is only meaningful on Linux (the builder), matching
    /// [`Self::new`].
    #[cfg_attr(
        not(target_os = "linux"),
        deprecated(note = "MkContainerNix is not supported on this platform")
    )]
    #[instrument(skip_all, fields(name = name.as_ref(), progress = "Building activation context"))]
    pub fn create_activate_ctx(
        &self,
        _flox: &Flox,
        name: impl AsRef<str>,
    ) -> Result<PathBuf, MkContainerNixError> {
        let mut command = nix_base_command();
        command.arg("build");
        command.arg("--json");
        command.arg("--no-link");
        command.arg("--file").arg(&*MK_CONTAINER_NIX);
        self.add_common_argstrs(&mut command, name.as_ref())?;
        command.arg("passthru.activateCtx");
        debug!(cmd=%command.display(), "building activation context");

        let output = command
            .output()
            .map_err(MkContainerNixError::CallNixError)?;
        if !output.status.success() {
            return Err(MkContainerNixError::BuildContainerError(
                String::from_utf8_lossy(&output.stderr).to_string(),
            ));
        }

        // `nix build --json` of a single attrpath returns a one-element array
        // whose element carries the realised `outputs.out` store path.
        #[derive(Debug, Clone, Deserialize)]
        struct ActivateCtxOutputs {
            out: PathBuf,
        }

        #[derive(Debug, Clone, Deserialize)]
        struct ActivateCtxResultRaw {
            outputs: ActivateCtxOutputs,
        }

        let [raw @ ActivateCtxResultRaw { .. }] = serde_json::from_slice(&output.stdout)
            .map_err(MkContainerNixError::ParseBuildOutput)?;
        let activate_ctx_path = raw.outputs.out;
        if activate_ctx_path.as_os_str().is_empty() {
            return Err(MkContainerNixError::EmptyActivateCtxPath);
        }
        Ok(activate_ctx_path)
    }
}

impl ContainerBuilder for MkContainerNix {
    type Error = MkContainerNixError;

    /// Create a [ContainerSource] that will assemble a container image.
    /// The container image will be built from the provided [BuiltStorePath],
    /// which is generally the output of a [crate::providers::buildenv::BuildEnv::build].
    /// The streaming script will be built via `nix build`.
    #[instrument(skip_all, fields(
        name = name.as_ref(),
        tag = tag.as_ref(),
        progress = "Building container layers"))]
    fn create_container_source(
        &self,
        _flox: &Flox,
        name: impl AsRef<str>,
        tag: impl AsRef<str>,
    ) -> Result<ContainerSource, Self::Error> {
        let mut command = nix_base_command();
        command.arg("build");
        command.arg("--json");
        command.arg("--no-link");
        command.arg("--file").arg(&*MK_CONTAINER_NIX);
        self.add_common_argstrs(&mut command, name.as_ref())?;
        // The tag applies only to the assembled image, not the activation
        // context, so it lives here rather than in the shared wiring.
        command.args(["--argstr", "containerTag", tag.as_ref()]);
        debug!(cmd=%command.display(), "building container");

        let output = command
            .output()
            .map_err(MkContainerNixError::CallNixError)?;
        if !output.status.success() {
            return Err(MkContainerNixError::BuildContainerError(
                String::from_utf8_lossy(&output.stderr).to_string(),
            ));
        }

        // defined inline as an implementation detail
        #[derive(Debug, Clone, Deserialize)]
        struct MkContainerOutputs {
            out: PathBuf,
        }

        #[derive(Debug, Clone, Deserialize)]
        struct MkContainerResultRaw {
            outputs: MkContainerOutputs,
        }

        let [raw @ MkContainerResultRaw { .. }] = serde_json::from_slice(&output.stdout)
            .map_err(MkContainerNixError::ParseBuildOutput)?;
        let container_builder_script_path = raw.outputs.out;
        let container_source = ContainerSource::new(Command::new(container_builder_script_path));

        Ok(container_source)
    }
}

/// Minimum interval between spinner label updates driven by builder stderr.
/// Nix build output can be very chatty; throttling prevents flicker.
const SPINNER_UPDATE_INTERVAL: Duration = Duration::from_millis(500);

/// Maximum length of a spinner label snippet taken from a builder line.
const SPINNER_SNIPPET_MAX_CHARS: usize = 60;

/// Rate-limits spinner label updates derived from subprocess output lines.
///
/// Blank lines never produce an update. Non-blank lines produce a truncated
/// snippet at most once per interval; suppressed lines do not reset the
/// interval timer.
#[derive(Debug)]
struct SpinnerThrottle {
    interval: Duration,
    last_update: Option<Instant>,
}

impl SpinnerThrottle {
    fn new(interval: Duration) -> Self {
        Self {
            interval,
            last_update: None,
        }
    }

    /// Returns a spinner-ready snippet (trimmed, truncated to
    /// [`SPINNER_SNIPPET_MAX_CHARS`]) when `line` should update the label,
    /// or `None` when the line is blank or arrives before the interval has
    /// elapsed since the last accepted line.
    fn accept(&mut self, line: &str, now: Instant) -> Option<String> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return None;
        }
        if let Some(last) = self.last_update
            && now.duration_since(last) < self.interval
        {
            return None;
        }
        self.last_update = Some(now);
        Some(trimmed.chars().take(SPINNER_SNIPPET_MAX_CHARS).collect())
    }
}

/// Type representing a container source,
/// i.e. a command that writes a container tarball to stdout.
/// This is typically created by [ContainerBuilder::create_container_source].
#[derive(Debug)]
pub struct ContainerSource {
    source_command: Command,
}

impl ContainerSource {
    pub fn new(source_command: Command) -> Self {
        Self { source_command }
    }

    /// Run the container builder script
    /// and write the container tarball to the given sink.
    ///
    /// Stderr from the builder process is forwarded via `info!` for verbose
    /// output (`-v`). Concurrently, the latest interesting builder line is
    /// surfaced (throttled) as the progress bar message of this function's
    /// span via [`IndicatifSpanExt::pb_set_message`], so the user sees motion
    /// during the long compile stage without needing `-v`. The logger's
    /// progress template renders the message after the static stage label
    /// carried by the `progress` field.
    #[instrument(skip_all, fields(command = ?self.source_command, progress = "[2/3] Writing container layers"))]
    pub fn stream_container(self, sink: &mut impl Write) -> Result<(), ContainerSourceError> {
        let mut container_source_command = self.source_command;

        // ensure the command writes to stdout
        container_source_command.stdout(Stdio::piped());
        container_source_command.stderr(Stdio::piped());

        debug!(
            "running container source command: {}",
            container_source_command.display()
        );

        let mut handle = container_source_command
            .spawn()
            .map_err(ContainerSourceError::CallContainerSourceCommand)?;

        // The span handle carries its own subscriber reference, so
        // pb_set_message works from the tap thread below (and gracefully
        // no-ops when no indicatif layer is registered, e.g. in tests).
        let span = Span::current();
        let throttle = Mutex::new(SpinnerThrottle::new(SPINNER_UPDATE_INTERVAL));

        let tap = handle
            .stderr
            .take()
            .expect("stderr set to piped")
            .tap_lines(move |line| {
                // Forward to info! so -v users still see the raw output.
                info!("{line}");

                let accepted = throttle
                    .lock()
                    .expect("spinner throttle mutex poisoned")
                    .accept(line, Instant::now());
                if let Some(snippet) = accepted {
                    span.pb_set_message(&snippet);
                }
            });

        let mut stdout = handle.stdout.take().expect("stdout set to piped");

        io::copy(&mut stdout, sink).map_err(ContainerSourceError::StreamContainer)?;

        let status = handle
            .wait()
            .map_err(ContainerSourceError::CallContainerSourceCommand)?;

        if !status.success() {
            let stderr_content = tap.wait();
            return Err(ContainerSourceError::CommandUnsuccessful(stderr_content));
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum ContainerSourceError {
    #[error("failed to call container source command")]
    CallContainerSourceCommand(#[source] std::io::Error),
    #[error("failed to stream container to sink")]
    StreamContainer(#[source] std::io::Error),
    #[error(
        "container source command unsuccessful\n\
        \n\
        stderr:\n\
        {0}"
    )]
    CommandUnsuccessful(String),
}

#[cfg(test)]
mod spinner_throttle_tests {
    use std::io::Cursor;
    use std::sync::Arc;

    use super::*;

    #[test]
    fn accepts_after_interval_elapses() {
        let mut throttle = SpinnerThrottle::new(Duration::from_millis(100));
        let t0 = Instant::now();
        assert_eq!(throttle.accept("first", t0), Some("first".to_string()));
        assert_eq!(
            throttle.accept("too soon", t0 + Duration::from_millis(50)),
            None
        );
        assert_eq!(
            throttle.accept("later", t0 + Duration::from_millis(150)),
            Some("later".to_string())
        );
    }

    #[test]
    fn skips_blank_lines_without_consuming_interval() {
        let mut throttle = SpinnerThrottle::new(Duration::from_millis(100));
        let t0 = Instant::now();
        assert_eq!(throttle.accept("   ", t0), None);
        assert_eq!(throttle.accept("", t0), None);
        // Blank lines do not start the interval; the first real line is
        // accepted immediately.
        assert_eq!(throttle.accept("real", t0), Some("real".to_string()));
    }

    #[test]
    fn truncates_long_lines_to_snippet_length() {
        let mut throttle = SpinnerThrottle::new(Duration::ZERO);
        let long = "x".repeat(SPINNER_SNIPPET_MAX_CHARS + 20);
        let snippet = throttle.accept(&long, Instant::now()).unwrap();
        assert_eq!(snippet.chars().count(), SPINNER_SNIPPET_MAX_CHARS);
    }

    #[test]
    fn gates_tap_lines_updates_to_one_per_interval() {
        // All lines from an in-memory reader arrive well within one
        // interval, so only the first non-blank line may produce an update.
        let input = "unpacking sources\n\nbuilding\ninstalling\n";
        let throttle = Mutex::new(SpinnerThrottle::new(Duration::from_secs(60)));
        let updates = Arc::new(Mutex::new(Vec::new()));
        let updates_in_tap = Arc::clone(&updates);
        let tap = Cursor::new(input.as_bytes().to_vec()).tap_lines(move |line| {
            let accepted = throttle.lock().unwrap().accept(line, Instant::now());
            if let Some(snippet) = accepted {
                updates_in_tap.lock().unwrap().push(snippet);
            }
        });
        tap.wait();
        assert_eq!(*updates.lock().unwrap(), vec![
            "unpacking sources".to_string()
        ]);
    }
}

#[cfg(test)]
mod container_source_tests {
    use std::collections::BTreeSet;
    use std::fs::{self, File};
    use std::os::unix::fs::PermissionsExt;

    use indoc::indoc;
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn oci_config_from_manifest() {
        let manifest_config = ContainerizeConfig {
            user: Some("root".to_string()),
            exposed_ports: Some(BTreeSet::from(["80/tcp".to_string()])),
            volumes: Some(BTreeSet::from(["/app".to_string()])),
            working_dir: Some("/app".to_string()),
            labels: Some(BTreeMap::from([
                ("app".to_string(), "myapp".to_string()),
                ("version".to_string(), "1.0".to_string()),
            ])),
            ..Default::default()
        };
        let oci_config: OCIConfig = manifest_config.into();
        let json = serde_json::to_string_pretty(&oci_config).unwrap();

        // Selection of fields that verify From + Serialize:
        // - omits fields that are `None`
        // - renames keys from kebab-case to PascalCase
        // - converts values from `BTreeSet` to `GoMap`
        assert_eq!(json, indoc! {r#"{
          "User": "root",
          "ExposedPorts": {
            "80/tcp": {}
          },
          "Volumes": {
            "/app": {}
          },
          "WorkingDir": "/app",
          "Labels": {
            "app": "myapp",
            "version": "1.0"
          }
        }"#});
    }

    /// OS error 26 is "Text file busy",
    /// which can happen when executing a script
    /// that is has been written to immediately before.
    /// We typically see this in tests, where we write
    /// a new script and immediately execute it.
    /// In production use, this should not happen as the script
    /// will be written by a different process (`nix`).
    ///
    /// <https://github.com/rust-lang/rust/issues/114554>
    const ERR_TEXT_FILE_BUSY: i32 = 26;

    const TEST_BUILDER: &str = indoc! {r#"
        #!/usr/bin/env bash
        echo "hello world"
    "#};

    fn create_test_script() -> (TempDir, PathBuf) {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("flox-test-container-builder");
        std::fs::write(&path, TEST_BUILDER).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        (tempdir, path)
    }

    #[test]
    fn test_writes_output_to_writer() {
        let (_tempdir, test_script) = create_test_script();

        let mut buf = Vec::new();

        let mut tries = 0;
        loop {
            if tries >= 3 {
                panic!("Test flaked with 'Text file busy' and can be re-run")
            }
            let container_builder = ContainerSource::new(Command::new(&test_script));
            match container_builder.stream_container(&mut buf) {
                Err(ContainerSourceError::CallContainerSourceCommand(e))
                    if e.raw_os_error() == Some(ERR_TEXT_FILE_BUSY) =>
                {
                    dbg!("Text file busy -- ignored");
                    tries += 1;
                    continue;
                },
                result => break result.unwrap(),
            }
        }
        assert_eq!(buf, b"hello world\n");
    }

    #[test]
    fn test_allows_forwarding_to_file() {
        let (tempdir, test_script) = create_test_script();
        let output_path = tempdir.path().join("output");

        let mut file = File::create(&output_path).unwrap();

        // looping to ignore "Text file busy" errors
        // see the comment on `ERR_TEXT_FILE_BUSY` for more information
        let mut tries = 0;
        loop {
            if tries >= 3 {
                panic!("Test flaked with 'Text file busy' and can be re-run")
            }
            let container_builder = ContainerSource::new(Command::new(&test_script));
            match container_builder.stream_container(&mut file) {
                Err(ContainerSourceError::CallContainerSourceCommand(e))
                    if e.raw_os_error() == Some(ERR_TEXT_FILE_BUSY) =>
                {
                    dbg!("Text file busy -- ignored");
                    tries += 1;
                    continue;
                },
                result => break result.unwrap(),
            }
        }
        drop(file);

        let output = fs::read_to_string(&output_path).unwrap();
        assert_eq!(output, "hello world\n");
    }
}

#[cfg(test)]
mod store_volume_refresh_tests {
    use super::*;

    fn valid_refresh() -> StoreVolumeRefresh {
        StoreVolumeRefresh {
            env_run: "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-environment-run".to_string(),
            activate_ctx: "/nix/store/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-activations-context"
                .to_string(),
            flox_bin: "/nix/store/cccccccccccccccccccccccccccccccc-flox-1.6.0/bin/flox".to_string(),
        }
    }

    #[test]
    fn round_trips_through_json_line() {
        let refresh = valid_refresh();
        let parsed = StoreVolumeRefresh::parse_line(&refresh.to_json_line())
            .expect("valid refresh should parse");
        assert_eq!(parsed, refresh);
    }

    #[test]
    fn parses_line_with_surrounding_whitespace() {
        let refresh = valid_refresh();
        let line = format!("  \n{}\n  ", refresh.to_json_line());
        assert_eq!(StoreVolumeRefresh::parse_line(&line), Some(refresh));
    }

    #[test]
    fn rejects_malformed_json() {
        assert_eq!(StoreVolumeRefresh::parse_line("not json"), None);
        assert_eq!(StoreVolumeRefresh::parse_line("{\"env_run\":}"), None);
    }

    #[test]
    fn rejects_missing_field() {
        let line = r#"{"env_run":"/nix/store/aaaa-environment-run","activate_ctx":"/nix/store/bbbb-activations-context"}"#;
        assert_eq!(StoreVolumeRefresh::parse_line(line), None);
    }

    #[test]
    fn rejects_non_store_path_field() {
        let mut refresh = valid_refresh();
        refresh.flox_bin = "/usr/local/bin/flox".to_string();
        assert_eq!(
            StoreVolumeRefresh::parse_line(&refresh.to_json_line()),
            None
        );

        let mut refresh = valid_refresh();
        refresh.env_run = "environment-run".to_string();
        assert_eq!(
            StoreVolumeRefresh::parse_line(&refresh.to_json_line()),
            None
        );
    }
}
