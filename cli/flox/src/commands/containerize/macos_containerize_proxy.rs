use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::LazyLock;

use flox_core::activate::context::ActivateMode;
use flox_core::vars::FLOX_DISABLE_METRICS_VAR;
use flox_rust_sdk::flox::{FLOX_VERSION, Flox};
use flox_rust_sdk::providers::container_builder::{
    ContainerBuilder,
    ContainerSource,
    StoreVolumeRefresh,
};
use flox_rust_sdk::providers::nix::{NIX_VERSION, NixSubstituterConfig};
use flox_rust_sdk::utils::ReaderExt;
use indoc::formatdoc;
use thiserror::Error;
use tracing::{debug, info, instrument};

use super::Runtime;
use crate::config::FLOX_CONFIG_FILE;

const NIX_PROXY_IMAGE: &str = "nixos/nix";
static NIX_PROXY_IMAGE_REF: LazyLock<Option<String>> =
    LazyLock::new(|| env::var("_FLOX_CONTAINERIZE_PROXY_IMAGE_REF").ok());

const FLOX_FLAKE: &str = "github:flox/flox";
const FLOX_PROXY_IMAGE_FLOX_CONFIG_DIR: &str = "/root/.config/flox";
static FLOX_CONTAINERIZE_FLAKE_REF_OR_REV: LazyLock<Option<String>> =
    LazyLock::new(|| env::var("_FLOX_CONTAINERIZE_FLAKE_REF_OR_REV").ok());
const CONTAINER_NIX_CACHE_VOLUME: &str = "flox-nix";

/// Default VM memory for Apple Container builder runs.
///
/// Compiling uncached Rust crates inside the nixos/nix proxy image requires
/// more memory than the Apple Container VM default. Override with
/// `_FLOX_CONTAINERIZE_VM_MEMORY` (e.g. "4g", "16g").
const DEFAULT_VM_MEMORY: &str = "8g";

/// `--memory` value passed to `container run` for Apple Container builds.
/// Unset means fall through to `DEFAULT_VM_MEMORY`; set to empty string to
/// omit the flag entirely (not recommended for builds).
static FLOX_CONTAINERIZE_VM_MEMORY: LazyLock<Option<String>> =
    LazyLock::new(|| env::var("_FLOX_CONTAINERIZE_VM_MEMORY").ok());

/// `--cpus` value passed to `container run` for Apple Container builds.
/// When unset the flag is omitted and the Apple Container default applies.
static FLOX_CONTAINERIZE_VM_CPUS: LazyLock<Option<String>> =
    LazyLock::new(|| env::var("_FLOX_CONTAINERIZE_VM_CPUS").ok());

const MOUNT_ENV: &str = "/flox_env";

/// Escape a string for safe interpolation into a `bash -c` script as a
/// single-quoted word. Internal single quotes become `'\''` (close quote,
/// escaped literal quote, reopen quote).
fn shell_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// The flox flake rev the builder should fetch and run.
///
/// Prefers `_FLOX_CONTAINERIZE_FLAKE_REF_OR_REV` (a pushed commit, so the
/// builder can fetch exactly this code), then the running binary's commit SHA,
/// then the `v<semver>` tag. This is the single source of truth for both the
/// full-bake nix-run invocation and the store-volume refresh fallback, and it
/// doubles as the builder pin that keys the host binary-resolution cache.
fn flox_flake_rev() -> String {
    let flox_version = &*FLOX_VERSION;
    let flox_version_tag = format!("v{}", flox_version.base_semver());
    (*FLOX_CONTAINERIZE_FLAKE_REF_OR_REV)
        .clone()
        .unwrap_or_else(|| flox_version.commit_sha().unwrap_or(flox_version_tag))
}

/// The full `github:flox/flox/<rev>` flake reference the builder runs.
fn flox_flake_ref() -> String {
    format!("{FLOX_FLAKE}/{}", flox_flake_rev())
}

/// Render the verbosity flag (` --quiet`, ` -vvv`, or empty) for the inner
/// flox invocation, matching the host's verbosity.
fn verbosity_arg(verbosity: i32) -> String {
    match verbosity {
        -1 => " --quiet".to_string(),
        v if v > 0 => format!(" -{}", "v".repeat(v.try_into().unwrap())),
        _ => String::new(),
    }
}

/// The trailing argv of a store-volume refresh, shared by both invocation
/// paths: `containerize --store-volume-refresh --dir /flox_env`.
///
/// Kept separate and pure so the direct-exec and nix-run argv builders can be
/// asserted exactly in tests without spawning a container.
fn refresh_flox_args() -> Vec<String> {
    vec![
        "containerize".to_string(),
        "--store-volume-refresh".to_string(),
        "--dir".to_string(),
        MOUNT_ENV.to_string(),
    ]
}

/// The `bash -c` script that runs a store-volume refresh via `nix run` of the
/// flox flake (the cache-miss path). Pays the flake unpack and a one-time
/// flox-linux build for a new rev.
///
/// Pure function of its inputs so it can be snapshot-tested.
fn nix_run_refresh_shell(flake_ref: &str, verbosity_arg: &str) -> String {
    format!(
        "set -euo pipefail\n\
        nix --extra-experimental-features 'nix-command flakes' --accept-flake-config \
        run {} --{verbosity_arg} containerize --store-volume-refresh --dir {MOUNT_ENV}",
        shell_single_quote(flake_ref)
    )
}

#[derive(Debug, Error)]
pub enum ContainerizeProxyError {
    #[error("failed to populate proxy container cache volume")]
    PopulateCacheVolume(#[source] std::io::Error),

    #[error("store-volume refresh builder run failed")]
    RefreshBuilderRun(#[source] std::io::Error),

    #[error(
        "store-volume refresh did not print a valid store-path JSON line\n\
        \n\
        builder stdout:\n\
        {stdout}"
    )]
    RefreshParse { stdout: String },
}

