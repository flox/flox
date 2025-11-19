use std::collections::{BTreeMap, HashMap};
use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::LazyLock;

use flox_core::activate::vars::FLOX_RUNTIME_DIR_VAR;
use indoc::formatdoc;
use itertools::Itertools;
use serde::Deserialize;
use tempfile::NamedTempFile;
use thiserror::Error;
use tracing::{debug, error};
use url::Url;

use super::buildenv::{BuildEnvOutputs, BuiltStorePath};
use super::catalog::BaseCatalogUrl;
use super::nix::nix_base_command;
use crate::flox::Flox;
use crate::models::environment::{Environment, EnvironmentError};
use crate::models::lockfile::Lockfile;
use crate::models::manifest::typed::{DEFAULT_GROUP_NAME, Inner, Manifest};
use crate::utils::{CommandExt, ReaderExt};

static FLOX_BUILD_MK: LazyLock<PathBuf> = LazyLock::new(|| {
    std::env::var("FLOX_BUILD_MK")
        .unwrap_or_else(|_| env!("FLOX_BUILD_MK").to_string())
        .into()
});

static FLOX_EXPRESSION_BUILD_NIX: LazyLock<PathBuf> = LazyLock::new(|| {
    std::env::var("FLOX_EXPRESSION_BUILD_NIX")
        .unwrap_or_else(|_| env!("FLOX_EXPRESSION_BUILD_NIX").to_string())
        .into()
});

static GNUMAKE_BIN: LazyLock<PathBuf> = LazyLock::new(|| {
    std::env::var("GNUMAKE_BIN")
        .unwrap_or_else(|_| env!("GNUMAKE_BIN").to_string())
        .into()
});

pub static COMMON_NIXPKGS_URL: LazyLock<Url> = LazyLock::new(|| {
    std::env::var("COMMON_NIXPKGS_URL")
        .as_deref()
        .unwrap_or(env!("COMMON_NIXPKGS_URL"))
        .parse()
        .unwrap()
});

pub trait ManifestBuilder {
    /// Build the specified packages defined in the environment at `flox_env`.
    /// The build process will start in the background.
    /// To process the output, the caller should iterate over the returned [BuildOutput].
    /// Once the process is complete, the [BuildOutput] will yield an [Output::Exit] message.
    fn build(
        self,
        expression_build_nixpkgs: &Url,
        flox_interpreter: &Path,
        package: &[PackageTargetName],
        build_cache: Option<bool>,
        system_override: Option<String>,
    ) -> Result<BuildResults, ManifestBuilderError>;

    fn clean(self, package: &[PackageTargetName]) -> Result<(), ManifestBuilderError>;
}

#[derive(Debug, Error)]
pub enum ManifestBuilderError {
    #[error("failed to call package builder: {0}")]
    CallBuilderError(#[source] std::io::Error),

    #[error("failed to create file for build results")]
    CreateBuildResultFile(#[source] std::io::Error),

    #[error("failed to read file for build results")]
    ReadBuildResultFile(#[source] std::io::Error),

    #[error("failed to parse file for build results")]
    ParseBuildResultFile(#[source] serde_json::Error),

    #[error("failed to call nix to eval NEF")]
    CallNef(#[source] std::io::Error),

    #[error("failed to list available nix expressions to build: {0}")]
    ListNixExpressions(String),

    #[error("failed to parse the license metadata from the build results: {0}")]
    ParseLicenseMetaData(String),

    #[error("failed to clean up build artifacts: {status}")]
    RunClean { status: ExitStatus },

    #[error("Build failed")]
    BuildFailure,
}

#[derive(Debug, PartialEq, Deserialize, Default, derive_more::Deref)]
pub struct BuildResults(Vec<BuildResult>);

#[derive(Debug, PartialEq, Deserialize)]
pub struct BuildResult {
    #[serde(rename = "drvPath")]
    pub drv_path: String,
    pub name: String,
    pub pname: String,
    pub outputs: HashMap<String, BuiltStorePath>,
    pub meta: BuildResultMeta,
    pub version: String,
    pub system: String,
    pub log: BuiltStorePath,
    // TODO: factor out and use buildenv::BuiltStorePath (?)
    #[serde(rename = "resultLinks")]
    pub result_links: BTreeMap<PathBuf, PathBuf>,
}

/// Represents different license formats that can be found in package metadata
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum NixyLicense {
    /// A single license as a string
    String(String),
    /// Multiple licenses as a list of maps or strings
    ComplexList(Vec<NixyLicense>),
    /// License information as a map (e.g., license names to booleans or strings)
    Map {
        #[serde(rename = "spdxId")]
        spdx_id: Option<String>,
        #[serde(rename = "fullName")]
        full_name: Option<String>,
        #[serde(rename = "shortName")]
        short_name: Option<String>,
        url: Option<String>,
    },
}

// Implement a to_string method that returns the string expected by the catalog.
// These rules are a port from the existing scraping logic for consistency.
// If the license metadata is a string, it returns it directly.
// If it's a list, it returns list.
// If it's a map, it takes the keys in the order of "spdxId", "fullName", "shortName", and "url",
impl NixyLicense {
    fn license_map_to_string(
        spdx_id: &Option<String>,
        full_name: &Option<String>,
        short_name: &Option<String>,
        url: &Option<String>,
    ) -> Result<String, ManifestBuilderError> {
        spdx_id.clone()
            .or_else(|| full_name.clone())
            .or_else(|| short_name.clone())
            .or_else(|| url.clone())
            .ok_or_else(|| ManifestBuilderError::ParseLicenseMetaData(
                "License map must contain at least one of the following keys: spdxId, fullName, shortName, url".to_string()
            ))
    }

    /// Convert the license to a string representation expected by the catalog.
    /// These rules are a port from the existing scraping logic for consistency.
    /// - String: Returns the string as-is
    /// - ComplexList: Recursively converts each element and returns a comma-separated string
    /// - Map: Returns the first available key in order of preference: "spdxId", "fullName", "shortName", "url"
    pub fn to_catalog_license(&self) -> Result<String, ManifestBuilderError> {
        match self {
            NixyLicense::String(s) => Ok(s.clone()),
            NixyLicense::ComplexList(list) => {
                let license_strings: Result<Vec<String>, ManifestBuilderError> = list
                    .iter()
                    .map(|license| license.to_catalog_license())
                    .collect();
                Ok(format!("[ {} ]", license_strings?.join(", ")))
            },
            NixyLicense::Map {
                spdx_id,
                full_name,
                short_name,
                url,
            } => NixyLicense::license_map_to_string(spdx_id, full_name, short_name, url),
        }
    }
}

#[derive(Debug, PartialEq, Deserialize)]
pub struct BuildResultMeta {
    pub description: Option<String>,
    pub homepage: Option<String>,
    pub license: Option<NixyLicense>,
    pub broken: Option<bool>,
    pub insecure: Option<bool>,
    pub unfree: Option<bool>,

    #[serde(rename = "outputsToInstall")]
    pub outputs_to_install: Vec<String>,
}

/// A manifest builder that uses the [FLOX_BUILD_MK] makefile to build packages.
pub struct FloxBuildMk<'args> {
    verbosity: i32,
    // should these be borrows?
    temp_dir: &'args Path,
    runtime_dir: &'args Path,

    // common build components
    base_dir: &'args Path,
    expression_dir: &'args Path,
    built_environments: &'args BuildEnvOutputs,

    // Optional buffers that collect output.
    // Without these set std{out,err} of the underlying make call
    // are inherited from the current process.
    stdout_buffer: Option<&'args mut String>,
    stderr_buffer: Option<&'args mut String>,
}

impl FloxBuildMk<'_> {
    pub fn new<'args>(
        flox: &'args Flox,
        base_dir: &'args Path,
        expression_dir: &'args Path,
        built_environments: &'args BuildEnvOutputs,
    ) -> FloxBuildMk<'args> {
        FloxBuildMk {
            verbosity: flox.verbosity,
            temp_dir: &flox.temp_dir,
            runtime_dir: &flox.runtime_dir,
            base_dir,
            expression_dir,
            built_environments,
            stdout_buffer: None,
            stderr_buffer: None,
        }
    }

    /// Create a new instance with std{out,err} piped into buffers
    /// instead of inherited from the current process.
    /// Useful for testing or when one wants to delibrately call the subsystem
    /// without its output forwarded.
    pub fn new_with_buffers<'args>(
        flox: &'args Flox,
        base_dir: &'args Path,
        expression_dir: &'args Path,
        built_environments: &'args BuildEnvOutputs,
        stdout: &'args mut String,
        stderr: &'args mut String,
    ) -> FloxBuildMk<'args> {
        FloxBuildMk {
            verbosity: flox.verbosity,
            temp_dir: &flox.temp_dir,
            runtime_dir: &flox.runtime_dir,
            base_dir,
            expression_dir,
            built_environments,
            stdout_buffer: Some(stdout),
            stderr_buffer: Some(stderr),
        }
    }

    fn base_command(&self, base_dir: &Path) -> Command {
        // todo: extra makeflags, eventually
        let mut command = Command::new(&*GNUMAKE_BIN);
        command.env_remove("MAKEFLAGS");
        command.arg("--file").arg(&*FLOX_BUILD_MK);
        command.arg("--directory").arg(base_dir); // Change dir before reading makefile.
        if self.verbosity <= 0 {
            command.arg("--no-print-directory"); // Only print directory with -v.
        }

        command
    }
}

impl ManifestBuilder for FloxBuildMk<'_> {
    /// Build `packages` defined in the environment rendered at
    /// `flox_env` using the [FLOX_BUILD_MK] makefile.
    ///
    /// `packages` SHOULD be a list of package names defined in the
    /// environment or an empty list to build all packages.
    ///
    /// If a package is not found in the environment,
    /// the makefile will fail with an error.
    /// However, currently the caller doesn't distinguish different error cases.
    ///
    /// The makefile is executed with its current working directory set to `base_dir`.
    /// Upon success, the builder will have built the specified packages
    /// and created links to the respective store paths in `base_dir/result-<build name>`.
    ///
    /// A _single_ nixpkgs revision is provided as a flake-url via BUILDTIME_NIXPKGS_URL,
    /// for use with both manifest and expression build.
    ///
    /// **Invariant**: the caller of this function has to ensure,
    /// that manifest builds are always built with a compatible version of nixpkgs!
    ///
    /// **Invariant**: the caller is expected to prevent mixed builds
    /// of manifest and expression build if `expression_build_nixpkgs_url`
    /// is different from the environments toplevel group,
    /// i.e. manifest builds and expression builds would use incompatible nixpkgs.
    fn build(
        self,
        expression_build_nixpkgs_url: &Url,
        flox_interpreter: &Path,
        packages: &[PackageTargetName],
        build_cache: Option<bool>,
        system_override: Option<String>,
    ) -> Result<BuildResults, ManifestBuilderError> {
        let mut command = self.base_command(self.base_dir);
        command.arg("build");
        command.arg(format!("BUILDTIME_NIXPKGS_URL={}", &*COMMON_NIXPKGS_URL));
        command.arg(format!(
            "EXPRESSION_BUILD_NIXPKGS_URL={expression_build_nixpkgs_url}"
        ));

        if system_override.is_some() {
            command.arg(format!("NIX_SYSTEM={}", system_override.unwrap()));
        }

        command.arg(format!(
            "FLOX_ENV={}",
            self.built_environments.develop.display()
        ));
        command.arg(format!(
            "FLOX_ENV_OUTPUTS={}",
            serde_json::json!(self.built_environments)
        ));

        // TODO: modify flox-build.mk to allow missing expression dirs
        let expression_dir = self.expression_dir.to_string_lossy();
        command.arg(format!("NIX_EXPRESSION_DIR={expression_dir}"));
        command.arg(format!("FLOX_INTERPRETER={}", flox_interpreter.display()));

        // Add the list of packages to be built by passing a space-delimited list
        // of pnames in the PACKAGES variable. If no packages are specified then
        // the makefile will build all packages by default.
        command.arg(format!(
            "PACKAGES={}",
            packages.iter().map(|name| name.as_ref()).join(" ")
        ));

        let build_result_path = NamedTempFile::new_in(self.temp_dir)
            .map_err(ManifestBuilderError::CreateBuildResultFile)?
            .into_temp_path();

        // SAFETY: according to the docs, this is fallible on _Windows_
        let build_result_path = build_result_path
            .keep()
            .expect("failed to keep build result fifo");

        command.arg(format!("BUILD_RESULT_FILE={}", build_result_path.display()));

        let build_cache = build_cache.unwrap_or(true);
        if !build_cache {
            command.arg("DISABLE_BUILDCACHE=true");
        }

        // activate needs this var
        // TODO: we should probably figure out a more consistent way to pass
        // this since it's also passed for `flox activate`
        command.env(FLOX_RUNTIME_DIR_VAR, self.runtime_dir);

        if self.stdout_buffer.is_some() {
            command.stdout(Stdio::piped());
        }

        if self.stderr_buffer.is_some() {
            command.stderr(Stdio::piped());
        }

        debug!(command = %command.display(), "running manifest build target");

        let mut child = command
            .spawn()
            .map_err(ManifestBuilderError::CallBuilderError)?;

        // setup `WireTap` for stdout
        let stdout_tap_context = self.stdout_buffer.map(|buffer| {
            let stdout = child
                .stdout
                .take()
                .expect("STDOUT is piped when stdout_buffer is provided");
            let tap = stdout.tap_lines(|_| {});
            (tap, buffer)
        });

        // setup `WireTap` for stderr
        let stderr_tap_context = self.stderr_buffer.map(|buffer| {
            let stderr = child
                .stderr
                .take()
                .expect("STDERR is piped when stdout_buffer is provided");
            let tap = stderr.tap_lines(|_| {});
            (tap, buffer)
        });

        // **After** taps have been started for std{out,err},
        // read until EOF on both outputs, i.e. wait until the process terminates.
        if let Some((tap, buffer)) = stdout_tap_context {
            *buffer = tap.wait()
        }
        if let Some((tap, buffer)) = stderr_tap_context {
            *buffer = tap.wait()
        }

        let status = child
            .wait()
            .map_err(ManifestBuilderError::CallBuilderError)?;

        if !status.success() {
            return Err(ManifestBuilderError::BuildFailure);
        }

        // TODO: should we bubble up errors through the channel?

        let build_results = std::fs::read_to_string(&build_result_path)
            .map_err(ManifestBuilderError::ReadBuildResultFile)?;

        let build_results = serde_json::from_str(&build_results)
            .map_err(ManifestBuilderError::ParseBuildResultFile)?;

        Ok(build_results)
    }

    /// Clean build artifacts for `packages` defined in the environment
    /// rendered at `flox_env` using the [FLOX_BUILD_MK] makefile.
    ///
    /// `packages` SHOULD be a list of package names defined in the
    /// environment or an empty list to clean all packages.
    ///
    /// `packages` are converted to clean targets by prefixing them with "clean/".
    /// If no packages are specified, all packages are cleaned by evaluating the "clean" target.
    ///
    /// Cleaning will remove the  following build artifacts for the specified packages:
    ///
    /// * the `result-<package>` and `result-<package>-buildCache` store links in `base_dir`
    /// * the store paths linked to by the `result-<package>` links
    /// * the temporary build directories for the specified packages
    fn clean(self, packages: &[PackageTargetName]) -> Result<(), ManifestBuilderError> {
        let mut command = self.base_command(self.base_dir);
        // Required to identify NEF builds.
        command.arg(format!("BUILDTIME_NIXPKGS_URL={}", &*COMMON_NIXPKGS_URL));
        // TODO: is this even necessary, or can we detect build outputs instead?
        command.arg(format!(
            "FLOX_ENV={}",
            self.built_environments.develop.display()
        ));

        // TODO: is this even necessary, or can we detect build outputs instead?
        let expression_dir = self.expression_dir.to_string_lossy();
        command.arg(format!("NIX_EXPRESSION_DIR={expression_dir}"));

        // Add clean target arguments by prefixing the package names with "clean/".
        // If no packages are specified, clean all packages.
        if packages.is_empty() {
            let clean_all_target = "clean";
            command.arg(clean_all_target);
        } else {
            let clean_targets = packages.iter().map(|p| format!("clean/{p}"));
            command.args(clean_targets);
        };

        debug!(command=%command.display(), "running manifest clean target");

        if self.stdout_buffer.is_some() {
            command.stdout(Stdio::piped());
        }

        if self.stderr_buffer.is_some() {
            command.stderr(Stdio::piped());
        }

        let mut child = command
            .spawn()
            .map_err(ManifestBuilderError::CallBuilderError)?;

        // setup `WireTap` for stdout
        let stdout_tap_context = self.stdout_buffer.map(|buffer| {
            let stdout = child
                .stdout
                .take()
                .expect("STDOUT is piped when stdout_buffer is provided");
            let tap = stdout.tap_lines(|_| {});
            (tap, buffer)
        });

        // setup `WireTap` for stderr
        let stderr_tap_context = self.stderr_buffer.map(|buffer| {
            let stderr = child
                .stderr
                .take()
                .expect("STDERR is piped when stdout_buffer is provided");
            let tap = stderr.tap_lines(|_| {});
            (tap, buffer)
        });

        // **After** taps have been started for std{out,err},
        // read until EOF on both outputs, i.e. wait until the process terminates.
        if let Some((tap, buffer)) = stdout_tap_context {
            *buffer = tap.wait()
        }
        if let Some((tap, buffer)) = stderr_tap_context {
            *buffer = tap.wait()
        }

        let status = child
            .wait()
            .map_err(ManifestBuilderError::CallBuilderError)?;

        if !status.success() {
            return Err(ManifestBuilderError::RunClean { status });
        }

        Ok(())
    }
}