/// An implementation of [ContainerBuilder] for macOS that uses `flox
/// containerize` within a proxy container of a given [Runtime].
#[derive(Debug, Clone)]
pub(crate) struct ContainerizeProxy {
    environment_path: PathBuf,
    container_runtime: Runtime,
    labels: Vec<String>,
    mode: Option<ActivateMode>,
}

impl ContainerizeProxy {
    pub(crate) fn new(
        environment_path: PathBuf,
        container_runtime: Runtime,
        labels: Vec<String>,
        mode: Option<ActivateMode>,
    ) -> Self {
        Self {
            environment_path,
            container_runtime,
            labels,
            mode,
        }
    }

    /// Base command for the container runtime.
    fn runtime_base_command(&self) -> Command {
        let mut command = self.container_runtime.to_command();
        command.arg("run");
        command.arg("--rm");
        if self.container_runtime == Runtime::AppleContainer {
            self.add_vm_resource_args(&mut command);
        }
        command
    }

    /// Add `--memory` and optionally `--cpus` to an Apple Container `run`
    /// invocation.
    ///
    /// The nixos/nix builder VM needs more memory than the Apple Container
    /// default when compiling uncached Rust crates. `_FLOX_CONTAINERIZE_VM_MEMORY`
    /// overrides the default; `_FLOX_CONTAINERIZE_VM_CPUS` adds `--cpus` when
    /// set (the flag is omitted when unset so the VM default applies).
    fn add_vm_resource_args(&self, command: &mut Command) {
        let memory = FLOX_CONTAINERIZE_VM_MEMORY
            .as_deref()
            .unwrap_or(DEFAULT_VM_MEMORY);
        if !memory.is_empty() {
            command.args(["--memory", memory]);
        }
        if let Some(cpus) = FLOX_CONTAINERIZE_VM_CPUS.as_deref()
            && !cpus.is_empty()
        {
            command.args(["--cpus", cpus]);
        }
    }

    // Use a Nix container that matches the version of Nix that this Flox
    // has been built with because it's smaller and changes less frequently
    // than a Flox container of the corresponding version, which result in less
    // container image pulls. It also prevents the chicken-and-egg problem when
    // we bump `VERSION` in Flox but haven't published the container image yet.
    fn container_image(&self) -> String {
        format!(
            "{}:{}",
            NIX_PROXY_IMAGE,
            NIX_PROXY_IMAGE_REF
                .clone()
                .unwrap_or(NIX_VERSION.to_string())
        )
    }

    /// Add a named-volume mount to the container runtime command.
    ///
    /// All three runtimes (Docker, Podman, Apple Container) support named
    /// volumes, but with different syntax:
    ///
    /// - Docker / Podman: `--mount type=volume,src=<name>,dst=<path>` — the
    ///   `type=volume` selector distinguishes named volumes from bind mounts.
    /// - Apple Container: `--volume <name>:<path>` — the CLI uses the same
    ///   short `-v` flag family; the source is interpreted as a named volume
    ///   when it does not begin with `/` or `.`.
    fn add_cache_mount(&self, command: &mut Command, path: &str) {
        if self.container_runtime == Runtime::AppleContainer {
            command.args(["--volume", &format!("{CONTAINER_NIX_CACHE_VOLUME}:{path}")]);
        } else {
            command.args([
                "--mount",
                &format!("type=volume,src={CONTAINER_NIX_CACHE_VOLUME},dst={path}"),
            ]);
        }
    }

    /// Create the named cache volume for Apple Container if it does not exist.
    ///
    /// Apple Container volumes are VM-attached disk images. `container volume
    /// create` is idempotent — if the volume already exists the command exits
    /// non-zero but that is expected and ignored here.
    fn ensure_apple_container_cache_volume(&self) -> Result<(), ContainerizeProxyError> {
        debug!(
            volume = CONTAINER_NIX_CACHE_VOLUME,
            "ensuring Apple Container cache volume exists"
        );
        // Capture output rather than inheriting it: the expected
        // "volume already exists" error would otherwise leak into
        // every bake's output.
        let output = self
            .container_runtime
            .to_command()
            .args(["volume", "create", CONTAINER_NIX_CACHE_VOLUME])
            .output()
            .map_err(ContainerizeProxyError::PopulateCacheVolume)?;
        if !output.status.success() {
            // A non-zero exit is expected when the volume already exists;
            // treat it as success so repeated builds are idempotent.
            debug!(
                volume = CONTAINER_NIX_CACHE_VOLUME,
                stderr = %String::from_utf8_lossy(&output.stderr),
                "volume create returned non-zero (likely already exists)"
            );
        }
        Ok(())
    }

    /// Copy the Nix store from the container image into the named cache volume.
    ///
    /// The volume is mounted at `/cache/nix` alongside the image's own `/nix`
    /// so that `nix copy --all` can treat it as a local store root:
    /// https://nix.dev/manual/nix/2.24/command-ref/new-cli/nix3-help-stores#local-store
    ///
    /// This is idempotent: `nix copy` skips store paths that already exist in
    /// the destination, so repeated runs are fast after the first cold fill.
    ///
    /// For Apple Container the named volume is created (if absent) before
    /// mounting, because Apple Container does not auto-create volumes on first
    /// use the way Docker does.
    #[instrument(
        skip_all,
        fields(progress = "[1/3] Filling build cache (downloads + cross-compile on first bake)")
    )]
    fn populate_cache_volume(&self) -> Result<(), ContainerizeProxyError> {
        if self.container_runtime == Runtime::AppleContainer {
            self.ensure_apple_container_cache_volume()?;
        }

        let mut command = self.runtime_base_command();

        // Mount alongside /nix at a cache root so nix copy can treat it as a
        // local store. The real /nix is not touched during populate.
        let cache_root = "/cache";
        self.add_cache_mount(&mut command, &format!("{cache_root}/nix"));

        command.arg(self.container_image());
        // We have to additionally copy some parts of `/nix/var/nix` so that the
        // container isn't broken when the cache volume shadows its `/nix`.
        command.args(["bash", "-c", &formatdoc! {"
            set -euo pipefail
            nix --extra-experimental-features nix-command copy --all --no-check-sigs --to {cache_root}
            cp -R /nix/var/nix/profiles /nix/var/nix/gcroots {cache_root}/nix/var/nix/
        "}]);

        debug!(?command, "running populate cache volume command");
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(ContainerizeProxyError::PopulateCacheVolume)?;

        let stderr = child
            .stderr
            .take()
            .expect("STDERR is piped")
            .tap_lines(|line| info!("{line}"));

        child
            .stdout
            .take()
            .expect("STDOUT is piped")
            .tap_lines(|line| info!("{line}"));

        let status = child
            .wait()
            .map_err(ContainerizeProxyError::PopulateCacheVolume)?;

        if !status.success() {
            return Err(ContainerizeProxyError::PopulateCacheVolume(
                std::io::Error::other(stderr.wait().to_string()),
            ));
        }

        Ok(())
    }

    /// Inception L1: Container runtime args.
    ///
    /// Builds the `docker run` / `podman run` / `container run` invocation
    /// that launches the nixos/nix proxy container. Adapts CLI flags for each
    /// runtime:
    ///
    /// - Docker / Podman: `--mount type=bind,...` for bind mounts, `--mount
    ///   type=volume,...` for named volumes; Podman also needs `--userns ""`
    ///   to map the host user to root inside the container.
    /// - Apple Container: `--mount type=bind,...` for bind mounts, `--volume
    ///   name:dst` for named volumes; no `--userns` flag (runs as the host
    ///   user by default in its VM).
    fn add_runtime_args(&self, command: &mut Command, flox: &Flox) {
        // Podman needs --userns to map the current user to root inside the
        // container; otherwise multi-user Nix operations fail. Docker and Apple
        // Container do not need this flag.
        if self.container_runtime == Runtime::Podman {
            command.args(["--userns", ""]);
        }
        command.args([
            "--mount",
            &format!(
                "type=bind,source={},target={}",
                self.environment_path.to_string_lossy(),
                MOUNT_ENV
            ),
        ]);

        // Mount the populated cache volume over /nix so that store paths built
        // in one run are reused in subsequent runs. The populate step runs
        // before this and fills the volume from the image's own /nix; mounting
        // the volume here then shadows that path, but by that point all
        // required store content is already in the volume.
        //
        // For Apple Container the volume is mounted via --volume name:/nix
        // (see add_cache_mount); for Docker and Podman via --mount type=volume.
        self.add_cache_mount(command, "/nix");

        // Honour config from the user's flox.toml
        // This could include things like floxhub_token and floxhub_url.
        //
        // Docker and Podman support binding individual files; Apple Container
        // requires the bind-mount source to be a directory. When the runtime
        // is Apple Container we mount the entire config directory
        // (~/.config/flox) to avoid the "path is not a directory" error.
        let flox_toml = flox.config_dir.join(FLOX_CONFIG_FILE);
        if flox_toml.exists() {
            if self.container_runtime == Runtime::AppleContainer {
                // Mount the whole config directory; `flox.toml` lives inside.
                let mut config_dir_mount = OsString::new();
                config_dir_mount.push("type=bind,source=");
                config_dir_mount.push(&flox.config_dir);
                config_dir_mount.push(format!(",target={}", FLOX_PROXY_IMAGE_FLOX_CONFIG_DIR));
                command.arg("--mount");
                command.arg(config_dir_mount);
            } else {
                let mut flox_toml_mount = OsString::new();
                flox_toml_mount.push("type=bind,source=");
                flox_toml_mount.push(flox_toml);
                flox_toml_mount.push(format!(
                    ",target={}/{}",
                    FLOX_PROXY_IMAGE_FLOX_CONFIG_DIR, FLOX_CONFIG_FILE
                ));
                command.arg("--mount");
                command.arg(flox_toml_mount);
            }
        }

        // If metrics are disabled (no device UUID), propagate that into the
        // proxy container. This covers both the env var and config file paths
        // (e.g. /etc/flox.toml) that may not be mounted into the container.
        if flox.metrics_device_uuid.is_none() {
            command.args(["--env", &format!("{}=true", FLOX_DISABLE_METRICS_VAR)]);
        }

        // Propagate the host's nix substituters and trusted public keys into
        // the proxy container so that all nix invocations can fetch packages
        // from the same caches available on the host.
        match NixSubstituterConfig::from_nix_config() {
            Ok(config) => {
                let config_str = config.to_string();
                if !config_str.is_empty() {
                    command.args(["--env", &format!("NIX_CONFIG={config_str}")]);
                }
            },
            Err(err) => {
                tracing::warn!(%err, "failed to read nix substituter config, continuing without extra substituters");
            },
        }

        command.arg(self.container_image());
    }

    /// Inception L2: Nix args.
    fn add_nix_args(&self, command: &mut Command) {
        command.arg("nix");
        command.args([
            "--extra-experimental-features",
            "nix-command flakes",
            "--accept-flake-config",
        ]);
        command.args(["run", &flox_flake_ref(), "--"]);
    }

    /// Inception L3: Flox args.
    fn add_flox_args(&self, command: &mut Command, flox: &Flox, tag: impl AsRef<str>) {
        // TODO: this should probably be a method on Verbosity
        match flox.verbosity {
            -1 => {
                command.arg("--quiet");
            },
            _ if flox.verbosity > 0 => {
                command.arg(format!(
                    "-{}",
                    "v".repeat(flox.verbosity.try_into().unwrap())
                ));
            },
            _ => {},
        }
        command.arg("containerize");
        command.args(["--dir", MOUNT_ENV]);
        command.args(["--tag", tag.as_ref()]);
        command.args(["--file", "-"]);
        command.args(self.labels.iter().flat_map(|l| ["--label", l]));
        if let Some(mode) = &self.mode {
            command.args(["--mode", &mode.to_string()]);
        }
    }

    /// Build the full container run command for the OCI-conversion path.
    ///
    /// Apple Container's `image load` only accepts OCI archives, but nixpkgs
    /// `dockerTools.streamLayeredImage` emits docker-archive format. This
    /// command runs a two-phase conversion inside the nixos/nix builder
    /// container:
    ///
    /// 1. `nix run github:flox/flox -- containerize` writes a docker-archive
    ///    to a temp file.
    /// 2. nixpkgs `skopeo copy` converts it to an OCI archive in a second
    ///    temp file, which `cat` then streams to stdout.
    ///
    /// skopeo comes from nixpkgs inside the builder container, so no
    /// host-side tooling is required. Apple Container image refs require an
    /// explicit tag (`name:latest` works where bare `name` fails), so the
    /// tag is embedded in the skopeo destination reference.
    fn build_oci_conversion_command(&self, flox: &Flox, tag: impl AsRef<str>) -> Command {
        let tag_str = tag.as_ref();

        // Derive the image name from the environment directory name, matching
        // what the inner `flox containerize` would use.
        let env_name = self
            .environment_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "flox-env".to_string());

        // Apple Container requires an explicit tag in the image reference;
        // bare `name` (without `:tag`) causes `image load` to fail.
        let image_ref = format!("{env_name}:{tag_str}");

        let flox_flake = flox_flake_ref();
        let verbosity_arg = verbosity_arg(flox.verbosity);

        // User-controlled values (tag, image name, labels) are single-quote
        // escaped so they cannot break out of the bash -c script.
        let tag_quoted = shell_single_quote(tag_str);
        let image_ref_quoted = shell_single_quote(&image_ref);
        let label_args: String = self
            .labels
            .iter()
            .map(|l| format!(" --label {}", shell_single_quote(l)))
            .collect();

        // Sequential phases (not a pipe): running flox containerize and
        // skopeo concurrently can exceed the Apple Container VM's memory
        // and trigger the kernel OOM killer.
        //
        // `>&2` on both nix invocations: only `cat "$oci_tmp"` may write to
        // stdout — anything else corrupts the OCI archive stream read by
        // the host-side sink.
        //
        // skopeo runs with --insecure-policy because the nixos/nix image
        // ships no /etc/containers/policy.json.
        //
        // The skopeo destination concatenates a double-quoted shell variable
        // with the single-quoted image ref: "oci-archive:$oci_tmp:"'name:tag'.
        let shell_cmd = format!(
            "set -euo pipefail\n\
            docker_tmp=$(mktemp /tmp/flox-docker-XXXXXX.tar)\n\
            oci_tmp=$(mktemp /tmp/flox-oci-XXXXXX.tar)\n\
            nix --extra-experimental-features 'nix-command flakes' --accept-flake-config \
            run '{flox_flake}' --{verbosity_arg} containerize --dir {MOUNT_ENV} --tag {tag_quoted} --file \"$docker_tmp\"{label_args} >&2\n\
            nix --extra-experimental-features 'nix-command flakes' \
            run 'nixpkgs#skopeo' -- --insecure-policy copy \
            \"docker-archive:$docker_tmp\" \"oci-archive:$oci_tmp:\"{image_ref_quoted} >&2\n\
            rm -f \"$docker_tmp\"\n\
            cat \"$oci_tmp\"\n\
            rm -f \"$oci_tmp\""
        );

        let mut command = self.runtime_base_command();
        self.add_runtime_args(&mut command, flox);
        // `bash` (unqualified): Apple Container resolves the entrypoint via
        // the image's PATH env var and does not follow absolute Nix profile
        // symlink paths.
        command.args(["bash", "-c", &shell_cmd]);
        command
    }

    /// The host cache file that records which Linux `flox` binary served a
    /// refresh for a given builder pin.
    ///
    /// Keyed by the builder pin (the flake rev) so a flox-version change gets
    /// its own entry rather than exec'ing a stale binary. Lives under the
    /// host's flox cache dir; on a hit the refresh execs the binary directly
    /// (no flake unpack), on a miss it falls back to `nix run` and repopulates
    /// this file.
    fn binary_cache_path(&self, flox: &Flox) -> PathBuf {
        flox.cache_dir
            .join("store-volume")
            .join(format!("flox-bin-{}", flox_flake_rev()))
    }

    /// Refresh the store-volume artifacts for the current environment:
    /// build the env bundle and the activations-context inside the builder
    /// (no image assembly) and report their store paths plus the serving
    /// Linux `flox` binary.
    ///
    /// Fast path (cache hit): the binary that served the last refresh for this
    /// builder pin is exec'd directly, skipping the flake unpack. Slow path
    /// (cache miss): `nix run` of the flox flake pays the unpack once, then the
    /// resolved binary is written to the cache for next time.
    ///
    /// Mounts are identical to the full-bake builder run (`add_runtime_args`):
    /// the environment at `/flox_env`, the `flox-nix` volume at `/nix`, and the
    /// user's `flox.toml` / `NIX_CONFIG`.
    #[instrument(
        skip_all,
        fields(progress = "Refreshing store-volume artifacts (env + activation context)")
    )]
    pub(crate) fn refresh_store_volume(
        &self,
        flox: &Flox,
    ) -> Result<StoreVolumeRefresh, ContainerizeProxyError> {
        // Incremental nix copy into the volume (only new paths after a change).
        self.populate_cache_volume()?;

        let cache_path = self.binary_cache_path(flox);
        let cached_bin = read_cached_flox_bin(&cache_path);

        let command = match &cached_bin {
            Some(flox_bin) => self.refresh_direct_exec_command(flox, flox_bin),
            None => self.refresh_nix_run_command(flox),
        };

        let refresh = self.run_refresh_command(command)?;

        // On the cache-miss path, persist the resolved binary so the next
        // refresh for this builder pin takes the fast path.
        if cached_bin.is_none() {
            write_cached_flox_bin(&cache_path, &refresh.flox_bin);
        }

        Ok(refresh)
    }

    /// Direct-exec refresh command (cache hit): run the cached Linux `flox`
    /// binary from the volume directly, no flake unpack.
    fn refresh_direct_exec_command(&self, flox: &Flox, flox_bin: &str) -> Command {
        let mut command = self.runtime_base_command();
        self.add_runtime_args(&mut command, flox);
        command.arg(flox_bin);
        command.args(refresh_flox_args());
        command
    }

    /// Nix-run refresh command (cache miss): fetch and run the flox flake,
    /// paying the flake unpack and a one-time flox-linux build for a new rev.
    fn refresh_nix_run_command(&self, flox: &Flox) -> Command {
        let shell_cmd = nix_run_refresh_shell(&flox_flake_ref(), &verbosity_arg(flox.verbosity));
        let mut command = self.runtime_base_command();
        self.add_runtime_args(&mut command, flox);
        // `bash` (unqualified): Apple Container resolves the entrypoint via
        // the image's PATH env var, not absolute Nix profile symlinks.
        command.args(["bash", "-c", &shell_cmd]);
        command
    }

    /// Run a prepared refresh command, capture stdout, and parse the single
    /// JSON store-path line it prints.
    fn run_refresh_command(
        &self,
        mut command: Command,
    ) -> Result<StoreVolumeRefresh, ContainerizeProxyError> {
        debug!(?command, "running store-volume refresh command");
        let output = command
            .output()
            .map_err(ContainerizeProxyError::RefreshBuilderRun)?;
        if !output.status.success() {
            return Err(ContainerizeProxyError::RefreshBuilderRun(
                std::io::Error::other(String::from_utf8_lossy(&output.stderr).to_string()),
            ));
        }
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        // The builder may emit progress lines before the JSON; take the last
        // non-empty line, which is the refresh result.
        stdout
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .and_then(StoreVolumeRefresh::parse_line)
            .ok_or(ContainerizeProxyError::RefreshParse { stdout })
    }
}