/// The canonical path for nix expressions when associated with an environment:
/// Evailable expression builds are discovered with in this directory
/// (see [get_nix_expression_targets] for the discovery results).
pub fn nix_expression_dir(environment: &impl Environment) -> PathBuf {
    environment.dot_flox_path().join("pkgs")
}

pub fn build_symlink_path(
    environment: &impl Environment,
    package: &str,
) -> Result<PathBuf, EnvironmentError> {
    Ok(environment.parent_path()?.join(format!("result-{package}")))
}

/// Look up the "toplevel" groups nixpkgs url from the lockfile.
///
/// Returns [None] if no package is locked under the "toplevel" group.
pub fn find_toplevel_group_nixpkgs(lockfile: &Lockfile) -> Option<BaseCatalogUrl> {
    let top_level_locked_desc = lockfile.packages.iter().find(|pkg| {
        let Some(catalog_package_ref) = pkg.as_catalog_package_ref() else {
            return false;
        };
        catalog_package_ref.group == DEFAULT_GROUP_NAME
    })?;

    Some(BaseCatalogUrl::from(
        &*top_level_locked_desc
            .as_catalog_package_ref()
            .unwrap()
            .locked_url,
    ))
}

/// Use our NEF nix subsystem to query expressions provided in a given expression dir.
/// We need this to verify arguments early rather than running into `make` or `nix` errors,
/// that while correct, have a bad signal/noise ratio.
///
/// The result of this function are the availaboe package names/attrpaths,
/// discovered in `expression_dir`:
///
/// ```text
/// /<expression dir>
///   /foo.nix
///   /bar/default.nix
///   /fizz/buzz/default.nix
/// ```
///
/// will expose the packages `foo`, `bar`, `fizz.buzz`.
fn get_nix_expression_targets(
    expression_dir: &Path,
) -> Result<Vec<(String, ExpressionBuildMetadata)>, ManifestBuilderError> {
    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct NefTargetReflect {
        attr_path_str: String,
        #[serde(flatten)]
        metadata: ExpressionBuildMetadata,
    }

    let output = nix_base_command()
        .arg("eval")
        .args(["--argstr", "nixpkgs-url", COMMON_NIXPKGS_URL.as_str()])
        .args(["--argstr", "pkgs-dir", &*expression_dir.to_string_lossy()])
        .args([
            "--file",
            &*FLOX_EXPRESSION_BUILD_NIX.to_string_lossy(),
            "reflect.attrPaths",
        ])
        .arg("--json")
        .output()
        .map_err(ManifestBuilderError::CallNef)?;

    if !output.status.success() {
        Err(ManifestBuilderError::ListNixExpressions(
            String::from_utf8_lossy(&output.stderr).to_string(),
        ))?
    }

    let attr_paths = serde_json::from_slice::<Vec<NefTargetReflect>>(&output.stdout)
        .map_err(|e| ManifestBuilderError::ListNixExpressions(e.to_string()))?
        .into_iter()
        .map(|reflect| (reflect.attr_path_str, reflect.metadata))
        .collect();

    Ok(attr_paths)
}

#[derive(Debug, Error)]
#[error("{message}")]
pub struct PackageTargetError {
    pub(crate) message: String,
}
impl PackageTargetError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpressionBuildMetadata {
    pub rel_file_path: PathBuf,
}

/// The kind of a package target,
/// i.e. whether a pacakge is sourced from the manifest or a nix expression.
///
/// While not relevant to the build itself,
/// publishing pacakges may differ depending on the kind.
/// For example [super::publish::check_package_metadata]
/// needs to infer the base catalog url
/// from the installed packages in the  top-level group
/// for manifest builds, while expression builds
/// are assigned their base nixpkgs url in coordination with the catalog API
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageTargetKind {
    ExpressionBuild(ExpressionBuildMetadata),
    ManifestBuild,
}

impl PackageTargetKind {
    pub fn is_expression_build(&self) -> bool {
        matches!(self, PackageTargetKind::ExpressionBuild(_))
    }

    pub fn is_manifest_build(&self) -> bool {
        matches!(self, PackageTargetKind::ManifestBuild)
    }
}

/// A wrapper type for the name of package targets.
///
/// This was added to maintain type safety from the [PackageTarget]
/// in the API of the Builder trait,
/// while avoiding the builder to require [PackageTargetKinds],
/// which would otherwise be unused.
///
/// Outside of tests [PacakgeTargetName]s
/// should only be produced via [PackageTarget::name],
/// to maintain the guarantee that the package suppiosedly exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, derive_more::Display, derive_more::AsRef)]
pub struct PackageTargetName<'t>(&'t str);

impl PackageTargetName<'_> {
    #[cfg(any(test, feature = "tests"))]
    pub fn new_unchecked(name: &impl AsRef<str>) -> PackageTargetName<'_> {
        let name = name.as_ref();
        PackageTargetName(name)
    }
}

/// A known package target, i.e. a target sourced from the manifest
/// or nix expression, carrying information about its kind / origin.
///
/// Outside of tests this should only be created via [PackageTargets::select],
/// or [PackageTargets::all] which validate (string) names and associate them with their origin.
#[derive(Debug, Clone, PartialEq, Eq, derive_more::Display)]
#[display("{name}")]
pub struct PackageTarget {
    name: String,
    kind: PackageTargetKind,
}

impl PackageTarget {
    pub fn name(&self) -> PackageTargetName<'_> {
        PackageTargetName(&self.name)
    }

    pub fn kind(&self) -> &PackageTargetKind {
        &self.kind
    }

    #[cfg(any(test, feature = "tests"))]
    pub fn new_unchecked(name: impl Into<String>, kind: PackageTargetKind) -> Self {
        let name = name.into();
        PackageTarget { name, kind }
    }
}

pub struct PackageTargets {
    targets: HashMap<String, PackageTargetKind>,
}

impl PackageTargets {
    /// Collects all build targets from the manifest
    /// and nix expression builds from the expression dir.
    /// For nix expressions [get_nix_expression_targets] is used,
    /// which consults the nef library via `nix eval`.
    ///
    /// This function returns an error if a target name is provided
    /// by both the manifest and an expression.
    ///
    /// Target names, e.g. arguments from the CLI
    /// can be validated against the known targets via [Self::select].
    pub fn new(
        manifest: &Manifest,
        expression_dir: &Path,
    ) -> Result<PackageTargets, PackageTargetError> {
        let environment_packages = &manifest.build;

        let nix_expression_packages = get_nix_expression_targets(expression_dir)
            .map_err(|e| PackageTargetError::new(e.to_string()))?;

        let mut targets = HashMap::new();

        targets.extend(
            environment_packages
                .inner()
                .keys()
                .map(|name| (name.to_string(), PackageTargetKind::ManifestBuild)),
        );

        for (expression_build_target, expression_build_metadata) in nix_expression_packages {
            if targets.contains_key(&expression_build_target) {
                return Err(PackageTargetError::new(formatdoc! {"
                    '{expression_build_target}' is defined in the manifest and as a Nix expression.
                    Rename or delete either the package definition in {expression_dir}
                    or the '[build]' section in the manifest.
                    ", expression_dir = expression_dir.display()
                }));
            }

            targets.insert(
                expression_build_target,
                PackageTargetKind::ExpressionBuild(expression_build_metadata),
            );
        }

        Ok(PackageTargets { targets })
    }

    pub fn is_empty(&self) -> bool {
        self.targets.is_empty()
    }

    /// Validates a list of target names (e.g. CLI arguments),
    /// against the knwon targets, returning a [Vec<PackageTarget>] of valid targets.
    /// If invalid target names are detected this function will return an error instead.
    pub fn select(
        &self,
        targets: &[impl AsRef<str>],
    ) -> Result<Vec<PackageTarget>, PackageTargetError> {
        targets
            .iter()
            .map(|target_name| {
                let target_name = target_name.as_ref();
                let (name, kind) = self.targets.get_key_value(target_name).ok_or_else(|| {
                    PackageTargetError::new(format!("Target '{target_name}' not found."))
                })?;
                Ok(PackageTarget {
                    name: name.to_string(),
                    kind: kind.clone(),
                })
            })
            .collect()
    }

    /// Returns all known targets as a [Vec<PackageTarget>]
    pub fn all(&self) -> Vec<PackageTarget> {
        self.targets
            .iter()
            .map(|(name, kind)| PackageTarget {
                name: name.to_string(),
                kind: kind.clone(),
            })
            .collect()
    }
}

#[cfg(any(test, feature = "tests"))]
pub mod test_helpers {
    use std::fs::{self};

    use tempfile::{TempDir, tempdir_in};

    use super::*;
    use crate::flox::Flox;
    use crate::models::environment::path_environment::PathEnvironment;
    use crate::models::environment::{
        ConcreteEnvironment,
        Environment,
        copy_dir_recursive,
        open_path,
    };

    pub fn result_dir(parent: &Path, package: &str) -> PathBuf {
        parent.join(format!("result-{package}"))
    }

    pub fn cache_dir(parent: &Path, package: &str) -> PathBuf {
        parent.join(format!("result-{package}-buildCache"))
    }

    #[derive(Default, Debug, PartialEq)]
    pub struct CollectedOutput {
        pub build_results: Option<BuildResults>,
        pub stdout: String,
        pub stderr: String,
    }