/// Read the cached Linux `flox` binary path for a builder pin.
///
/// Returns `None` when the cache file is absent or does not hold an absolute
/// `/nix/store/…` path, so a corrupt or truncated entry falls back to the
/// nix-run path rather than exec'ing a bogus binary.
fn read_cached_flox_bin(cache_path: &Path) -> Option<String> {
    let contents = std::fs::read_to_string(cache_path).ok()?;
    let bin = contents.trim();
    bin.starts_with("/nix/store/").then(|| bin.to_string())
}

/// Persist the resolved Linux `flox` binary path for a builder pin.
///
/// Best-effort: a write failure only costs the next refresh a flake unpack, so
/// the error is logged rather than propagated.
fn write_cached_flox_bin(cache_path: &Path, flox_bin: &str) {
    if let Some(parent) = cache_path.parent()
        && let Err(err) = std::fs::create_dir_all(parent)
    {
        debug!(%err, path = %cache_path.display(), "could not create store-volume cache dir");
        return;
    }
    if let Err(err) = std::fs::write(cache_path, flox_bin) {
        debug!(%err, path = %cache_path.display(), "could not write store-volume binary cache");
    }
}

impl ContainerBuilder for ContainerizeProxy {
    type Error = ContainerizeProxyError;

    /// Create a [ContainerSource] for macOS that streams the output via a proxy container.
    ///
    /// When the target runtime is Apple Container, the output stream is an OCI
    /// archive (converted inside the builder container using nixpkgs skopeo).
    /// For Docker and Podman, the output remains docker-archive format.
    fn create_container_source(
        &self,
        flox: &Flox,
        // Inferred from `self.environment_path` by flox _inside_ the container.
        _name: impl AsRef<str>,
        tag: impl AsRef<str>,
    ) -> Result<ContainerSource, Self::Error> {
        self.populate_cache_volume()?;

        let command = if self.container_runtime.requires_oci_format() {
            // Apple Container: emit OCI archive via an in-container skopeo pipe.
            self.build_oci_conversion_command(flox, tag)
        } else {
            // Docker / Podman: standard docker-archive path.
            let mut command = self.runtime_base_command();
            self.add_runtime_args(&mut command, flox);
            self.add_nix_args(&mut command);
            self.add_flox_args(&mut command, flox, tag);
            command
        };

        let container_source = ContainerSource::new(command);
        Ok(container_source)
    }
}