    pub fn assert_build_status_with_nix_expr(
        flox: &Flox,
        env: &mut PathEnvironment,
        expression_dir: &Path,
        package: &str,
        build_cache: Option<bool>,
        expect_success: bool,
    ) -> CollectedOutput {
        let toplevel_or_common_nixpkgs =
            find_toplevel_group_nixpkgs(&env.lockfile(flox).unwrap().into())
                .map(|toplevel_nixpkgs| toplevel_nixpkgs.as_flake_ref().unwrap())
                .unwrap_or_else(|| COMMON_NIXPKGS_URL.clone());

        let mut output_stdout = String::new();
        let mut output_stderr = String::new();

        let output_build_results = FloxBuildMk::new_with_buffers(
            flox,
            &env.parent_path().unwrap(),
            expression_dir,
            &env.build(flox).unwrap(),
            &mut output_stdout,
            &mut output_stderr,
        )
        .build(
            &toplevel_or_common_nixpkgs,
            &env.rendered_env_links(flox).unwrap().development,
            &[PackageTargetName::new_unchecked(&package)],
            build_cache,
            None,
        );

        let output_build_results = match output_build_results {
            Ok(_) if !expect_success => {
                panic!("expected build to fail");
            },
            Ok(result) => Some(result),
            Err(err) if expect_success => {
                panic!("{}", formatdoc! {"
                    expected build to succeed: {err}
                    stderr: {output_stderr}
                "})
            },
            Err(_) => None,
        };

        CollectedOutput {
            build_results: output_build_results,
            stdout: output_stdout,
            stderr: output_stderr,
        }
    }

    /// Runs a build and asserts that the `ExitStatus` matches `expect_status`.
    /// STDOUT and STDERR are returned if you wish to make additional
    /// assertions on the output of the build.
    pub fn assert_build_status(
        flox: &Flox,
        env: &mut PathEnvironment,
        package_name: &str,
        build_cache: Option<bool>,
        expect_success: bool,
    ) -> CollectedOutput {
        assert_build_status_with_nix_expr(
            flox,
            env,
            &nix_expression_dir(env),
            package_name,
            build_cache,
            expect_success,
        )
    }

    pub fn assert_clean_success(flox: &Flox, env: &mut PathEnvironment, package_names: &[&str]) {
        let err = FloxBuildMk::new_with_buffers(
            flox,
            &env.parent_path().unwrap(),
            &nix_expression_dir(env),
            &env.build(flox).unwrap(),
            &mut String::new(),
            &mut String::new(),
        )
        .clean(
            &package_names
                .iter()
                .map(|name| PackageTargetName::new_unchecked(name))
                .collect::<Vec<_>>(),
        )
        .err();

        assert!(err.is_none(), "expected clean to succeed: {err:?}")
    }

    /// Asserts that `file_name` exists with `content` within the build result
    /// for `package_name`.
    /// Further, asserts that the result is a symlink into the nix store.
    pub fn assert_build_file(parent: &Path, package_name: &str, file_name: &str, content: &str) {
        let dir = result_dir(parent, package_name);
        assert!(dir.is_symlink());
        assert!(dir.read_link().unwrap().starts_with("/nix/store/"));

        let file = dir.join(file_name);
        assert!(file.is_file());
        assert_eq!(fs::read_to_string(file).unwrap(), content);
    }

    /// Reads the content of a file in the build result for `package_name`.
    pub fn result_content(parent: &Path, package: &str, file_name: &str) -> String {
        let dir = result_dir(parent, package);
        let file = dir.join(file_name);
        fs::read_to_string(file).unwrap()
    }

    /// For a list tuples `(AttrPath, NixExpr)`,
    /// create a file structure compatible with nef loading,
    /// within a provided tempdir.
    /// Places the file structure within _a new directory_ within the provided path.
    pub fn prepare_nix_expressions_in(
        tempdir: impl AsRef<Path>,
        expressions: &[(&[&str], &str)],
    ) -> PathBuf {
        let all_expressions_base_dir = tempdir_in(&tempdir).unwrap().keep();

        for (attr_path, expr) in expressions {
            let expression_dir =
                all_expressions_base_dir.join(attr_path.iter().collect::<PathBuf>());
            fs::create_dir_all(&expression_dir).unwrap();
            fs::write(expression_dir.join("default.nix"), expr).unwrap();
        }

        all_expressions_base_dir.canonicalize().unwrap()
    }

    /// Assert that a build succeeds given the path to the environment
    pub fn assert_manifest_build_succeeds(
        path: impl AsRef<Path>,
        name: &str,
        flox: &Flox,
        tmpdir: TempDir,
    ) {
        let path = path.as_ref();
        copy_dir_recursive(path, &tmpdir, true).unwrap();
        let ConcreteEnvironment::Path(mut env) = open_path(flox, &tmpdir, None).unwrap() else {
            panic!("expected path environment")
        };
        assert_build_status(flox, &mut env, name, None, true);
    }
}

#[cfg(test)]
mod license_tests {
    use super::*;

    #[test]
    fn test_build_result_meta_with_license() {
        let json = r#"
        {
            "description": "A test package",
            "homepage": "https://example.com",
            "license": "MIT",
            "broken": false,
            "insecure": null,
            "outputsToInstall": ["out"]
        }
        "#;

        let meta: BuildResultMeta = serde_json::from_str(json).unwrap();
        assert_eq!(meta.description, Some("A test package".to_string()));
        assert_eq!(meta.homepage, Some("https://example.com".to_string()));
        assert_eq!(meta.license, Some(NixyLicense::String("MIT".to_string())));
        assert_eq!(meta.broken, Some(false));
        assert_eq!(meta.insecure, None);
        assert_eq!(meta.outputs_to_install, vec!["out".to_string()]);
    }

    #[test]
    fn test_to_catalog_license_string() {
        let license = NixyLicense::String("MIT".to_string());
        assert_eq!(license.to_catalog_license().unwrap(), "MIT");
    }

    #[test]
    fn test_to_catalog_license_complex_list_strings() {
        let license = NixyLicense::ComplexList(vec![
            NixyLicense::String("MIT".to_string()),
            NixyLicense::String("Apache-2.0".to_string()),
        ]);
        assert_eq!(license.to_catalog_license().unwrap(), "[ MIT, Apache-2.0 ]");
    }

    #[test]
    fn test_to_catalog_license_map_with_spdx_id() {
        let license = NixyLicense::Map {
            spdx_id: Some("MIT".to_string()),
            full_name: Some("MIT License".to_string()),
            short_name: None,
            url: None,
        };
        assert_eq!(license.to_catalog_license().unwrap(), "MIT");
    }

    #[test]
    fn test_to_catalog_license_map_with_full_name() {
        let license = NixyLicense::Map {
            spdx_id: None,
            full_name: Some("MIT License".to_string()),
            short_name: Some("MIT".to_string()),
            url: None,
        };
        assert_eq!(license.to_catalog_license().unwrap(), "MIT License");
    }

    #[test]
    #[should_panic(
        expected = "License map must contain at least one of the following keys: spdxId, fullName, shortName, url"
    )]
    fn test_to_catalog_license_map_no_preferred_keys() {
        let license = NixyLicense::Map {
            spdx_id: None,
            full_name: None,
            short_name: None,
            url: None,
        };
        license.to_catalog_license().unwrap();
    }

    #[test]
    fn test_to_catalog_license_complex_list_maps() {
        let license = NixyLicense::ComplexList(vec![
            NixyLicense::Map {
                spdx_id: Some("MIT".to_string()),
                full_name: None,
                short_name: None,
                url: None,
            },
            NixyLicense::Map {
                spdx_id: Some("Apache-2.0".to_string()),
                full_name: None,
                short_name: None,
                url: None,
            },
        ]);
        assert_eq!(license.to_catalog_license().unwrap(), "[ MIT, Apache-2.0 ]");
    }

    #[test]
    fn test_to_catalog_license_complex_list_mixed() {
        let license = NixyLicense::ComplexList(vec![
            NixyLicense::String("BSD-3-Clause".to_string()),
            NixyLicense::Map {
                spdx_id: Some("MIT".to_string()),
                full_name: Some("MIT License".to_string()),
                short_name: None,
                url: None,
            },
        ]);
        assert_eq!(
            license.to_catalog_license().unwrap(),
            "[ BSD-3-Clause, MIT ]"
        );
    }

    #[test]
    fn test_deserialize_string_array_as_complex_list() {
        // Test that a JSON array of strings is correctly deserialized as ComplexList
        let json = r#"["MIT", "Apache-2.0"]"#;
        let license: NixyLicense = serde_json::from_str(json).unwrap();

        // Should be deserialized as ComplexList containing String variants
        match &license {
            NixyLicense::ComplexList(list) => {
                assert_eq!(list.len(), 2);
                assert_eq!(list[0], NixyLicense::String("MIT".to_string()));
                assert_eq!(list[1], NixyLicense::String("Apache-2.0".to_string()));
            },
            _ => panic!("Expected ComplexList variant"),
        }

        assert_eq!(license.to_catalog_license().unwrap(), "[ MIT, Apache-2.0 ]");
    }

    #[test]
    fn test_deserialize_map_array_as_complex_list() {
        // Test that a JSON array of license maps is correctly deserialized as ComplexList
        let json = r#"[
            {
                "deprecated": false,
                "free": true,
                "fullName": "GNU General Public License v2.0 or later",
                "redistributable": true,
                "shortName": "gpl2Plus",
                "spdxId": "GPL-2.0-or-later",
                "url": "https://spdx.org/licenses/GPL-2.0-or-later.html"
            },
            {
                "deprecated": false,
                "free": false,
                "fullName": "Unfree",
                "redistributable": false,
                "shortName": "NVidia OptiX EULA"
            }
        ]"#;
        let license: NixyLicense = serde_json::from_str(json).unwrap();

        // Should be deserialized as ComplexList containing Map variants
        match &license {
            NixyLicense::ComplexList(list) => {
                assert_eq!(list.len(), 2);
                match &list[0] {
                    NixyLicense::Map {
                        spdx_id, full_name, ..
                    } => {
                        assert_eq!(spdx_id, &Some("GPL-2.0-or-later".to_string()));
                        assert_eq!(
                            full_name,
                            &Some("GNU General Public License v2.0 or later".to_string())
                        );
                    },
                    _ => panic!("Expected first element to be a Map variant"),
                }
                match &list[1] {
                    NixyLicense::Map {
                        spdx_id, full_name, ..
                    } => {
                        assert!(spdx_id.is_none());
                        assert_eq!(full_name, &Some("Unfree".to_string()));
                    },
                    _ => panic!("Expected second element to be a Map variant"),
                }
            },
            _ => panic!("Expected ComplexList variant"),
        }

        assert_eq!(
            license.to_catalog_license().unwrap(),
            "[ GPL-2.0-or-later, Unfree ]"
        );
    }
}

/// Unit tests for the `flox-build.mk` "black box" builder, via
/// the [`FloxBuildMk`] implementation of [`ManifestBuilder`].
///
/// Currently, this is _the_ testsuite for the `flox-build.mk` builder.
#[cfg(test)]
mod tests {
    use std::fs::{self, File};
    use std::os::unix::fs::PermissionsExt;

    use anyhow::Context;
    use indoc::{formatdoc, indoc};

    use super::test_helpers::*;
    use super::*;
    use crate::flox::test_helpers::flox_instance;
    use crate::models::environment::path_environment::test_helpers::{
        new_path_environment,
        new_path_environment_from_env_files,
    };
    use crate::models::environment::{Environment, copy_dir_recursive};
    use crate::providers::catalog::GENERATED_DATA;
    use crate::providers::catalog::test_helpers::catalog_replay_client;
    use crate::providers::git::{GitCommandProvider, GitProvider};

    #[test]
    fn build_returns_failure_when_package_not_defined() {
        let package_name = String::from("foo");

        let (flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, "version = 1");

        assert_build_status(&flox, &mut env, &package_name, None, false);
    }