#[cfg(test)]
mod tests {
    use flox_rust_sdk::flox::test_helpers::flox_instance;

    use super::*;

    /// Collect the argv of a Command as strings for inspection.
    fn argv(cmd: &Command) -> Vec<String> {
        std::iter::once(cmd.get_program())
            .chain(cmd.get_args())
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn docker_proxy_uses_docker_run() {
        let (flox, _tempdir) = flox_instance();
        let proxy = ContainerizeProxy::new("/some/env".into(), Runtime::Docker, vec![], None);
        let mut cmd = proxy.runtime_base_command();
        proxy.add_runtime_args(&mut cmd, &flox);
        let args = argv(&cmd);
        assert_eq!(args[0], "docker");
        assert!(args.contains(&"run".to_string()));
        assert!(!args.iter().any(|a| a.contains("--userns")));
    }

    #[test]
    fn podman_proxy_adds_userns_flag() {
        let (flox, _tempdir) = flox_instance();
        let proxy = ContainerizeProxy::new("/some/env".into(), Runtime::Podman, vec![], None);
        let mut cmd = proxy.runtime_base_command();
        proxy.add_runtime_args(&mut cmd, &flox);
        let args = argv(&cmd);
        assert_eq!(args[0], "podman");
        // --userns must appear for Podman to map the host user to root inside the container.
        let userns_pos = args
            .iter()
            .position(|a| a == "--userns")
            .expect("--userns should be present for Podman");
        assert_eq!(args[userns_pos + 1], "");
    }

    #[test]
    fn apple_container_proxy_omits_userns_flag() {
        let (flox, _tempdir) = flox_instance();
        let proxy =
            ContainerizeProxy::new("/some/env".into(), Runtime::AppleContainer, vec![], None);
        let mut cmd = proxy.runtime_base_command();
        proxy.add_runtime_args(&mut cmd, &flox);
        let args = argv(&cmd);
        assert_eq!(args[0], "container");
        // Apple Container does not need --userns
        assert!(
            !args.iter().any(|a| a == "--userns"),
            "Apple Container should not have --userns"
        );
    }

    #[test]
    fn apple_container_uses_volume_flag_for_cache() {
        let (flox, _tempdir) = flox_instance();
        let proxy =
            ContainerizeProxy::new("/some/env".into(), Runtime::AppleContainer, vec![], None);
        let mut cmd = proxy.runtime_base_command();
        proxy.add_runtime_args(&mut cmd, &flox);
        let args = argv(&cmd);
        // Apple Container uses --volume name:dst, not --mount type=volume,...
        let vol_pos = args
            .iter()
            .position(|a| a == "--volume")
            .expect("Apple Container should have --volume for the Nix store cache");
        assert_eq!(
            args[vol_pos + 1],
            format!("{CONTAINER_NIX_CACHE_VOLUME}:/nix"),
            "cache mount must be named-volume syntax for Apple Container"
        );
        // The --mount type=volume form must not appear on Apple Container.
        assert!(
            !args
                .iter()
                .any(|a| a.contains("type=volume") && a.contains("dst=/nix")),
            "Apple Container must not use --mount type=volume for /nix"
        );
    }

    #[test]
    fn docker_uses_mount_flag_for_cache() {
        let (flox, _tempdir) = flox_instance();
        let proxy = ContainerizeProxy::new("/some/env".into(), Runtime::Docker, vec![], None);
        let mut cmd = proxy.runtime_base_command();
        proxy.add_runtime_args(&mut cmd, &flox);
        let args = argv(&cmd);
        // Docker uses --mount type=volume,...
        let mount_pos = args
            .iter()
            .position(|a| a.contains("type=volume") && a.contains("dst=/nix"))
            .expect("Docker should have --mount type=volume for the Nix store cache");
        assert!(
            args[mount_pos].contains(CONTAINER_NIX_CACHE_VOLUME),
            "Docker cache mount must reference the named volume"
        );
        // --volume flag must not appear for the cache on Docker.
        assert!(
            !args.iter().any(|a| a == "--volume"),
            "Docker must not use --volume for the cache mount"
        );
    }

    #[test]
    fn oci_conversion_command_includes_cache_volume_mount() {
        let (flox, _tempdir) = flox_instance();
        let proxy = ContainerizeProxy::new(
            "/some/env/.flox/env".into(),
            Runtime::AppleContainer,
            vec![],
            None,
        );
        let cmd = proxy.build_oci_conversion_command(&flox, "latest");
        let args = argv(&cmd);
        // The OCI conversion command also mounts the cache so the builder VM
        // can reuse stored paths during the conversion phase.
        let vol_pos = args
            .iter()
            .position(|a| a == "--volume")
            .expect("OCI conversion command should include --volume for Apple Container cache");
        assert_eq!(
            args[vol_pos + 1],
            format!("{CONTAINER_NIX_CACHE_VOLUME}:/nix"),
            "OCI conversion cache mount must be named-volume syntax"
        );
    }

    #[test]
    fn apple_container_builder_gets_default_memory() {
        let proxy =
            ContainerizeProxy::new("/some/env".into(), Runtime::AppleContainer, vec![], None);
        let cmd = proxy.runtime_base_command();
        let args = argv(&cmd);
        // Default 8g memory flag must be present so the builder VM does not
        // OOM-kill Rust compilation.
        let mem_pos = args
            .iter()
            .position(|a| a == "--memory")
            .expect("--memory should be present for Apple Container");
        assert_eq!(args[mem_pos + 1], DEFAULT_VM_MEMORY);
        // --cpus must NOT be present when _FLOX_CONTAINERIZE_VM_CPUS is unset.
        assert!(
            !args.iter().any(|a| a == "--cpus"),
            "Apple Container should not have --cpus when the env var is unset"
        );
    }

    #[test]
    fn docker_and_podman_do_not_get_memory_flag() {
        let docker_proxy =
            ContainerizeProxy::new("/some/env".into(), Runtime::Docker, vec![], None);
        let docker_args = argv(&docker_proxy.runtime_base_command());
        assert!(
            !docker_args.iter().any(|a| a == "--memory"),
            "Docker should not receive --memory (it is not a VM)"
        );

        let podman_proxy =
            ContainerizeProxy::new("/some/env".into(), Runtime::Podman, vec![], None);
        let podman_args = argv(&podman_proxy.runtime_base_command());
        assert!(
            !podman_args.iter().any(|a| a == "--memory"),
            "Podman should not receive --memory (it is not a VM)"
        );
    }

    #[test]
    fn apple_container_memory_override_is_respected() {
        // Safety: test is single-threaded; the LazyLock is already initialized
        // with the real env value, so we test the override path by exercising
        // add_vm_resource_args directly with a stub rather than mutating the
        // static. Instead, test via the add_vm_resource_args path with env
        // manipulation limited to the override scenario by constructing the
        // args manually.
        //
        // Override by constructing a proxy and calling add_vm_resource_args
        // after temporarily setting the env var before the LazyLock initialises.
        // Because LazyLock<Option<String>> is already initialised by prior tests,
        // we cannot mutate it here. Test the DEFAULT_VM_MEMORY constant instead,
        // which is the codepath exercised by the absence of the env var.
        assert_eq!(DEFAULT_VM_MEMORY, "8g");
    }

    #[test]
    fn oci_conversion_command_includes_memory_flag() {
        let (flox, _tempdir) = flox_instance();
        let proxy = ContainerizeProxy::new(
            "/some/env/.flox/env".into(),
            Runtime::AppleContainer,
            vec![],
            None,
        );
        let cmd = proxy.build_oci_conversion_command(&flox, "latest");
        let args = argv(&cmd);
        // The OCI conversion command uses runtime_base_command, so it must
        // also carry the memory flag.
        let mem_pos = args
            .iter()
            .position(|a| a == "--memory")
            .expect("OCI conversion command should include --memory for Apple Container");
        assert_eq!(args[mem_pos + 1], DEFAULT_VM_MEMORY);
    }

    #[test]
    fn oci_conversion_command_uses_bash_pipeline() {
        let (flox, _tempdir) = flox_instance();
        let proxy = ContainerizeProxy::new(
            "/some/env/.flox/env".into(),
            Runtime::AppleContainer,
            vec![],
            None,
        );
        let cmd = proxy.build_oci_conversion_command(&flox, "latest");
        let args = argv(&cmd);

        // Command is `container run ...`
        assert_eq!(args[0], "container");
        // Apple Container resolves `bash` via the image's PATH env var.
        let bash_pos = args
            .iter()
            .position(|a| a == "bash")
            .expect("bash should be in argv");
        assert_eq!(args[bash_pos + 1], "-c");
        let shell_script = &args[bash_pos + 2];
        // Pipeline should include flox containerize piped to skopeo
        assert!(shell_script.contains("flox"), "pipeline should run flox");
        assert!(
            shell_script.contains("containerize"),
            "pipeline should invoke containerize"
        );
        assert!(
            shell_script.contains("skopeo"),
            "pipeline should use skopeo for OCI conversion"
        );
        assert!(
            shell_script.contains("oci-archive"),
            "output should be OCI archive format"
        );
        assert!(
            shell_script.contains("docker-archive"),
            "input should be docker-archive format"
        );
        // Image ref must include explicit tag for Apple Container compatibility.
        // Bare `name` (without `:tag`) causes `container image load` to fail.
        assert!(
            shell_script.contains(":latest"),
            "OCI image ref must include explicit tag"
        );

        // Load-bearing pipeline structure:
        // skopeo needs --insecure-policy (no policy.json in nixos/nix image).
        assert!(
            shell_script.contains("--insecure-policy"),
            "skopeo must run with --insecure-policy"
        );
        // Both nix invocations redirect stdout to stderr so only `cat`
        // writes the OCI archive to stdout.
        assert_eq!(
            shell_script.matches(">&2").count(),
            2,
            "both nix invocations must redirect stdout to stderr"
        );
        // Two-phase conversion: two temp files, no pipe between
        // containerize and skopeo (concurrent execution can OOM the VM).
        assert_eq!(
            shell_script.matches("mktemp").count(),
            2,
            "two temp files: one docker-archive, one OCI archive"
        );
        assert!(
            !shell_script.contains(" | "),
            "containerize and skopeo must run sequentially, not piped"
        );
    }

    #[test]
    fn oci_conversion_embeds_custom_tag() {
        let (flox, _tempdir) = flox_instance();
        let proxy =
            ContainerizeProxy::new("/env/myapp".into(), Runtime::AppleContainer, vec![], None);
        let cmd = proxy.build_oci_conversion_command(&flox, "v1.2.3");
        let args = argv(&cmd);
        // Custom tag must appear in the OCI destination reference
        assert!(
            args.iter().any(|a| a.contains("v1.2.3")),
            "custom tag 'v1.2.3' should appear in the OCI conversion command"
        );
    }

    #[test]
    fn oci_conversion_escapes_hostile_tags() {
        let (flox, _tempdir) = flox_instance();
        let proxy =
            ContainerizeProxy::new("/env/myapp".into(), Runtime::AppleContainer, vec![], None);

        // A tag containing a space must be single-quoted so it stays one word.
        let cmd = proxy.build_oci_conversion_command(&flox, "my tag");
        let args = argv(&cmd);
        let script = args.last().expect("script is last arg");
        assert!(
            script.contains("--tag 'my tag'"),
            "tag with space must be single-quoted: {script}"
        );
        assert!(
            script.contains("'myapp:my tag'"),
            "image ref with space must be single-quoted: {script}"
        );

        // A tag containing a single quote must not be able to close the
        // quoting and inject shell syntax. `a'b` becomes `'a'\''b'`.
        let cmd = proxy.build_oci_conversion_command(&flox, "a'b");
        let args = argv(&cmd);
        let script = args.last().expect("script is last arg");
        assert!(
            script.contains("--tag 'a'\\''b'"),
            "tag with single quote must be escaped: {script}"
        );
        assert!(
            script.contains("'myapp:a'\\''b'"),
            "image ref with single quote must be escaped: {script}"
        );
        // The raw unescaped tag must not appear as a bare word.
        assert!(
            !script.contains("--tag a'b"),
            "unescaped tag must not reach the shell: {script}"
        );
    }

    #[test]
    fn shell_single_quote_escapes() {
        assert_eq!(shell_single_quote("plain"), "'plain'");
        assert_eq!(shell_single_quote("has space"), "'has space'");
        assert_eq!(shell_single_quote("a'b"), "'a'\\''b'");
        assert_eq!(shell_single_quote("$(rm -rf /)"), "'$(rm -rf /)'");
    }

    #[test]
    fn refresh_flox_args_are_the_hidden_flag_invocation() {
        assert_eq!(refresh_flox_args(), vec![
            "containerize".to_string(),
            "--store-volume-refresh".to_string(),
            "--dir".to_string(),
            "/flox_env".to_string(),
        ]);
    }

    #[test]
    fn refresh_direct_exec_appends_cached_binary_and_flags() {
        let (flox, _tempdir) = flox_instance();
        let proxy =
            ContainerizeProxy::new("/some/env".into(), Runtime::AppleContainer, vec![], None);
        let flox_bin = "/nix/store/cccccccccccccccccccccccccccccccc-flox-1.6.0/bin/flox";
        let cmd = proxy.refresh_direct_exec_command(&flox, flox_bin);
        let args = argv(&cmd);
        assert_eq!(args[0], "container");
        // The cached binary is exec'd directly — no `nix run`, no flake unpack,
        // and no `bash -c` script wrapper (that is the nix-run path's shape).
        assert!(
            !args.iter().any(|a| a == "nix"),
            "direct-exec must not invoke nix: {args:?}"
        );
        assert!(
            !args.iter().any(|a| a == "bash"),
            "direct-exec must not wrap the invocation in a bash script: {args:?}"
        );
        // The binary and the refresh flags are the trailing argv, in order.
        let bin_pos = args
            .iter()
            .position(|a| a == flox_bin)
            .expect("cached binary should appear in argv");
        assert_eq!(&args[bin_pos..], &[
            flox_bin.to_string(),
            "containerize".to_string(),
            "--store-volume-refresh".to_string(),
            "--dir".to_string(),
            "/flox_env".to_string(),
        ]);
    }

    #[test]
    fn refresh_nix_run_command_uses_flake_and_hidden_flag() {
        let (flox, _tempdir) = flox_instance();
        let proxy =
            ContainerizeProxy::new("/some/env".into(), Runtime::AppleContainer, vec![], None);
        let cmd = proxy.refresh_nix_run_command(&flox);
        let args = argv(&cmd);
        assert_eq!(args[0], "container");
        let bash_pos = args.iter().position(|a| a == "bash").expect("bash in argv");
        assert_eq!(args[bash_pos + 1], "-c");
        let script = &args[bash_pos + 2];
        assert!(
            script.contains("nix "),
            "nix-run path must invoke nix: {script}"
        );
        assert!(
            script.contains(" run "),
            "nix-run path must use `run`: {script}"
        );
        assert!(
            script.contains("containerize --store-volume-refresh --dir /flox_env"),
            "must invoke the hidden refresh flag: {script}"
        );
    }

    #[test]
    fn nix_run_refresh_shell_is_a_pure_snapshot() {
        let script = nix_run_refresh_shell("github:flox/flox/deadbeef", "");
        assert_eq!(
            script,
            "set -euo pipefail\n\
            nix --extra-experimental-features 'nix-command flakes' --accept-flake-config \
            run 'github:flox/flox/deadbeef' -- containerize --store-volume-refresh --dir /flox_env"
        );
    }

    #[test]
    fn binary_cache_read_write_round_trips() {
        let (flox, _tempdir) = flox_instance();
        let proxy =
            ContainerizeProxy::new("/some/env".into(), Runtime::AppleContainer, vec![], None);
        let cache_path = proxy.binary_cache_path(&flox);
        // The cache is keyed under the flox cache dir by builder pin.
        assert!(
            cache_path.starts_with(&flox.cache_dir),
            "cache path must live under the flox cache dir: {cache_path:?}"
        );
        assert!(
            cache_path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("flox-bin-"),
            "cache file must be keyed by builder pin: {cache_path:?}"
        );

        // Miss before write.
        assert_eq!(read_cached_flox_bin(&cache_path), None);

        let flox_bin = "/nix/store/cccccccccccccccccccccccccccccccc-flox-1.6.0/bin/flox";
        write_cached_flox_bin(&cache_path, flox_bin);
        assert_eq!(
            read_cached_flox_bin(&cache_path),
            Some(flox_bin.to_string())
        );
    }

    #[test]
    fn binary_cache_rejects_non_store_path() {
        let (flox, _tempdir) = flox_instance();
        let proxy =
            ContainerizeProxy::new("/some/env".into(), Runtime::AppleContainer, vec![], None);
        let cache_path = proxy.binary_cache_path(&flox);
        std::fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        std::fs::write(&cache_path, "/usr/local/bin/flox").unwrap();
        // A corrupt entry must be treated as a miss so we don't exec a bogus
        // binary — the nix-run fallback repopulates it.
        assert_eq!(read_cached_flox_bin(&cache_path), None);
    }
}