    #[test]
    fn build_command_generates_file() {
        let package_name = String::from("foo");
        let file_name = String::from("bar");
        let file_content = String::from("some content");

        let manifest = formatdoc! {r#"
            version = 1

            [build.{package_name}]
            command = """
                mkdir $out
                echo -n {file_content} > $out/{file_name}
            """
        "#};

        let (flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, &manifest);
        let env_path = env.parent_path().unwrap();

        assert_build_status(&flox, &mut env, &package_name, None, true);
        assert_build_file(&env_path, &package_name, &file_name, &file_content);
    }

    #[test]
    fn build_no_dollar_out_sandbox_off() {
        let pname = String::from("foo");

        let manifest = formatdoc! {r#"
            version = 1

            [build.{pname}]
            command = "[ ! -e $out ]"
            sandbox = "off"
        "#};

        let (flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, &manifest);

        let output = assert_build_status(&flox, &mut env, &pname, None, false);

        let expected_output = formatdoc! {r#"
            {pname}> ❌  ERROR: Build command did not copy outputs to '$out'.
            {pname}>   - copy a single file with 'mkdir -p $out/bin && cp file $out/bin'
            {pname}>   - copy a bin directory with 'mkdir $out && cp -r bin $out'
            {pname}>   - copy multiple files with 'mkdir -p $out/bin && cp bin/* $out/bin'
            {pname}>   - copy files from an Autotools project with 'make install PREFIX=$out'
        "#};
        assert!(
            output.stderr.contains(&expected_output),
            "{expected_output}"
        );
    }

    #[test]
    fn build_no_dollar_out_sandbox_pure() {
        let pname = String::from("foo");

        let manifest = formatdoc! {r#"
            version = 1

            [build.{pname}]
            command = "[ ! -e $out ]"
            sandbox = "pure"
        "#};

        let (flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, &manifest);
        let env_path = env.parent_path().unwrap();

        let _git = GitCommandProvider::init(&env_path, false).unwrap();

        let output = assert_build_status(&flox, &mut env, &pname, None, false);

        let expected_output = formatdoc! {r#"
            {pname}> ❌  ERROR: Build command did not copy outputs to '$out'.
            {pname}>   - copy a single file with 'mkdir -p $out/bin && cp file $out/bin'
            {pname}>   - copy a bin directory with 'mkdir $out && cp -r bin $out'
            {pname}>   - copy multiple files with 'mkdir -p $out/bin && cp bin/* $out/bin'
            {pname}>   - copy files from an Autotools project with 'make install PREFIX=$out'
        "#};
        assert!(
            output.stderr.contains(&expected_output),
            "{expected_output}"
        );
        assert!(
            !output.stderr.contains("failed to produce output path"),
            "nix's own error for empty output path is bypassed"
        );
    }

    /// Test for:
    /// - non-files in {bin,sbin} (note we do not warn for libexec)
    /// - non-executables in {bin,sbin} (note we do not warn for libexec)
    /// - no executable files found in bin
    /// - executable files in directories other than {bin,sbin,libexec},
    ///   including subdirectories of {bin,sbin,libexec}
    fn build_verify_sane_out(mode: &str) {
        let pname = String::from("foo");

        let manifest = formatdoc! {r#"
            version = 1

            [build.{pname}]
            command = '''
                mkdir -p $out/bin/subdir $out/not-bin
                touch \
                  $out/bin/not-executable \
                  $out/bin/subdir/executable-in-subdir \
                  $out/not-bin/hello
                chmod +x \
                  $out/bin/subdir/executable-in-subdir \
                  $out/not-bin/hello
            '''
            sandbox = "{mode}"
        "#};

        let (flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, &manifest);

        // Create git clone for pure mode only
        if mode == "pure" {
            let _git = GitCommandProvider::init(env.parent_path().unwrap(), false).unwrap();
        }

        // expect the build to succeed
        let output = assert_build_status(&flox, &mut env, &pname, None, true);

        let expected_output = formatdoc! {r#"
            {pname}> ⚠️  WARNING: $out/bin/not-executable is not executable.
            {pname}> ⚠️  WARNING: $out/bin/subdir is not a file.
            {pname}> ⚠️  WARNING: No executables found in '$out/bin'.
            {pname}> Only executables in '$out/bin' will be available on the PATH.
            {pname}> If your build produces executables, make sure they are copied to '$out/bin'.
            {pname}>   - copy a single file with 'mkdir -p $out/bin && cp file $out/bin'
            {pname}>   - copy a bin directory with 'mkdir $out && cp -r bin $out'
            {pname}>   - copy multiple files with 'mkdir -p $out/bin && cp bin/* $out/bin'
            {pname}>   - copy files from an Autotools project with 'make install PREFIX=$out'
            {pname}>{}
            {pname}> HINT: The following executables were found outside of '$out/bin':
            {pname}>   - not-bin/hello
            {pname}>   - bin/subdir/executable-in-subdir
        "#,
        // Nix logs always include one space of padding even on empty lines.
        // Add a trailing space like this so auto-formatters don't trim trailing
        // whitespace
        " "};
        if !output.stderr.contains(&expected_output) {
            pretty_assertions::assert_eq!(
                output.stderr,
                expected_output,
                "didn't find expected output, diffing entire output"
            );
        }
    }

    #[test]
    fn build_verify_sane_out_sandbox_off() {
        build_verify_sane_out("off");
    }

    #[test]
    fn build_verify_sane_out_sandbox_pure() {
        build_verify_sane_out("pure");
    }

    #[test]
    fn build_sandbox_pure() {
        let package_name = String::from("foo");
        let file_name = String::from("bar");
        let file_content = String::from("some content");

        let manifest = formatdoc! {r#"
            version = 1

            [build.{package_name}]
            sandbox = "pure"
            command = """
                mkdir $out
                cp {file_name} $out/{file_name}
            """
        "#};

        let (flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, &manifest);
        let env_path = env.parent_path().unwrap();

        let _git = GitCommandProvider::init(&env_path, false).unwrap();

        // This file is not accessible from a pure build.
        fs::write(env_path.join(&file_name), &file_content).unwrap();
        let output = assert_build_status(&flox, &mut env, &package_name, None, false);
        assert!(output.stderr.contains(&format!(
            "cp: cannot stat '{file_name}': No such file or directory",
        )));

        let dir = result_dir(&env_path, &package_name);
        assert!(!dir.exists());
    }

    #[test]
    fn build_sandbox_off_as_default() {
        let package_name = String::from("foo");
        let file_name = String::from("bar");
        let file_content = String::from("some content");

        let manifest = formatdoc! {r#"
            version = 1

            [build.{package_name}]
            command = """
                mkdir $out
                cp {file_name} $out/{file_name}
            """
        "#};

        let (flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, &manifest);
        let env_path = env.parent_path().unwrap();

        // This file is accessible from an impure build.
        fs::write(env_path.join(&file_name), &file_content).unwrap();
        assert_build_status(&flox, &mut env, &package_name, None, true);
        assert_build_file(&env_path, &package_name, &file_name, &file_content);
    }

    /// Test that buildscripts in the sandbox can write to $HOME
    /// and $HOME is in the sandbox.
    /// In the Nix sandbox $HOME is usually set to `/homeless-shelter`,
    /// does not exist, and cannot be written to.
    /// In turn, any tool attempting to write to $HOME will experience errors to do so.
    /// We set $HOME to another writable location in the sandbox,
    /// to ensure such errors do not occur.
    #[test]
    fn build_sandbox_pure_can_write_home() {
        let package_name = String::from("foo");
        let file_name = String::from("bar");
        let file_content = String::from("some content");

        let manifest = formatdoc! {r#"
            version = 1

            [build.{package_name}]
            sandbox = "pure"
            command = """
                mkdir $out
                echo -n "{file_content}" > "$HOME/{file_name}"
                cp "$HOME/{file_name}" "$out/{file_name}"
            """
        "#};

        let (flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, &manifest);
        let env_path = env.parent_path().unwrap();

        let _git = GitCommandProvider::init(&env_path, false).unwrap();

        assert_build_status(&flox, &mut env, &package_name, None, true);
        assert_build_file(&env_path, &package_name, &file_name, &file_content);

        // Asserts that the build script did not write to the actual $HOME
        let actual_home = std::env::var("HOME").unwrap();
        assert!(!Path::new(&actual_home).join(&file_name).exists());
    }

    #[test]
    fn build_cache_sandbox_pure_uses_cache() {
        let package_name = String::from("foo");
        let file_name = String::from("bar");

        let manifest = formatdoc! {r#"
            version = 1

            [build.{package_name}]
            sandbox = "pure"
            command = """
                mkdir -p $out

                if [ ! -e ./cached-value ]; then
                    # Generate a random value to cache,
                    # successive builds should use this value
                    # RANDOM is a bash builtin that returns a random integer
                    # each time it's evaluated
                    echo "$RANDOM" > ./cached-value
                fi

                cp ./cached-value $out/{file_name}
            """
        "#};

        let (flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, &manifest);
        let env_path = env.parent_path().unwrap();

        let _git = GitCommandProvider::init(&env_path, false).unwrap();

        assert_build_status(&flox, &mut env, &package_name, None, true);
        let file_content = result_content(&env_path, &package_name, &file_name);

        // Asserts that the build result uses the cached value of the previous build
        assert_build_status(&flox, &mut env, &package_name, None, true);
        assert_build_file(&env_path, &package_name, &file_name, &file_content);
    }

    #[test]
    fn build_can_disable_buildcache() {
        let package_name = String::from("foo");
        let file_name = String::from("bar");

        let manifest = formatdoc! {r#"
            version = 1

            [build.{package_name}]
            sandbox = "pure"
            command = """
                mkdir -p $out

                if [ ! -e ./cached-value ]; then
                    # Generate a random value to cache,
                    # successive builds should use this value
                    # RANDOM is a bash builtin that returns a random integer
                    # each time it's evaluated
                    echo "$RANDOM" > ./cached-value
                fi

                cp ./cached-value $out/{file_name}
            """
        "#};

        let (flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, &manifest);
        let env_path = env.parent_path().unwrap();

        let _git = GitCommandProvider::init(&env_path, false).unwrap();

        assert_build_status(&flox, &mut env, &package_name, Some(false), true);

        let cache_dir = cache_dir(&env_path, &package_name);
        assert!(!cache_dir.exists());
    }

    #[test]
    fn build_cache_sandbox_pure_cache_can_be_invalidated() {
        let package_name = String::from("foo");
        let file_name = String::from("bar");

        let manifest = formatdoc! {r#"
            version = 1

            [build.{package_name}]
            sandbox = "pure"
            command = """
                mkdir -p $out

                if [ ! -e ./cached-value ]; then
                    # Generate a random value to cache,
                    # successive builds should use this value
                    # RANDOM is a bash builtin that returns a random integer
                    # each time it's evaluated
                    echo "$RANDOM" > ./cached-value
                fi

                cp ./cached-value $out/{file_name}
            """
        "#};

        let (flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, &manifest);
        let env_path = env.parent_path().unwrap();

        let _git = GitCommandProvider::init(&env_path, false).unwrap();

        assert_build_status(&flox, &mut env, &package_name, None, true);
        let file_content_first_run = result_content(&env_path, &package_name, &file_name);

        let cache_dir = cache_dir(&env_path, &package_name);
        assert!(cache_dir.exists());
        fs::remove_file(cache_dir).unwrap();

        assert_build_status(&flox, &mut env, &package_name, None, true);
        let file_content_second_run = result_content(&env_path, &package_name, &file_name);

        assert_ne!(file_content_first_run, file_content_second_run);
    }

    #[test]
    fn build_cache_sandbox_off_uses_fs_as_cache() {
        let package_name = String::from("foo");
        let file_name = String::from("bar");

        let manifest = formatdoc! {r#"
            version = 1

            [build.{package_name}]
            sandbox = "off"
            command = """
                # Previous $out is left in place!
                mkdir -p $out

                if [ ! -e ./cached-value ]; then
                    # Generate a random value to cache,
                    # successive builds should use this value
                    # RANDOM is a bash builtin that returns a random integer
                    # each time it's evaluated
                    echo "$RANDOM" > ./cached-value
                fi

                cp ./cached-value $out/{file_name}
            """
        "#};

        let (flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, &manifest);
        let env_path = env.parent_path().unwrap();

        assert_build_status(&flox, &mut env, &package_name, None, true);
        let file_content = result_content(&env_path, &package_name, &file_name);

        assert_build_status(&flox, &mut env, &package_name, None, true);
        assert_build_file(&env_path, &package_name, &file_name, &file_content);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn build_uses_package_from_manifest() {
        let package_name = String::from("foo");
        let file_name = String::from("bar");
        let file_content = String::from("developcopy-build-foo/bin/hello\n");

        let manifest = formatdoc! {r#"
            version = 1
            [install]
            hello.pkg-path = "hello"

            [build.{package_name}]
            sandbox = "pure"
            command = """
                mkdir $out
                type hello | grep -o "{file_content}" > $out/{file_name}
            """
        "#};

        let (mut flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, &manifest);
        let env_path = env.parent_path().unwrap();

        let _git = GitCommandProvider::init(&env_path, false).unwrap();

        flox.catalog_client =
            catalog_replay_client(GENERATED_DATA.join("resolve/hello.yaml")).await;
        assert_build_status(&flox, &mut env, &package_name, None, true);
        assert_build_file(&env_path, &package_name, &file_name, &file_content);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn build_result_uses_package_from_environment() {
        let package_name = String::from("foo");
        let file_name = String::from("exec-hello-from-env.sh");

        let manifest = formatdoc! {r#"
            version = 1
            [install]
            hello.pkg-path = "hello"

            [build.{package_name}]
            sandbox = "pure"
            command = """
                mkdir -p $out/bin
                cat > $out/bin/{file_name} <<EOF
                    #!/usr/bin/env bash
                    exec hello
                EOF
                chmod +x $out/bin/{file_name}
            """
        "#};

        let (mut flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, &manifest);
        let env_path = env.parent_path().unwrap();

        let _git = GitCommandProvider::init(&env_path, false).unwrap();

        flox.catalog_client =
            catalog_replay_client(GENERATED_DATA.join("resolve/hello.yaml")).await;
        assert_build_status(&flox, &mut env, &package_name, None, true);

        let result_path = result_dir(&env_path, &package_name)
            .join("bin")
            .join(&file_name);

        fs::write(env_path.join("hello"), indoc! {r#"
            #!/usr/bin/env bash
            echo "This should not be used because the environment's PATH takes precedence"
            exit 1
        "#})
        .unwrap();

        let output = Command::new(&result_path)
            .env("PATH", env_path)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim_end(),
            "Hello, world!",
            "should successfully execute hello from environment"
        );
    }

    fn build_uses_var_from_manifest(sandbox: bool) {
        let package_name = String::from("foo");
        let file_name = String::from("bar");
        let file_content = String::from("some content");

        let manifest = formatdoc! {r#"
            version = 1

            [vars]
            FOO = "{file_content}"

            [build.{package_name}]
            command = """
                mkdir $out
                echo -n "$FOO" > $out/{file_name}
            """
            sandbox = "{}"
        "#, if sandbox { "pure" } else { "off" }};

        let (flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, &manifest);
        let env_path = env.parent_path().unwrap();

        if sandbox {
            let _git = GitCommandProvider::init(&env_path, false).unwrap();
        }

        assert_build_status(&flox, &mut env, &package_name, None, true);
        assert_build_file(&env_path, &package_name, &file_name, &file_content);
    }

    #[test]
    fn build_uses_var_from_manifest_sandbox_off() {
        build_uses_var_from_manifest(false);
    }

    #[test]
    fn build_uses_var_from_manifest_pure() {
        build_uses_var_from_manifest(true);
    }

    #[test]
    fn vars_not_set_at_runtime() {
        let package_name = String::from("foo");
        let file_path = String::from("bin/print_var");
        let inner_var_value = String::from("some content");
        let var = "FOO";

        let manifest = formatdoc! {r#"
            version = 1

            [vars]
            {var} = "{inner_var_value}"

            [build.{package_name}]
            command = """
            mkdir -p $out/bin
            cat > $out/{file_path} <<'EOF'
                #!/usr/bin/env bash
                echo "${var}"
            EOF
            chmod +x $out/{file_path}
            """
        "#};

        let (flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, &manifest);
        let env_path = env.parent_path().unwrap();

        assert_build_status(&flox, &mut env, &package_name, None, true);

        let package_bin = result_dir(&env_path, &package_name).join(file_path);
        let outer_var_value = "outer";
        let output = Command::new(&package_bin)
            .env(var, outer_var_value)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim_end(),
            outer_var_value,
        );
    }

    #[test]
    fn build_does_not_use_hook_from_manifest() {
        let package_name = String::from("foo");
        let file_name = String::from("bar");

        let manifest = formatdoc! {r#"
            version = 1

            [hook]
            on-activate = '''
                # Touch a file as side effect of running hook.
                touch {file_name}
            '''

            [build.{package_name}]
            command = """
                mkdir $out
                if [ -e "{file_name}" ]; then
                    echo "Hook ran, but this should not happen."
                    exit 1
                fi
                touch $out/{file_name}
            """
        "#};

        let (flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, &manifest);

        assert_build_status(&flox, &mut env, &package_name, None, true);
    }

    #[test]
    fn build_can_contain_heredocs() {
        let package_name = String::from("with-heredocs");
        let file_name = String::from("bar");

        let manifest = formatdoc! {r#"
            version = 1

            [hook]
            on-activate = '''
              export FOO="will not be used"
            '''

            [build.{package_name}]
            command = """
                mkdir $out
                cat << EOF > $out/{file_name}
                Triple quotes embrace
                Multiline wisdom flows
                Syntax peace descends
                EOF
            """
        "#};

        let (flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, &manifest);
        let env_path = env.parent_path().unwrap();

        assert_build_status(&flox, &mut env, &package_name, None, true);
        assert_build_file(&env_path, &package_name, &file_name, indoc! {"
            Triple quotes embrace
            Multiline wisdom flows
            Syntax peace descends
        "});
    }

    fn build_depending_on_another_build(dep_sandbox: &str, package_sandbox: &str) {
        let package_name = String::from("app-with-dashes");
        let file_name = String::from("foo");
        let file_content = String::from("some content");

        let manifest = formatdoc! {r#"
            version = 1

            [build.dep-with-dashes]
            sandbox = "{dep_sandbox}"
            command = """
                mkdir $out
                echo -n "{file_content}" > $out/{file_name}
            """

            [build.{package_name}]
            sandbox = "{package_sandbox}"
            command = """
                mkdir $out
                cp ${{dep-with-dashes}}/{file_name} $out/{file_name}
            """
        "#};

        let (flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, &manifest);
        let env_path = env.parent_path().unwrap();
        let _ = GitCommandProvider::init(&env_path, false).unwrap();

        assert_build_status(&flox, &mut env, &package_name, None, true);
        assert_build_file(&env_path, &package_name, &file_name, &file_content);
    }

    #[test]
    fn build_depending_on_another_build_both_off() {
        build_depending_on_another_build("off", "off");
    }

    #[test]
    fn build_depending_on_another_build_both_pure() {
        build_depending_on_another_build("pure", "pure");
    }

    #[test]
    fn build_depending_on_another_build_off_and_pure() {
        build_depending_on_another_build("off", "pure");
    }

    #[test]
    fn build_depending_on_another_build_pure_and_off() {
        build_depending_on_another_build("pure", "off");
    }

    #[test]
    fn rebuild_with_modified_command() {
        let package_name = String::from("foo");
        let file_name = String::from("bar");
        let content_before = "before";
        let content_after = "after";

        let (flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, &formatdoc! {r#"
            version = 1

            [build.{package_name}]
            command = """
                mkdir -p $out
                echo -n "{content_before}" > $out/{file_name}
            """
        "#});
        let env_path = env.parent_path().unwrap();
        assert_build_status(&flox, &mut env, &package_name, None, true);
        assert_build_file(&env_path, &package_name, &file_name, content_before);

        let _ = env
            .edit(&flox, formatdoc! {r#"
            version = 1

            [build.{package_name}]
            command = """
                mkdir -p $out
                echo -n "{content_after}" > $out/{file_name}
            """
        "#})
            .unwrap();
        assert_build_status(&flox, &mut env, &package_name, None, true);
        assert_build_file(&env_path, &package_name, &file_name, content_after);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn build_wraps_binaries_with_preserved_arg0() {
        let package_name = String::from("foo");
        let file_name = String::from("print_arg0");

        let manifest = formatdoc! {r#"
            version = 1

            [install]
            go.pkg-path = "go"

            [build.{package_name}]
            command = """
                go build main.go
                mkdir -p $out/bin
                cp main $out/bin/{file_name}
            """
        "#};

        let (mut flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, &manifest);
        let env_path = env.parent_path().unwrap();

        let arg0_code = indoc! {r#"
            package main

            import (
                "fmt"
                "os"
            )

            func main() {
                fmt.Println(os.Args[0])
            }
        "#};
        fs::write(env_path.join("main.go"), arg0_code).unwrap();

        flox.catalog_client = catalog_replay_client(GENERATED_DATA.join("resolve/go.yaml")).await;
        assert_build_status(&flox, &mut env, &package_name, None, true);
        let result_path = result_dir(&env_path, &package_name)
            .join("bin")
            .join(&file_name);

        let output = Command::new(&result_path).output().unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim_end(),
            result_path.to_string_lossy(),
            "binaries should have the correct arg0"
        );
    }

    #[test]
    fn build_wraps_scripts_without_preserved_arg0() {
        let package_name = String::from("foo");
        let file_name = String::from("print_arg0");

        let manifest = formatdoc! {r#"
            version = 1

            [build.{package_name}]
            command = """
                mkdir -p $out/bin
                cp {file_name} $out/bin/{file_name}
                chmod +x $out/bin/{file_name}
            """
        "#};

        let (flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, &manifest);
        let env_path = env.parent_path().unwrap();

        // Beware inlining this script and having $0 interpolated too early.
        let arg0_code = indoc! {r#"
            #!/usr/bin/env bash
            echo "$0"
        "#};
        fs::write(env_path.join(&file_name), arg0_code).unwrap();

        assert_build_status(&flox, &mut env, &package_name, None, true);
        let result_path = result_dir(&env_path, &package_name)
            .join("bin")
            .join(&file_name);
        let result_wrapped = result_dir(&env_path, &package_name)
            .read_link() // store path
            .unwrap()
            .join("bin")
            .join(format!(".{}-wrapped", &file_name));

        let output = Command::new(&result_path).output().unwrap();
        assert!(output.status.success());

        // This isn't possible for interpreted scripts as described in:
        // https://github.com/NixOS/nixpkgs/issues/150841
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim_end(),
            result_wrapped.to_string_lossy(),
            "interpreted scripts are known to have the wrong arg0"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn build_wraps_scripts_without_preserved_exe() {
        let package_name = String::from("foo");
        let file_name = String::from("print_exe");

        let manifest = formatdoc! {r#"
            version = 1

            [install]
            go.pkg-path = "go"

            [build.{package_name}]
            command = """
                go build main.go
                mkdir -p $out/bin
                cp main $out/bin/{file_name}
            """
        "#};

        let (mut flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, &manifest);
        let env_path = env.parent_path().unwrap();

        let exe_code = indoc! {r#"
            package main

            import (
                "fmt"
                "os"
            )

            func main() {
                exe, err := os.Executable()
                if err != nil {
                    fmt.Println(err)
                    os.Exit(1)
                }

                fmt.Println(exe)
            }
        "#};
        fs::write(env_path.join("main.go"), exe_code).unwrap();

        flox.catalog_client = catalog_replay_client(GENERATED_DATA.join("resolve/go.yaml")).await;
        assert_build_status(&flox, &mut env, &package_name, None, true);
        let result_path = result_dir(&env_path, &package_name)
            .join("bin")
            .join(&file_name);
        let result_wrapped = result_dir(&env_path, &package_name)
            .read_link() // store path
            .unwrap()
            .join("bin")
            .join(format!(".{}-wrapped", &file_name));

        let output = Command::new(&result_path).output().unwrap();
        assert!(output.status.success());

        // This isn't currently implemented. For ideas see:
        // https://brioche.dev/docs/how-it-works/packed-executables/
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim_end(),
            result_wrapped.to_string_lossy(),
            "binaries are known to have the wrong exe"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn build_impure_against_libc() {
        let package_name = String::from("foo");
        let bin_name = String::from("links-against-libc");
        let source_name = String::from("main.go");

        let (flox, _temp_dir_handle) = flox_instance();
        let mut env =
            new_path_environment_from_env_files(&flox, GENERATED_DATA.join("envs/go_gcc"));
        let env_path = env.parent_path().unwrap();

        let base_manifest = env.manifest_contents(&flox).unwrap();
        let build_manifest = formatdoc! {r#"
            {base_manifest}

            [vars]
            CGO_ENABLED = "1"

            [build.{package_name}]
            command = """
                cat main.go
                go build {source_name}
                mkdir -p $out/bin
                cp main $out/bin/{bin_name}
            """
        "#};
        env.edit(&flox, build_manifest).unwrap();

        let expected_message = "Hello from C!";
        // Literal `{` and `}` are escaped as `{{` and `}}`.
        let source_code = formatdoc! {r#"
            package main

            /*
            #include <stdio.h>

            void hello() {{
                printf("{expected_message}\n");
                fflush(stdout);
            }}
            */
            import "C"

            func main() {{
                C.hello()
            }}
        "#};
        fs::write(env_path.join(source_name), source_code).unwrap();

        assert_build_status(&flox, &mut env, &package_name, None, true);

        let result_path = result_dir(&env_path, &package_name)
            .join("bin")
            .join(&bin_name);
        let output = Command::new(&result_path).output().unwrap();

        // The binary should execute successfully but we can't make any
        // guarantees about the portability or reproducibility of impure builds
        // which may link against system libraries.
        //
        // This also serves as a regression test against `autoPathelfHook`
        // conflicting with `gcc` or `libc` from the Flox environment which will
        // cause either binaries that hang or fail with:
        //
        // `*** stack smashing detected ***: terminated`
        assert!(
            output.status.success(),
            "should execute successfully, stderr: {:?}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim_end(),
            expected_message
        );
    }

    /// Test that Flox provided boost can be included at build time and linked
    /// against when boost is in runtime-packages.
    /// but if linking isn't needed, runtime-packages can be empty.
    /// Use boost::system::error_code so we have to link against boost_system.
    fn boost_runtime(sandbox: bool) {
        let package_name = String::from("test_boost");
        let source_name = String::from("test_boost.cpp");
        let bin_name = String::from("test_boost");

        let (flox, _temp_dir_handle) = flox_instance();
        let mut env =
            new_path_environment_from_env_files(&flox, GENERATED_DATA.join("envs/gcc_boost"));
        let env_path = env.parent_path().unwrap();

        let base_manifest = env.manifest_contents(&flox).unwrap();
        let build_manifest = formatdoc! {r#"
            {base_manifest}

            [build.{package_name}]
            command = """
                g++ -o {bin_name} {source_name} -lboost_system
                mkdir -p $out/bin
                cp {bin_name} $out/bin/{bin_name}
            """
            # FIXME: replace with gcc.lib once we support outputs
            runtime-packages = [ "boost", "gcc" ]
            sandbox = "{}"
            "#, if sandbox { "pure" } else { "off" }};
        env.edit(&flox, build_manifest).unwrap();

        let source_code = indoc! {r#"
            #include <iostream>
            #include <boost/system/error_code.hpp>

            int main() {
                // Create an error code representing a generic "invalid argument"
                boost::system::error_code ec(boost::system::errc::invalid_argument,
                                             boost::system::generic_category());

                std::cout << ec.value() << std::endl;

                return 0;
            }
            "#};
        fs::write(env_path.join(&source_name), source_code).unwrap();

        if sandbox {
            let git = GitCommandProvider::init(&env_path, false).unwrap();
            git.add(&[&PathBuf::from(source_name)]).unwrap();
        }

        assert_build_status(&flox, &mut env, &package_name, None, true);

        let result_path = result_dir(&env_path, &package_name)
            .join("bin")
            .join(&bin_name);
        let output = Command::new(&result_path).output().unwrap();

        assert!(
            output.status.success(),
            "should execute successfully, stderr: {:?}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim_end(), "22",);
    }

    #[test]
    fn boost_runtime_sandbox_off() {
        boost_runtime(false);
    }

    #[test]
    fn boost_runtime_sandbox_pure() {
        boost_runtime(true);
    }

    /// Test that Flox provided boost can be included at build time,
    /// but if linking isn't needed, runtime-packages can be empty.
    /// Use lexical_cast from boost which does require linking against boost.
    fn boost_include_only(sandbox: bool) {
        let package_name = String::from("test_boost");
        let source_name = String::from("test_boost.cpp");
        let bin_name = String::from("test_boost");

        let (flox, _temp_dir_handle) = flox_instance();
        let mut env =
            new_path_environment_from_env_files(&flox, GENERATED_DATA.join("envs/gcc_boost"));
        let env_path = env.parent_path().unwrap();

        let base_manifest = env.manifest_contents(&flox).unwrap();
        let build_manifest = formatdoc! {r#"
            {base_manifest}

            [build.{package_name}]
            command = """
                g++ -o {bin_name} {source_name}
                mkdir -p $out/bin
                cp {bin_name} $out/bin/{bin_name}
            """
            # FIXME: replace with gcc.lib once we support outputs
            runtime-packages = [ "gcc" ]
            sandbox = "{}"
            "#, if sandbox { "pure" } else { "off" }};
        env.edit(&flox, build_manifest).unwrap();

        let source_code = indoc! {r#"
            #include <iostream>
            #include <string>
            #include <boost/lexical_cast.hpp>

            int main() {
                try {
                    std::string str = "123";
                    int num = boost::lexical_cast<int>(str);
                    std::cout << num << std::endl;
                }
                catch (const boost::bad_lexical_cast& e) {
                    std::cerr << "Lexical cast error: " << e.what() << std::endl;
                }

                return 0;
            }
            "#};
        fs::write(env_path.join(&source_name), source_code).unwrap();

        if sandbox {
            let git = GitCommandProvider::init(&env_path, false).unwrap();
            git.add(&[&PathBuf::from(source_name)]).unwrap();
        }

        assert_build_status(&flox, &mut env, &package_name, None, true);

        let result_path = result_dir(&env_path, &package_name)
            .join("bin")
            .join(&bin_name);
        let output = Command::new(&result_path).output().unwrap();

        assert!(
            output.status.success(),
            "should execute successfully, stderr: {:?}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim_end(), "123",);
    }

    #[test]
    fn boost_include_only_sandbox_off() {
        boost_include_only(false);
    }

    #[test]
    fn boost_include_only_sandbox_pure() {
        boost_include_only(true);
    }

    /// Test that a runtime package installed to an "other" system type does
    /// not trigger a build failure (#3055).
    #[test]
    fn other_system_runtime_packages() {
        let package_name = String::from("foo");
        let file_name = String::from("bar");
        let file_content = String::from("some content");

        let manifest = formatdoc! {r#"
            version = 1

            [install]
            hello.pkg-path = "hello"
            # Intentionally not installing hello for any systems to trigger
            # runtime-packages exception below.
            hello.systems = [ ]

            [build.{package_name}]
            command = """
                mkdir $out
                echo -n {file_content} > $out/{file_name}
            """
            runtime-packages = [ "hello" ]
        "#};
        let (flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, &manifest);
        let env_path = env.parent_path().unwrap();

        assert_build_status(&flox, &mut env, &package_name, None, true);
        assert_build_file(&env_path, &package_name, &file_name, &file_content);
    }

    /// Contrived example to represent a binary that links against something
    /// from the environment but isn't included in the final package closure.
    /// The sub-shells are evaluated at build time.
    fn closure_check_hello_command() -> String {
        indoc! {r#"
            mkdir -p $out/bin
            cat > "$out/bin/test" <<EOF
            #!/usr/bin/env bash
            $(realpath $(which hello))
            EOF
            chmod +x "$out/bin/test"
        "#}
        .to_string()
    }

    async fn assert_closure_check_failure(manifest: &str, package_name: &str, mock_file: &str) {
        let (mut flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, manifest);
        flox.catalog_client = catalog_replay_client(GENERATED_DATA.join(mock_file)).await;
        let output = assert_build_status(&flox, &mut env, package_name, None, false);

        // TODO: Provide more targeted advice based on the current lockfile's
        //       `runtime-packages` and `install` groups so that we don't need
        //       to tell the user to try everything.
        let expected_output = formatdoc! {r#"
            ❌ ERROR: Unexpected dependencies found in package '{package_name}':

            1. Remove any unneeded references (e.g. debug symbols) from your build.
            2. If you’re using package groups, move these packages into the 'toplevel' group.
            3. If you’re using 'runtime-packages', make sure each package is listed both in
               'runtime-packages' and in the 'toplevel' group.

        "#};
        if !output.stderr.contains(&expected_output) {
            pretty_assertions::assert_eq!(
                output.stderr,
                expected_output,
                "didn't find expected output, diffing entire output"
            );
        }

        let store_path_prefix_pattern = r"/nix/store/[\w]{32}";
        let expected_pattern = if cfg!(target_os = "macos") {
            formatdoc! {r#"
                2 packages found in {store_path_prefix_pattern}-{package_name}-0\.0\.0
                       not found in {store_path_prefix_pattern}-environment-build-{package_name}
        "#}
        } else {
            formatdoc! {r#"
                4 packages found in {store_path_prefix_pattern}-{package_name}-0\.0\.0
                       not found in {store_path_prefix_pattern}-environment-build-{package_name}

                Displaying first 3 only:
        "#}
        };
        let re = regex::Regex::new(&expected_pattern).unwrap();
        assert!(
            re.is_match(&output.stderr),
            "output does not match expected pattern\noutput: {}\n\npattern: {}",
            &output.stderr,
            expected_pattern,
        );
    }

    /// Packages referenced from outside `runtime-packages` trigger a build failure.
    #[tokio::test(flavor = "multi_thread")]
    async fn closure_check_runtime_packages() {
        let package_name = String::from("my-package");
        let build_command = closure_check_hello_command();
        let manifest = formatdoc! {r#"
            version = 1

            [install]
            hello.pkg-path = "hello"

            [build.{package_name}]
            runtime-packages = []
            command = """
            {build_command}
            """
        "#};

        assert_closure_check_failure(&manifest, &package_name, "resolve/hello.yaml").await;
    }

    /// Packages referenced from outside the `toplevel` group trigger a build
    /// failure even when `runtime-packages` is not specified.
    #[tokio::test(flavor = "multi_thread")]
    async fn closure_check_non_toplevel_pkg_group() {
        let package_name = String::from("my-package");
        let build_command = closure_check_hello_command();
        let manifest = formatdoc! {r#"
            version = 1

            [install]
            hello.pkg-path = "hello"
            hello.pkg-group = "other"

            [build.{package_name}]
            command = """
            {build_command}
            """
        "#};

        assert_closure_check_failure(
            &manifest,
            &package_name,
            "envs/hello_other_pkg_group/hello_other_pkg_group.yaml",
        )
        .await;
    }

    /// Contrived example of a package referring to files possibly found
    /// in the develop environment that aren't in the build closure.
    async fn assert_nonexistent_path_check_failure(bin_name: &str, hint: &str) {
        let package_name = String::from("my-package");
        let build_command = formatdoc! {r#"
            mkdir -p $out/bin
            cat > "$out/bin/test" <<EOF
            #!/usr/bin/env bash
            $FLOX_ENV/bin/{bin_name}
            EOF
            chmod +x "$out/bin/test"
        "#};
        let manifest = formatdoc! {r#"
            version = 1

            [install]
            hello.pkg-path = "hello"
            curl.pkg-path = "curl"
            curl.pkg-group = "not-toplevel"

            [build.{package_name}]
            runtime-packages = []
            command = """
            {build_command}
            """
        "#};

        let (mut flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, &manifest);

        flox.catalog_client =
            catalog_replay_client(GENERATED_DATA.join("resolve/hello-curl-not-in-toplevel.yaml"))
                .await;
        let output = assert_build_status(&flox, &mut env, &package_name, None, false);

        let store_path_prefix_pattern = r"/nix/store/[\w]{32}";
        let expected_pattern = formatdoc! {r#"
            ❌ ERROR: Nonexistent path reference to '\$FLOX_ENV/bin/{bin_name}' found in package '{package_name}':
                Hint: {hint}
            Path referenced by: {store_path_prefix_pattern}-{package_name}-0.0.0/bin/.test-wrapped
              Nonexistent path: {store_path_prefix_pattern}-environment-build-{package_name}/bin/{bin_name}
        "#};
        let re = regex::Regex::new(&expected_pattern).unwrap();
        if !re.is_match(&output.stderr) {
            pretty_assertions::assert_eq!(
                output.stderr,
                expected_pattern,
                "didn't find expected pattern, diffing entire output"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn nonexistent_path_check_in_runtime_packages() {
        let bin_name = String::from("hello");
        let hint = format!("consider adding package '{bin_name}' to 'runtime-packages'");
        assert_nonexistent_path_check_failure(&bin_name, &hint).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn nonexistent_path_check_not_in_toplevel() {
        let bin_name = String::from("curl");
        let hint = format!("consider moving package '{bin_name}' to 'toplevel' pkg-group");
        assert_nonexistent_path_check_failure(&bin_name, &hint).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn nonexistent_path_check_complete_fiction() {
        let bin_name = String::from("not-found");
        let hint = format!(
            "check your build script and project files for any mention of the '{bin_name}' string"
        );
        assert_nonexistent_path_check_failure(&bin_name, &hint).await;
    }

    #[test]
    fn cleans_up_data_sandbox() {
        let package_name = String::from("foo");
        let file_name = String::from("bar");
        let file_content = String::from("some content");

        let manifest = formatdoc! {r#"
            version = 1

            [build.{package_name}]
            sandbox = "pure"
            command = """
                mkdir $out
                echo "{file_content}" > $out/{file_name}
            """
        "#};

        let (flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, &manifest);
        let env_path = env.parent_path().unwrap();

        let _git = GitCommandProvider::init(&env_path, false).unwrap();

        let result = result_dir(&env_path, &package_name);
        let cache = cache_dir(&env_path, &package_name);

        assert_build_status(&flox, &mut env, &package_name, None, true);

        assert!(result.exists());
        assert!(cache.exists());

        assert_clean_success(&flox, &mut env, &[&package_name]);
        assert!(!result.exists());
        assert!(!cache.exists());
    }

    #[test]
    fn cleans_up_data_no_sandbox() {
        let package_name = String::from("foo");
        let file_name = String::from("bar");
        let file_content = String::from("some content");

        let manifest = formatdoc! {r#"
            version = 1

            [build.{package_name}]
            sandbox = "off"
            command = """
                mkdir $out
                echo "{file_content}" > $out/{file_name}
            """
        "#};

        let (flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, &manifest);
        let env_path = env.parent_path().unwrap();

        let result = result_dir(&env_path, &package_name);

        assert_build_status(&flox, &mut env, &package_name, None, true);

        assert!(result.exists());

        assert_clean_success(&flox, &mut env, &[&package_name]);
        assert!(!result.exists());
    }

    #[test]
    fn cleans_up_all() {
        let package_foo = String::from("foo");
        let package_bar = String::from("bar");

        let file_name = String::from("file");
        let file_content = String::from("some content");

        let manifest = formatdoc! {r#"
            version = 1

            [build.{package_foo}]
            sandbox = "pure"
            command = """
                mkdir $out
                echo "{file_content}" > $out/{file_name}
            """
            [build.{package_bar}]
            sandbox = "off"
            command = """
                mkdir $out
                echo "{file_content}" > $out/{file_name}
            """
        "#};

        let (flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, &manifest);
        let env_path = env.parent_path().unwrap();

        let _git = GitCommandProvider::init(&env_path, false).unwrap();

        let result_foo = result_dir(&env_path, &package_foo);
        let cache_foo = cache_dir(&env_path, &package_foo);
        let result_bar = result_dir(&env_path, &package_bar);

        assert_build_status(&flox, &mut env, &package_foo, None, true);
        assert_build_status(&flox, &mut env, &package_bar, None, true);

        assert!(result_foo.exists());
        assert!(cache_foo.exists());
        assert!(result_bar.exists());

        assert_clean_success(&flox, &mut env, &[]);
        assert!(!result_foo.exists());
        assert!(!cache_foo.exists());
        assert!(!result_bar.exists());
    }

    #[test]
    fn dollar_out_persisted_no_sandbox() {
        let package_name = String::from("foo");

        let manifest = formatdoc! {r#"
            version = 1

            [build.{package_name}]
            sandbox = "off"
            command = """
                echo "Hello, World!" >> $out
                exit 42
            """
        "#};

        let (flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, &manifest);

        let output = temp_env::with_var("_FLOX_SUBSYSTEM_VERBOSITY", Some("1"), || {
            assert_build_status(&flox, &mut env, &package_name, None, false)
        });

        let out_path_message_regex = regex::Regex::new("out=(.+?)\\s").unwrap();

        let out_path = match out_path_message_regex.captures(&output.stdout) {
            Some(captures) => Path::new(captures.get(1).unwrap().as_str()),
            None => panic!("$out path not found in stdout"),
        };

        assert!(out_path.exists(), "out_path not found: {out_path:?}");

        let out_content = fs::read_to_string(out_path).unwrap();
        assert_eq!(out_content, "Hello, World!\n");
    }

    fn build_script_persisted(mode: &str, succeed: bool) {
        let package_name = String::from("foo");

        let command = if succeed {
            r#"echo "Hello, World!" >> $out"#
        } else {
            "exit 42"
        };

        let manifest = formatdoc! {r#"
            version = 1

            [build.{package_name}]
            sandbox = "{mode}"
            command = '{command}'
        "#};

        let (flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, &manifest);
        let env_path = env.parent_path().unwrap();

        let _git = GitCommandProvider::init(&env_path, false).unwrap();

        let output = temp_env::with_var("_FLOX_SUBSYSTEM_VERBOSITY", Some("1"), || {
            assert_build_status(&flox, &mut env, &package_name, None, succeed)
        });

        let build_script_path_message_regex =
            regex::Regex::new(r#"bash -e (.+/build.bash)|--argstr buildScript "(.+build.bash)""#)
                .unwrap();

        let build_script_path = match build_script_path_message_regex.captures(&output.stdout) {
            Some(captures) => Path::new(
                captures
                    .get(1)
                    .or_else(|| captures.get(2))
                    .unwrap()
                    .as_str(),
            ),
            None => panic!("$build_script_path not found in stdout"),
        };

        assert!(
            build_script_path.exists(),
            "build_script_path not found: {build_script_path:?}"
        );
    }

    #[test]
    fn build_script_persisted_pure_on_success() {
        build_script_persisted("pure", true);
    }

    #[test]
    fn build_script_persisted_pure_on_failure() {
        build_script_persisted("pure", false);
    }

    #[test]
    fn build_script_persisted_no_sandbox_on_success() {
        build_script_persisted("off", true);
    }

    #[test]
    fn build_script_persisted_no_sandbox_on_failure() {
        build_script_persisted("off", false);
    }

    fn assert_derivation_metadata_propagated(keypath: &[&str], expected: &str, store_path: &Path) {
        let child = Command::new("nix")
            .args([
                "--extra-experimental-features",
                "nix-command",
                "derivation",
                "show",
                store_path.to_str().unwrap(),
            ])
            .stderr(Stdio::inherit())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.wait_with_output().unwrap().stdout;
        let drv = serde_json::from_slice::<serde_json::Value>(&stdout).unwrap();
        // `nix derivation show` prints a map with the .drv path as the key
        // We just care about the value and discard the key
        let mut current = drv.as_object().unwrap().values().next().unwrap();
        for key in keypath {
            current = &current[key];
        }
        let drv_value = current.as_str().unwrap();
        assert_eq!(drv_value, expected);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn build_version_propagated() {
        let pname = "foo".to_string();
        let version = "4.2.0";
        let version_file = "VERSION";

        for sandbox_mode in ["off", "pure"].iter() {
            let version_specs = [
                format!("version = '{version}'"),
                format!("version.file = '{version_file}'"),
                format!("version.command = 'echo {version}'"),
                format!("version.command = 'echo $(echo {version})'"),
                // Verify that the command is invoked from within the activated
                // environment with access to the "hello" command.
                format!("version.command = 'hello >/dev/null && echo {version}'"),
            ];

            for version_spec in version_specs {
                let manifest = formatdoc! {r#"
                    version = 1
                    [install]
                    hello.pkg-path = "hello"

                    [build.{pname}]
                    sandbox = "{sandbox_mode}"
                    command = """
                        echo "foo" > $out
                    """
                    {version_spec}
                "#};
                let (mut flox, _temp_dir_handle) = flox_instance();
                let mut env = new_path_environment(&flox, &manifest);
                let env_path = env.parent_path().unwrap();

                fs::write(env_path.join(version_file), version).unwrap();

                let _git = GitCommandProvider::init(&env_path, false).unwrap();
                flox.catalog_client =
                    catalog_replay_client(GENERATED_DATA.join("resolve/hello.yaml")).await;
                let collected = assert_build_status(&flox, &mut env, &pname, None, true);
                let result_path = env_path.join(format!("result-{pname}"));
                let build_results = collected.build_results.unwrap();

                assert_eq!(build_results.len(), 1);
                assert_eq!(build_results[0].version, version);
                let realpath = std::fs::read_link(&result_path).unwrap();
                assert_derivation_metadata_propagated(&["env", "version"], version, &realpath);
            }
        }
    }

    fn build_does_not_run_profile(sandbox: bool) {
        let package_name = String::from("foo");
        let file_name = String::from("bar");
        let file_content = String::from("some content");

        let manifest = formatdoc! {r#"
            version = 1
            [profile]
            common = """
                export FOO=profile
                exit 1
            """

            [build.{package_name}]
            command = """
                mkdir $out
                if [ -n "$FOO" ]; then
                    echo "profile should not have run"
                    exit 1
                fi
                echo -n "{file_content}" > $out/{file_name}
            """
            sandbox = "{}"
        "#, if sandbox { "pure" } else { "off" }};

        let (flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, &manifest);
        let env_path = env.parent_path().unwrap();

        if sandbox {
            let _git = GitCommandProvider::init(&env_path, false).unwrap();
        }

        assert_build_status(&flox, &mut env, &package_name, None, true);
        assert_build_file(&env_path, &package_name, &file_name, &file_content);
    }

    #[test]
    fn build_does_not_run_profile_sandbox_off() {
        build_does_not_run_profile(false);
    }

    #[test]
    fn build_does_not_run_profile_sandbox_pure() {
        build_does_not_run_profile(true);
    }

    fn build_patch_shebangs_prefers_build_env(sandbox: bool) {
        let package_name = String::from("foo");
        let file_name = String::from("bar");

        let (flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment_from_env_files(&flox, GENERATED_DATA.join("envs/bash"));
        let env_path = env.parent_path().unwrap();

        let base_manifest = env.manifest_contents(&flox).unwrap();
        let build_manifest = formatdoc! {r#"
            {base_manifest}

            [build.{package_name}]
            command = """
                mkdir -p $out/bin
                cat > $out/bin/{file_name} <<EOF
                #!/usr/bin/env bash
                echo "Hello, World!"
                EOF
                chmod +x $out/bin/{file_name}
            """
            sandbox = "{}"
        "#, if sandbox { "pure" } else { "off" }};
        env.edit(&flox, build_manifest).unwrap();

        if sandbox {
            let _git = GitCommandProvider::init(&env_path, false).unwrap();
        }

        let output = assert_build_status(&flox, &mut env, &package_name, None, true);

        let store_path_prefix_pattern = r"/nix/store/[\w]{32}";
        let expected_pattern = formatdoc! {r##"
            interpreter directive changed from "#!/usr/bin/env bash" to "{store_path_prefix_pattern}-environment-build-{package_name}/bin/bash"
        "##};
        let re = regex::Regex::new(&expected_pattern).unwrap();
        assert!(
            re.is_match(&output.stderr),
            "expected STDERR to match regex",
        );
    }

    #[test]
    fn build_patch_shebangs_prefers_build_env_sandbox_off() {
        build_patch_shebangs_prefers_build_env(false);
    }

    #[test]
    fn build_patch_shebangs_prefers_build_env_sandbox_pure() {
        build_patch_shebangs_prefers_build_env(true);
    }

    /// Test that patchShebangs is able to substitute the path for `cat`
    /// as provided by Nix runCommmand by way of the `coreutils` package.
    /// If it uses a version of `coreutils` from a different nixpkgs
    /// revision then the build will fail the closure check, and
    /// `assert_build_status()` will flag the error accordingly.
    fn build_patch_shebangs_falls_back_to_correct_nixpkgs(sandbox: bool) {
        let package_name = String::from("foo");
        let file_name = String::from("bar");

        let manifest = formatdoc! {r#"
            version = 1

            [build.{package_name}]
            command = """
                mkdir -p $out/bin
                cat > $out/bin/{file_name} <<EOF
                #!/usr/bin/env cat
                echo "Hello, World!"
                EOF
                chmod +x $out/bin/{file_name}
            """
            sandbox = "{}"
        "#, if sandbox { "pure" } else { "off" }};
        let (flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, &manifest);
        let env_path = env.parent_path().unwrap();

        if sandbox {
            let _git = GitCommandProvider::init(&env_path, false).unwrap();
        }

        let output = assert_build_status(&flox, &mut env, &package_name, None, true);

        let store_path_prefix_pattern = r"/nix/store/[\w]{32}";
        let expected_pattern = formatdoc! {r##"
            interpreter directive changed from "#!/usr/bin/env cat" to "{store_path_prefix_pattern}-coreutils-[\d.]*/bin/cat"
        "##};
        let re = regex::Regex::new(&expected_pattern).unwrap();
        assert!(
            re.is_match(&output.stderr),
            "expected STDERR to match regex",
        );
    }

    #[test]
    fn build_patch_shebangs_falls_back_to_correct_nixpkgs_sandbox_off() {
        build_patch_shebangs_falls_back_to_correct_nixpkgs(false);
    }

    #[test]
    fn build_patch_shebangs_falls_back_to_correct_nixpkgs_sandbox_pure() {
        build_patch_shebangs_falls_back_to_correct_nixpkgs(true);
    }

    #[test]
    fn hello_world_builds() {
        let (flox, tmpdir) = flox_instance();
        assert_manifest_build_succeeds(GENERATED_DATA.join("build/hello"), "hello", &flox, tmpdir);
    }

    /// Test that patchShebangs is able to substitute the path for `cat`
    /// as provided by Nix runCommmand by way of the `coreutils` package.
    /// If it uses a version of `coreutils` from a different nixpkgs
    /// revision then the build will fail the closure check, and
    /// `assert_build_status()` will flag the error accordingly.
    fn build_do_not_eval_with_nixpkgs_from_toplevel(sandbox: bool) {
        let package_name = String::from("foo");
        let file_name = String::from("bar");

        let (flox, _temp_dir_handle) = flox_instance();
        // hello@2.10 sets toplevel to an old nixpkgs revision that will
        // cause the closure check to fail if build-manifest.nix pulls in
        // `bash` from the toplevel nixpkgs.
        let mut env =
            new_path_environment_from_env_files(&flox, GENERATED_DATA.join("envs/old_hello"));
        let env_path = env.parent_path().unwrap();

        let base_manifest = env.manifest_contents(&flox).unwrap();
        let build_manifest = formatdoc! {r##"
            {base_manifest}

            [build.{package_name}]
            command = """
                mkdir -p $out/bin
                echo "#!/usr/bin/env bash" > $out/bin/{file_name}
                type -p hello >> $out/bin/{file_name}
                chmod +x $out/bin/{file_name}
            """
            sandbox = "{}"
        "##, if sandbox { "pure" } else { "off" }};
        env.edit(&flox, build_manifest).unwrap();

        if sandbox {
            let _git = GitCommandProvider::init(&env_path, false).unwrap();
        }

        let output = assert_build_status(&flox, &mut env, &package_name, None, true);

        let store_path_prefix_pattern = r"/nix/store/[\w]{32}";
        let expected_pattern = formatdoc! {r##"
            {store_path_prefix_pattern}-{package_name}-0.0.0/bin/{file_name}: interpreter directive changed from "#!/usr/bin/env bash" to "{store_path_prefix_pattern}-bash-.*/bin/bash"
        "##};
        let re = regex::Regex::new(&expected_pattern).unwrap();
        assert!(
            re.is_match(&output.stderr),
            "expected STDERR to match {re}: {}",
            output.stderr
        );
    }

    #[test]
    fn build_do_not_eval_with_nixpkgs_from_toplevel_sandbox_off() {
        build_do_not_eval_with_nixpkgs_from_toplevel(false);
    }

    #[test]
    fn build_do_not_eval_with_nixpkgs_from_toplevel_sandbox_pure() {
        build_do_not_eval_with_nixpkgs_from_toplevel(true);
    }

    async fn build_symlinks_can_refer_to_flox_env(sandbox: &str) {
        let package_name = String::from("foo");
        let manifest = formatdoc! {r#"
            version = 1
            [install]
            hello.pkg-path = "hello"

            [build.{package_name}]
            sandbox = "{sandbox}"
            command = """
                mkdir -p $out/bin
                ln -s $FLOX_ENV/bin/hello $out/bin
            """
        "#};

        let (mut flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, &manifest);
        let env_path = env.parent_path().unwrap();
        let _ = GitCommandProvider::init(&env_path, false).unwrap();
        flox.catalog_client =
            catalog_replay_client(GENERATED_DATA.join("resolve/hello.yaml")).await;
        assert_build_status(&flox, &mut env, &package_name, None, true);

        let result_path = result_dir(&env_path, &package_name)
            .join("bin")
            .join("hello");
        let output = Command::new(&result_path).output().unwrap();
        assert!(
            output.status.success(),
            "should execute successfully, stderr: {:?}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim_end(),
            "Hello, world!"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn build_symlinks_can_refer_to_flox_env_sandbox_pure() {
        build_symlinks_can_refer_to_flox_env("pure").await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn build_symlinks_can_refer_to_flox_env_sandbox_off() {
        build_symlinks_can_refer_to_flox_env("off").await;
    }

    fn build_has_access_to_user_provided_path(sandbox: bool) {
        let package_name = String::from("foo");
        let file_name = String::from("bar");
        let (flox, tmpdir) = flox_instance();

        // Create tmpdir/testbin and put a script in it.
        let test_script_name = String::from("test-script-in-PATH");
        let test_script_output = String::from("123456");
        let test_bin = tmpdir.path().join("test-bin");
        fs::create_dir_all(&test_bin).unwrap();
        let test_script_path = test_bin.join(&test_script_name);
        fs::write(
            &test_script_path,
            format!("#!/usr/bin/env bash\necho {test_script_output}"),
        )
        .unwrap();
        // Make the script executable.
        fs::set_permissions(&test_script_path, fs::Permissions::from_mode(0o755)).unwrap();

        // Construct a PATH string with test_bin first.
        let path_var = std::env::var("PATH").context("Could not read PATH variable");
        let test_bin_first_path = format!(
            "{}:{}",
            test_bin.to_string_lossy(),
            path_var.unwrap_or_default()
        );

        // Create a manifest that uses the script in the build command.
        let manifest = formatdoc! {r##"
            version = 1

            [build.{package_name}]
            command = """
              echo "Expecting to find '{test_script_name}' in PATH"
              mkdir -p $out/bin
              echo "#!/usr/bin/env bash" > $out/bin/{file_name}
              type -p {test_script_name} >> $out/bin/{file_name}
              chmod +x $out/bin/{file_name}
            """
            sandbox = "{}"
        "##, if sandbox { "pure" } else { "off" }}; // [sic] sandbox can be "warn" and "enforce" too

        // Build package.
        let mut env = new_path_environment(&flox, &manifest);
        let env_path = env.parent_path().unwrap();

        if sandbox {
            let _git = GitCommandProvider::init(&env_path, false).unwrap();
        }

        // Perform build with the modified PATH.
        temp_env::with_var("PATH", Some(&test_bin_first_path), || {
            assert_build_status(&flox, &mut env, &package_name, None, !sandbox)
        });

        // The pure build not expected to succeed.
        if sandbox {
            return;
        }

        // Confirm script emits the expected output, referencing script from PATH.
        let result_path = result_dir(&env_path, &package_name)
            .join("bin")
            .join(&file_name);
        let output = Command::new(&result_path).output().unwrap();
        assert!(
            output.status.success(),
            "should execute successfully, stderr: {:?}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim_end(),
            test_script_output,
        );
    }

    #[test]
    fn build_has_access_to_user_provided_path_sandbox_off() {
        build_has_access_to_user_provided_path(false);
    }

    #[test]
    fn build_does_not_have_access_to_user_provided_path_sandbox_pure() {
        build_has_access_to_user_provided_path(true);
    }

    fn build_has_access_to_stdenv_packages(sandbox: bool) {
        let package_name = String::from("foo");
        let file_name = String::from("bar");
        let (flox, _tmpdir) = flox_instance();

        let manifest = formatdoc! {r##"
            version = 1

            [build.{package_name}]
            command = """
              # The Rust '.*' regex doesn't match multiple lines, so turn off
              # shell tracing so that all we get from the build is the output.
              set +x
              # Report where we find a representative executable from each pkg.
              # Print this to stdout so that we can assert it in the test, but
              # do not embed these paths in the output because those packages
              # are not expected to be present in the final closure.
              for i in bash cat find grep sed; do
                path="$(type -p $i)"
                echo "found $i in $path"
              done
              # Create a valid executable in $out/bin for a clean build.
              mkdir -p $out/bin
              echo true > $out/bin/{file_name}
              chmod +x $out/bin/{file_name}
            """
            sandbox = "{}"
        "##, if sandbox { "pure" } else { "off" }}; // [sic] sandbox can be "warn" and "enforce" too

        // Build package.
        let mut env = new_path_environment(&flox, &manifest);
        let env_path = env.parent_path().unwrap();

        if sandbox {
            let _git = GitCommandProvider::init(&env_path, false).unwrap();
        }

        // Perform build.
        let output = assert_build_status(&flox, &mut env, &package_name, None, true);

        // Look for expected output in build.
        let store_path_prefix_pattern = r"/nix/store/[\w]{32}";
        let expected_pattern = formatdoc! {r#"
            .*found bash in {store_path_prefix_pattern}-bash-[\w\d.-]*/bin/bash.*
            .*found cat in {store_path_prefix_pattern}-coreutils-[\w\d.-]*/bin/cat.*
            .*found find in {store_path_prefix_pattern}-findutils-[\w\d.-]*/bin/find.*
            .*found grep in {store_path_prefix_pattern}-gnugrep-[\w\d.-]*/bin/grep.*
            .*found sed in {store_path_prefix_pattern}-gnused-[\w\d.-]*/bin/sed.*
        "#};
        let re = regex::Regex::new(&expected_pattern).unwrap();

        // Assert that the expected output is present in the build output.
        // Note that this output will appear in the stdout for the local
        // build and stderr for the sandbox build.
        let output_stream = if sandbox {
            output.stderr
        } else {
            output.stdout
        };
        if !re.is_match(&output_stream) {
            pretty_assertions::assert_eq!(
                output_stream,
                expected_pattern,
                "didn't find expected pattern, diffing entire output"
            );
        }
    }

    #[test]
    fn build_has_access_to_stdenv_packages_sandbox_off() {
        build_has_access_to_stdenv_packages(false);
    }

    #[test]
    fn build_has_access_to_stdenv_packages_sandbox_pure() {
        build_has_access_to_stdenv_packages(true);
    }

    async fn build_result_only_has_runtime_packages(sandbox: bool) {
        let package_name = String::from("foo");
        let file_name = String::from("bar");

        let manifest = formatdoc! {r#"
            version = 1
            [install]
            hello.pkg-path = "hello"
            curl.pkg-path = "curl"
            curl.pkg-group = "not-toplevel"

            [build.{package_name}]
            command = """
                mkdir -p $out/bin
                cat > $out/bin/{file_name} <<EOF
                #!/usr/bin/env bash -x
                hello_path="\\$(type -p hello || echo notfound)"
                if [ "\\$hello_path" != "$FLOX_ENV/bin/hello" ]; then
                    echo "hello not found in build environment" 1>&2
                    exit 1
                fi
                curl_path="\\$(type -p curl || echo notfound)"
                # Insert quotes in the middle of the path to prevent the existence
                # check from failing the build on account of a missing path.
                if [ "\\$curl_path" == "$FLOX_ENV/bin""/curl" ]; then
                    echo "curl found at '\\$curl_path' but should not be in the build environment" 1>&2
                    exit 1
                fi
                EOF
                chmod +x $out/bin/{file_name}
            """
            runtime-packages = [ "hello" ]
            sandbox = "{}"
        "#, if sandbox { "pure" } else { "off" }};

        let (mut flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, &manifest);
        let env_path = env.parent_path().unwrap();

        if sandbox {
            let _git = GitCommandProvider::init(&env_path, false).unwrap();
        }

        flox.catalog_client =
            catalog_replay_client(GENERATED_DATA.join("resolve/hello-curl-not-in-toplevel.yaml"))
                .await;
        assert_build_status(&flox, &mut env, &package_name, None, true);

        // Confirm script runs successfully.
        let result_path = result_dir(&env_path, &package_name)
            .join("bin")
            .join(&file_name);
        let output = Command::new(&result_path).output().unwrap();
        assert!(
            output.status.success(),
            "should execute successfully, stderr: {:?}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn build_result_only_has_runtime_packages_sandbox_off() {
        build_result_only_has_runtime_packages(false).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn build_result_only_has_runtime_packages_sandbox_pure() {
        build_result_only_has_runtime_packages(true).await;
    }

    /// Test that cmake can make use of the CMAKE_PREFIX_PATH variable as
    /// set by etc-profiles.
    async fn build_can_use_cmake(sandbox: bool) {
        let package_name = String::from("hello-cmake");
        // from: test_data/input_data/build/hello-cmake/HelloTarget/share/hello/HelloTarget.cmake
        let file_name = String::from("hello.txt");
        // from: test_data/input_data/build/hello-cmake/hello.in
        let file_content = String::from("Hello, world!\n");

        // Start by initializing a flox instance and tmpdir.
        let (mut flox, _temp_dir_handle) = flox_instance();

        // We have to materialize the environment at test time because of the
        // requirement to install by storepath, so we start copying in the
        // build environment which is comprised of source code and an empty
        // flox environment.
        let path = GENERATED_DATA.join("build/hello-cmake");
        let path: &Path = path.as_ref();
        copy_dir_recursive(path, &flox.temp_dir, true).unwrap();

        // Import the HelloTarget directory as a package to be installed.
        let output = Command::new("nix")
            .current_dir(&flox.temp_dir)
            .args([
                "--extra-experimental-features",
                "nix-command",
                "store",
                "add",
                "HelloTarget",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "should execute successfully, stderr: {:?}",
            String::from_utf8_lossy(&output.stderr)
        );

        // stdout contains the full store path of the added package and a
        // trailing newline. Strip the newline to get the path to install.
        let hello_target_pkg = String::from_utf8_lossy(&output.stdout);
        let hello_target_pkg = hello_target_pkg.trim_end();

        // Materialize the environment, including the HelloTarget package.
        let system = env!("system");
        let manifest = formatdoc! {r#"
            version = 1

            [install]
            cmake.pkg-path = "cmake"
            # Nix build of cmake does not include gnumake as a dependency?
            gnumake.pkg-path = "gnumake"
            # Install HelloTarget package by store path for the test.
            HelloTarget.store-path = "{hello_target_pkg}"
            HelloTarget.systems = ["{system}"]

            [build.hello-cmake]
            command = '''
              mkdir build && cd build
              cmake .. && cmake --build .
              mkdir $out && cp {file_name} $out
            '''
            sandbox = "{}"
        "#, if sandbox { "pure" } else { "off" }}; // [sic] sandbox can be "warn" and "enforce" too

        flox.catalog_client =
            catalog_replay_client(GENERATED_DATA.join("resolve/cmake-gnumake.yaml")).await;

        let mut env = new_path_environment(&flox, &manifest);
        let env_path = env.parent_path().unwrap();

        // Rename the source files from flox.temp_dir to env_path.
        let source_files = ["CMakeLists.txt", "hello.in"];
        for file in source_files {
            let src = flox.temp_dir.join(file);
            let dst = env_path.join(file);
            fs::rename(src, dst)
                .unwrap_or_else(|_| panic!("Failed to rename {file} to {env_path:?}"));
        }

        // Add the source files to git, if required.
        if sandbox {
            let git = GitCommandProvider::init(&env_path, false).unwrap();
            for file in source_files {
                git.add(&[&PathBuf::from(file)]).unwrap();
            }
        }

        assert_build_status(&flox, &mut env, &package_name, None, true);
        assert_build_file(&env_path, &package_name, &file_name, &file_content);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn build_can_use_cmake_sandbox_off() {
        build_can_use_cmake(false).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn build_can_use_cmake_sandbox_pure() {
        build_can_use_cmake(true).await;
    }

    #[test]
    fn pure_manifest_builds_succeed_with_deleted_tracked_files() {
        let package_name = "foo";
        let file_name = "bar";
        let deleted_file_name = "to-be-deleted";

        let manifest = formatdoc! {r#"
            version = 1
            [install]

            [build.{package_name}]
            sandbox = "pure"
            command = """
                mkdir $out
                touch $out/{file_name}
            """
        "#};

        let (flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, &manifest);
        let env_path = env.parent_path().unwrap();

        let deleted_file_path = env_path.join(deleted_file_name);
        File::create(&deleted_file_path).unwrap();

        let git = GitCommandProvider::init(&env_path, false).unwrap();

        // simulate deleting a tracked file (explicically, without 'git rm')
        git.add(&[&deleted_file_path]).unwrap();
        fs::remove_file(&deleted_file_path).unwrap();

        assert_build_status(&flox, &mut env, package_name, None, true);
    }
}

#[cfg(test)]
mod nef_tests {
    use std::fs;

    use indoc::{formatdoc, indoc};
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::flox::test_helpers::flox_instance;
    use crate::models::environment::path_environment::test_helpers::new_path_environment;
    use crate::providers::build::test_helpers::{
        assert_build_status_with_nix_expr,
        prepare_nix_expressions_in,
    };

    #[test]
    fn nef_build_creates_out_link() {
        let pname = "foo".to_string();

        let (flox, tempdir) = flox_instance();

        // Create a manifest (may be empty)
        let manifest = formatdoc! {r#"
            version = 1
        "#};
        let mut env = new_path_environment(&flox, &manifest);
        let env_path = env.parent_path().unwrap();

        // Create expressions
        let expressions_dir = prepare_nix_expressions_in(&tempdir, &[(&[&pname], indoc! {r#"
            {runCommand}: runCommand "{pname}" {} ''
                echo -n "Hello, World!" >> $out
            ''
            "#})]);

        // build
        let collected = assert_build_status_with_nix_expr(
            &flox,
            &mut env,
            &expressions_dir,
            &pname,
            None,
            true,
        );

        // assert results
        let result_path = env_path.join(format!("result-{pname}"));
        let build_results = collected.build_results.unwrap();
        assert_eq!(build_results.len(), 1);

        let content = fs::read_to_string(result_path).unwrap();
        assert_eq!(content, "Hello, World!");
    }

    #[test]
    fn nef_build_results_contain_common_metadata() {
        let pname = "foo".to_string();

        let (flox, tempdir) = flox_instance();

        // Create a manifest (may be empty)
        let manifest = formatdoc! {r#"
            version = 1
        "#};
        let mut env = new_path_environment(&flox, &manifest);

        // Create expressions
        let expressions_dir =
            prepare_nix_expressions_in(&tempdir, &[(&[&pname], &formatdoc! {r#"
            {{runCommand}}: runCommand "{pname}" {{
                version = "1.0.1";
                pname = "not-{pname}";
                outputs = ["out" "man" "lib"];
            }} ''
                echo -n "Hello, World!" >> $out
                touch $man
                touch $lib
            ''
            "#})]);

        // build
        let collected = assert_build_status_with_nix_expr(
            &flox,
            &mut env,
            &expressions_dir,
            &pname,
            None,
            true,
        );

        // assert results
        let build_results = collected.build_results.unwrap();
        assert_eq!(build_results.len(), 1);
        assert_eq!(build_results[0].name, pname);
        assert_eq!(build_results[0].pname, format!("not-{pname}"));
        assert_eq!(build_results[0].version, "1.0.1");
        for output in ["out", "lib", "man"] {
            assert!(build_results[0].outputs.contains_key(output));
        }
    }

    #[test]
    fn nef_builds_use_impure_evaluation() {
        let pname = "foo".to_string();

        let (flox, tempdir) = flox_instance();

        // Create a manifest (may be empty)
        let manifest = formatdoc! {r#"
            version = 1
        "#};
        let mut env = new_path_environment(&flox, &manifest);
        let env_path = env.parent_path().unwrap();

        // Create expressions
        let expressions_dir = prepare_nix_expressions_in(&tempdir, &[(&[&pname], indoc! {r#"
            {runCommand}: runCommand "{pname}" {} ''
                echo -n "${if builtins ? currentSystem then "impure" else "pure-eval"}" >> $out
            ''
            "#})]);

        // build
        let _collected = assert_build_status_with_nix_expr(
            &flox,
            &mut env,
            &expressions_dir,
            &pname,
            None,
            true,
        );

        // assert results
        let result_path = env_path.join(format!("result-{pname}"));
        let content = fs::read_to_string(result_path).unwrap();
        // currently an implication of using `nix eval --file` but may change in the future
        assert_eq!(content, "impure");
    }

    #[test]
    fn nef_builds_built_lazily() {
        let eval_success = "eval-success".to_string();
        let eval_failure = "eval-failure".to_string();

        let (flox, tempdir) = flox_instance();

        // Create a manifest (may be empty)
        let manifest = formatdoc! {r#"
            version = 1
        "#};
        let mut env = new_path_environment(&flox, &manifest);

        // Create expressions
        let expressions_dir = prepare_nix_expressions_in(&tempdir, &[
            (&[&eval_success], indoc! {r#"
            {runCommand}: runCommand "{eval_success}" {} ''
                touch $out
            ''
            "#}),
            (&[&eval_failure], r#"{}: throw "eval failure""#),
        ]);

        // build fails with eval failure
        assert_build_status_with_nix_expr(
            &flox,
            &mut env,
            &expressions_dir,
            &eval_failure,
            None,
            false,
        );

        // build succeeds if eval failure is in another expression
        assert_build_status_with_nix_expr(
            &flox,
            &mut env,
            &expressions_dir,
            &eval_success,
            None,
            true,
        );
    }

    #[test]
    fn manifest_builds_can_depend_on_nef() {
        // Bug: pname and attr_path need to match
        let pname_expression = "foo";
        let attr_path_expression = "foo";
        let pname_manifest_build = "bar";

        let (flox, tempdir) = flox_instance();

        // Create a manifest (may be empty)
        let manifest = formatdoc! {r#"
            version = 1
            [build.{pname_manifest_build}]
            command = '''
                cat ${{{attr_path_expression}}} | rev > $out
            '''
        "#};
        let mut env = new_path_environment(&flox, &manifest);
        let env_path = env.parent_path().unwrap();

        // Create expressions
        let expressions_dir =
            prepare_nix_expressions_in(&tempdir, &[(&[attr_path_expression], &formatdoc! {r#"
            {{runCommand}}: runCommand "{pname_expression}" {{}} ''
                echo "123" >> $out
            ''
            "#})]);

        // build
        let _collected = assert_build_status_with_nix_expr(
            &flox,
            &mut env,
            &expressions_dir,
            pname_manifest_build,
            None,
            true,
        );

        // assert results
        let result_path = env_path.join(format!("result-{pname_manifest_build}"));
        let content = fs::read_to_string(result_path).unwrap();
        assert_eq!(content, "321\n");
    }
}
