use std::collections::HashMap;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc::Receiver;
use std::sync::LazyLock;
use std::{env, thread};

use serde::Deserialize;
use tempfile::NamedTempFile;
use thiserror::Error;
use tracing::{debug, error, warn};

use super::buildenv::{BuildEnvOutputs, BuiltStorePath};
use crate::flox::Flox;
use crate::models::environment::{Environment, EnvironmentError};
use crate::utils::CommandExt;

pub const FLOX_RUNTIME_DIR_VAR: &str = "FLOX_RUNTIME_DIR";

static FLOX_BUILD_MK: LazyLock<PathBuf> = LazyLock::new(|| {
    std::env::var("FLOX_BUILD_MK")
        .unwrap_or_else(|_| env!("FLOX_BUILD_MK").to_string())
        .into()
});

static GNUMAKE_BIN: LazyLock<PathBuf> = LazyLock::new(|| {
    std::env::var("GNUMAKE_BIN")
        .unwrap_or_else(|_| env!("GNUMAKE_BIN").to_string())
        .into()
});

pub(super) static BUILDTIME_NIXPKGS_URL: LazyLock<String> = LazyLock::new(|| {
    std::env::var("COMMON_NIXPKGS_URL").unwrap_or_else(|_| env!("COMMON_NIXPKGS_URL").to_string())
});

pub trait ManifestBuilder {
    /// Build the specified packages defined in the environment at `flox_env`.
    /// The build process will start in the background.
    /// To process the output, the caller should iterate over the returned [BuildOutput].
    /// Once the process is complete, the [BuildOutput] will yield an [Output::Exit] message.
    fn build(
        &self,
        flox: &Flox,
        base_dir: &Path,
        built_environments: &BuildEnvOutputs,
        flox_interpreter: &Path,
        package: &[String],
    ) -> Result<BuildOutput, ManifestBuilderError>;

    fn clean(
        &self,
        flox: &Flox,
        base_dir: &Path,
        flox_env: &Path,
        package: &[String],
    ) -> Result<(), ManifestBuilderError>;
}

#[derive(Debug, Error)]
pub enum ManifestBuilderError {
    #[error("failed to call package builder: {0}")]
    CallBuilderError(#[source] std::io::Error),

    #[error("failed to create file for build results")]
    CreateBuildResultFile(#[source] std::io::Error),

    #[error("failed to clean up build artifacts: {stderr}")]
    RunClean {
        stdout: String,
        stderr: String,
        status: ExitStatus,
    },
}

#[derive(Debug)]
pub enum Output {
    /// A line of stdout output from the build process.
    Stdout(String),
    /// A line of stderr output from the build process.
    Stderr(String),
    /// The build process has successfully and produced the given [BuildResults].
    Success(BuildResults),
    /// The build process has failed with the given exit status.
    /// On error `flox-build.mk` will not produce a build result file,
    /// so we don't have build results to return.
    /// Todo: we may want to recombine [Output::Failure] and [Output::Success]
    /// eventually if `flox-build.mk` is updated to always produce a build result file,
    /// e.g. containing error context for the failing builds.
    Failure(ExitStatus),
}

#[derive(Debug, PartialEq, Deserialize, Default, derive_more::Deref)]
pub struct BuildResults(Vec<BuildResult>);

#[derive(Debug, PartialEq, Deserialize)]
pub struct BuildResult {
    pub pname: String,
    pub outputs: HashMap<String, BuiltStorePath>,
    pub version: String,
    pub log: BuiltStorePath,
}

/// Output received from an ongoing build process.
/// Callers of [ManifestBuilder::build] should iterate over this type
/// to process the output of the build process and await its completion.
#[must_use = "The build process is started in the background.
To process the output and wait for the process to finish,
iterate over the returned BuildOutput."]
pub struct BuildOutput {
    receiver: Receiver<Output>,
}

impl Iterator for BuildOutput {
    type Item = Output;

    fn next(&mut self) -> Option<Self::Item> {
        self.receiver.recv().ok()
    }
}

/// A manifest builder that uses the [FLOX_BUILD_MK] makefile to build packages.
pub struct FloxBuildMk;

impl FloxBuildMk {
    fn base_command(&self, flox: &Flox, base_dir: &Path) -> Command {
        // todo: extra makeflags, eventually
        let mut command = Command::new(&*GNUMAKE_BIN);
        command.env_remove("MAKEFLAGS");
        command.arg("--file").arg(&*FLOX_BUILD_MK);
        command.arg("--directory").arg(base_dir); // Change dir before reading makefile.
        if flox.verbosity <= 0 {
            command.arg("--no-print-directory"); // Only print directory with -v.
        }

        command
    }
}

impl ManifestBuilder for FloxBuildMk {
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
    /// The build process will start in the background.
    /// To process the output, the caller should iterate over the returned [BuildOutput].
    /// Once the process is complete, the [BuildOutput] will yield an [Output::Exit] message.
    fn build(
        &self,
        flox: &Flox,
        base_dir: &Path,
        built_environments: &BuildEnvOutputs,
        flox_interpreter: &Path,
        packages: &[String],
    ) -> Result<BuildOutput, ManifestBuilderError> {
        let mut command = self.base_command(flox, base_dir);
        command.arg("build");
        command.arg(format!("BUILDTIME_NIXPKGS_URL={}", &*BUILDTIME_NIXPKGS_URL));
        command.arg(format!("FLOX_ENV={}", built_environments.develop.display()));
        command.arg(format!(
            "FLOX_ENV_OUTPUTS={}",
            serde_json::json!(built_environments)
        ));
        command.arg(format!("FLOX_INTERPRETER={}", flox_interpreter.display()));

        // Add the list of packages to be built by passing a space-delimited list
        // of pnames in the PACKAGES variable. If no packages are specified then
        // the makefile will build all packages by default.
        command.arg(format!("PACKAGES={}", packages.join(" ")));

        let build_result_path = NamedTempFile::new_in(&flox.temp_dir)
            .map_err(ManifestBuilderError::CreateBuildResultFile)?
            .into_temp_path();

        // SAFETY: according to the docs, this is fallible on _Windows_
        let build_result_path = build_result_path
            .keep()
            .expect("failed to keep build result fifo");

        command.arg(format!("BUILD_RESULT_FILE={}", build_result_path.display()));

        // activate needs this var
        // TODO: we should probably figure out a more consistent way to pass
        // this since it's also passed for `flox activate`
        command.env(FLOX_RUNTIME_DIR_VAR, &flox.runtime_dir);

        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());

        debug!(command = %command.display(), "running manifest build target");

        let mut child = command
            .spawn()
            .map_err(ManifestBuilderError::CallBuilderError)?;

        let (sender, receiver) = std::sync::mpsc::channel();
        let stdout_sender = sender.clone();
        let stderr_sender = sender.clone();
        let command_status_sender = sender;

        let stdout = child.stdout.take().unwrap();
        std::thread::spawn(move || {
            let stdout = std::io::BufReader::new(stdout);
            read_output_to_channel(stdout, stdout_sender, Output::Stdout);
        });

        let stderr = child.stderr.take().unwrap();
        std::thread::spawn(move || {
            let stderr = std::io::BufReader::new(stderr);
            read_output_to_channel(stderr, stderr_sender, Output::Stderr);
        });

        thread::spawn(move || {
            let status = child.wait().expect("failed to wait on child");

            // `flox-build.mk` will not produce/write to a build result file on failure.
            if !status.success() {
                let _ = command_status_sender.send(Output::Failure(status));
                return;
            }

            // TODO: should we bubble up errors through the channel?
            let build_results = 'results: {
                let build_results = match std::fs::read_to_string(&build_result_path) {
                    Ok(build_results) => build_results,
                    Err(e) => {
                        error!("failed to read build results file at {build_result_path:?}: {e}");
                        break 'results BuildResults::default();
                    },
                };

                match serde_json::from_str(&build_results) {
                    Ok(build_results) => build_results,
                    Err(e) => {
                        error!("failed to parse build results: {e}");
                        BuildResults::default()
                    },
                }
            };

            let _ = command_status_sender.send(Output::Success(build_results));
        });

        Ok(BuildOutput { receiver })
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
    fn clean(
        &self,
        flox: &Flox,
        base_dir: &Path,
        flox_env: &Path,
        packages: &[String],
    ) -> Result<(), ManifestBuilderError> {
        let mut command = self.base_command(flox, base_dir);
        // TODO: is this even necessary, or can we detect build outputs instead?
        command.arg(format!("FLOX_ENV={}", flox_env.display()));

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

        let output = command
            .output()
            .map_err(ManifestBuilderError::CallBuilderError)?;
        let status = output.status;

        if !status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();

            debug!(%status, %stderr, %stdout, "failed to clean build artifacts");

            return Err(ManifestBuilderError::RunClean {
                stdout,
                stderr,
                status,
            });
        }

        Ok(())
    }
}

/// Read output from a reader and send it to a channel
/// until the reader is exhausted or the receiver is dropped.
fn read_output_to_channel(
    reader: impl BufRead,
    sender: std::sync::mpsc::Sender<Output>,
    mk_output: impl Fn(String) -> Output,
) {
    for line in reader.lines() {
        let line = match line {
            Err(e) => {
                warn!("failed to read line: {e}");
                continue;
            },
            Ok(line) => line,
        };

        let Ok(_) = sender.send(mk_output(line)) else {
            // if the receiver is dropped, we can stop reading
            break;
        };
    }
}

pub fn build_symlink_path(
    environment: &impl Environment,
    package: &str,
) -> Result<PathBuf, EnvironmentError> {
    Ok(environment.parent_path()?.join(format!("result-{package}")))
}

pub mod test_helpers {
    use std::fs::{self};

    use super::*;
    use crate::flox::Flox;
    use crate::models::environment::path_environment::PathEnvironment;
    use crate::models::environment::Environment;

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

    /// Runs a build and asserts that the `ExitStatus` matches `expect_status`.
    /// STDOUT and STDERR are returned if you wish to make additional
    /// assertions on the output of the build.
    pub fn assert_build_status(
        flox: &Flox,
        env: &mut PathEnvironment,
        package_name: &str,
        expect_success: bool,
    ) -> CollectedOutput {
        let builder = FloxBuildMk;
        let output_stream = builder
            .build(
                flox,
                &env.parent_path().unwrap(),
                &env.build(flox).unwrap(),
                &env.rendered_env_links(flox).unwrap().development,
                &[package_name.to_owned()],
            )
            .unwrap();

        let mut output_stdout = String::new();
        let mut output_stderr = String::new();
        let mut output_build_results = None;
        for message in output_stream {
            match message {
                Output::Success(build_results) => {
                    assert!(expect_success, "expected build to fail");
                    let _ = output_build_results.insert(build_results);
                },
                Output::Failure(_) => {
                    assert!(!expect_success, "expected build to succeed");
                },
                Output::Stdout(line) => {
                    println!("stdout: {line}"); // To debug failing tests
                    output_stdout.push_str(&line);
                    output_stdout.push('\n');
                },
                Output::Stderr(line) => {
                    println!("stderr: {line}"); // To debug failing tests
                    output_stderr.push_str(&line);
                    output_stderr.push('\n');
                },
            }
        }

        CollectedOutput {
            build_results: output_build_results,
            stdout: output_stdout,
            stderr: output_stderr,
        }
    }

    pub fn assert_clean_success(flox: &Flox, env: &mut PathEnvironment, package_names: &[&str]) {
        let builder = FloxBuildMk;
        let err = builder
            .clean(
                flox,
                &env.parent_path().unwrap(),
                &env.rendered_env_links(flox).unwrap().development,
                &package_names
                    .iter()
                    .map(|s| s.to_string())
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
}

/// Unit tests for the `flox-build.mk` "black box" builder, via
/// the [`FloxBuildMk`] implementation of [`ManifestBuilder`].
///
/// Currently, this is _the_ testsuite for the `flox-build.mk` builder.
#[cfg(test)]
// TODO: https://github.com/flox/flox/issues/2185
// Serialise all build tests to workaround potential Nix bug.
// Use file-based locking to be compatible with `nextest`.
#[serial_test::file_serial(build)]
mod tests {
    use std::fs::{self};

    use indoc::{formatdoc, indoc};

    use super::test_helpers::*;
    use super::*;
    use crate::flox::test_helpers::flox_instance;
    use crate::models::environment::path_environment::test_helpers::{
        new_path_environment,
        new_path_environment_from_env_files,
    };
    use crate::models::environment::Environment;
    use crate::providers::catalog::test_helpers::reset_mocks_from_file;
    use crate::providers::catalog::GENERATED_DATA;
    use crate::providers::git::{GitCommandProvider, GitProvider};

    #[test]
    fn build_returns_failure_when_package_not_defined() {
        let package_name = String::from("foo");

        let (flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, "version = 1");

        assert_build_status(&flox, &mut env, &package_name, false);
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

        assert_build_status(&flox, &mut env, &package_name, true);
        assert_build_file(&env_path, &package_name, &file_name, &file_content);
    }

    #[test]
    fn build_no_dollar_out_sandbox_off() {
        let pname = String::from("foo");
        let name = String::from("foo-unknown");

        let manifest = formatdoc! {r#"
            version = 1

            [build.{pname}]
            command = "[ ! -e $out ]"
            sandbox = "off"
        "#};

        let (flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, &manifest);

        let output = assert_build_status(&flox, &mut env, &pname, false);

        let expected_output = formatdoc! {r#"
            {name}> ❌  ERROR: Build command did not copy outputs to '$out'.
            {name}>   - copy a single file with 'mkdir -p $out/bin && cp file $out/bin'
            {name}>   - copy a bin directory with 'mkdir $out && cp -r bin $out'
            {name}>   - copy multiple files with 'mkdir -p $out/bin && cp bin/* $out/bin'
            {name}>   - copy files from an Autotools project with 'make install PREFIX=$out'
        "#};
        assert!(
            output.stderr.contains(&expected_output),
            "{expected_output}"
        );
    }

    #[test]
    fn build_no_dollar_out_sandbox_pure() {
        let pname = String::from("foo");
        let name = String::from("foo-unknown");

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

        let output = assert_build_status(&flox, &mut env, &pname, false);

        let expected_output = formatdoc! {r#"
            {name}> ❌  ERROR: Build command did not copy outputs to '$out'.
            {name}>   - copy a single file with 'mkdir -p $out/bin && cp file $out/bin'
            {name}>   - copy a bin directory with 'mkdir $out && cp -r bin $out'
            {name}>   - copy multiple files with 'mkdir -p $out/bin && cp bin/* $out/bin'
            {name}>   - copy files from an Autotools project with 'make install PREFIX=$out'
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
        let name = String::from("foo-unknown");

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
        let output = assert_build_status(&flox, &mut env, &pname, true);

        // [sic] newline before 'HINT: ...' ignored in 'nix build -L' output:
        // <https://github.com/NixOS/nix/issues/11991>
        let expected_output = formatdoc! {r#"
            {name}> ⚠️  WARNING: $out/bin/not-executable is not executable.
            {name}> ⚠️  WARNING: $out/bin/subdir is not a file.
            {name}> ⚠️  WARNING: No executables found in '$out/bin'.
            {name}> Only executables in '$out/bin' will be available on the PATH.
            {name}> If your build produces executables, make sure they are copied to '$out/bin'.
            {name}>   - copy a single file with 'mkdir -p $out/bin && cp file $out/bin'
            {name}>   - copy a bin directory with 'mkdir $out && cp -r bin $out'
            {name}>   - copy multiple files with 'mkdir -p $out/bin && cp bin/* $out/bin'
            {name}>   - copy files from an Autotools project with 'make install PREFIX=$out'
            {name}> HINT: The following executables were found outside of '$out/bin':
            {name}>   - not-bin/hello
            {name}>   - bin/subdir/executable-in-subdir
        "#};
        assert!(
            output.stderr.contains(&expected_output),
            "{expected_output}"
        );
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
    #[ignore = "TODO: `files` isn't currently passed to or parsed by `flox-build.mk`."]
    fn build_includes_files() {
        let package_name = String::from("foo");
        let file_name = String::from("bar");
        let file_content = String::from("some content");

        let manifest = formatdoc! {r#"
            version = 1

            [build.{package_name}]
            files = ["{file_name}"]
            command = "mkdir $out"
        "#};

        let (flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, &manifest);
        let env_path = env.parent_path().unwrap();

        fs::write(env_path.join(&file_name), &file_content).unwrap();

        assert_build_status(&flox, &mut env, &package_name, true);
        assert_build_file(&env_path, &package_name, &file_name, &file_content);
    }

    #[test]
    #[ignore = "TODO: `systems` isn't currently passed to or parsed by `flox-build.mk`."]
    fn build_restricts_systems() {
        let package_name = String::from("foo");
        let file_name = String::from("bar");
        let file_content = String::from("some content");

        let manifest = formatdoc! {r#"
            version = 1

            [build.{package_name}]
            systems = ["invalid"]
            command = """
                mkdir $out
                echo -n {file_content} > $out/{file_name}
            """
        "#};

        let (flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, &manifest);
        let env_path = env.parent_path().unwrap();

        fs::write(env_path.join(&file_name), &file_content).unwrap();

        assert_build_status(&flox, &mut env, &package_name, false);
        let dir = result_dir(&env_path, &package_name);
        assert!(!dir.exists());
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
        let output = assert_build_status(&flox, &mut env, &package_name, false);
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
        assert_build_status(&flox, &mut env, &package_name, true);
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

        assert_build_status(&flox, &mut env, &package_name, true);
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

        assert_build_status(&flox, &mut env, &package_name, true);
        let file_content = result_content(&env_path, &package_name, &file_name);

        // Asserts that the build result uses the cached value of the previous build
        assert_build_status(&flox, &mut env, &package_name, true);
        assert_build_file(&env_path, &package_name, &file_name, &file_content);
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

        assert_build_status(&flox, &mut env, &package_name, true);
        let file_content_first_run = result_content(&env_path, &package_name, &file_name);

        let cache_dir = cache_dir(&env_path, &package_name);
        assert!(cache_dir.exists());
        fs::remove_file(cache_dir).unwrap();

        assert_build_status(&flox, &mut env, &package_name, true);
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

        assert_build_status(&flox, &mut env, &package_name, true);
        let file_content = result_content(&env_path, &package_name, &file_name);

        assert_build_status(&flox, &mut env, &package_name, true);
        assert_build_file(&env_path, &package_name, &file_name, &file_content);
    }

    #[test]
    fn build_uses_package_from_manifest() {
        let package_name = String::from("foo");
        let file_name = String::from("bar");
        let file_content = String::from("environment-build-foo/bin/hello\n");

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

        reset_mocks_from_file(&mut flox.catalog_client, "resolve/hello.json");
        assert_build_status(&flox, &mut env, &package_name, true);
        assert_build_file(&env_path, &package_name, &file_name, &file_content);
    }

    #[test]
    fn build_result_uses_package_from_environment() {
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

        reset_mocks_from_file(&mut flox.catalog_client, "resolve/hello.json");
        assert_build_status(&flox, &mut env, &package_name, true);

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

    #[test]
    fn build_uses_var_from_manifest() {
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
        "#};

        let (flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, &manifest);
        let env_path = env.parent_path().unwrap();

        assert_build_status(&flox, &mut env, &package_name, true);
        assert_build_file(&env_path, &package_name, &file_name, &file_content);
    }

    #[test]
    fn build_uses_hook_from_manifest() {
        let package_name = String::from("foo");
        let file_name = String::from("bar");
        let file_content = String::from("some content");

        let manifest = formatdoc! {r#"
            version = 1

            [hook]
            on-activate = '''
              export FOO="{file_content}"
            '''

            [build.{package_name}]
            command = """
                mkdir $out
                echo -n "$FOO" > $out/{file_name}
            """
        "#};

        let (flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, &manifest);
        let env_path = env.parent_path().unwrap();

        assert_build_status(&flox, &mut env, &package_name, true);
        assert_build_file(&env_path, &package_name, &file_name, &file_content);
    }

    #[test]
    fn build_depending_on_another_build() {
        let package_name = String::from("foo");
        let file_name = String::from("bar");
        let file_content = String::from("some content");

        let manifest = formatdoc! {r#"
            version = 1

            [build.dep]
            command = """
                mkdir $out
                echo -n "{file_content}" > $out/{file_name}
            """

            [build.{package_name}]
            command = """
                mkdir $out
                cp ${{dep}}/{file_name} $out/{file_name}
            """
        "#};

        let (flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, &manifest);
        let env_path = env.parent_path().unwrap();

        assert_build_status(&flox, &mut env, &package_name, true);
        assert_build_file(&env_path, &package_name, &file_name, &file_content);
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
        assert_build_status(&flox, &mut env, &package_name, true);
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
        assert_build_status(&flox, &mut env, &package_name, true);
        assert_build_file(&env_path, &package_name, &file_name, content_after);
    }

    #[test]
    fn build_wraps_binaries_with_preserved_arg0() {
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

        reset_mocks_from_file(&mut flox.catalog_client, "resolve/go.json");
        assert_build_status(&flox, &mut env, &package_name, true);
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

        assert_build_status(&flox, &mut env, &package_name, true);
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

    #[test]
    fn build_wraps_scripts_without_preserved_exe() {
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

        reset_mocks_from_file(&mut flox.catalog_client, "resolve/go.json");
        assert_build_status(&flox, &mut env, &package_name, true);
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

    #[test]
    fn build_impure_against_libc() {
        let package_name = String::from("foo");
        let bin_name = String::from("links-against-libc");
        let source_name = String::from("main.go");

        let (mut flox, _temp_dir_handle) = flox_instance();
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

        reset_mocks_from_file(&mut flox.catalog_client, "envs/go_gcc.json");
        assert_build_status(&flox, &mut env, &package_name, true);

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

        assert_build_status(&flox, &mut env, &package_name, true);

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

        assert_build_status(&flox, &mut env, &package_name, true);

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

        assert_build_status(&flox, &mut env, &package_foo, true);
        assert_build_status(&flox, &mut env, &package_bar, true);

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

        let output = temp_env::with_var("_FLOX_PKGDB_VERBOSITY", Some("1"), || {
            assert_build_status(&flox, &mut env, &package_name, false)
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

        let output = temp_env::with_var("_FLOX_PKGDB_VERBOSITY", Some("1"), || {
            assert_build_status(&flox, &mut env, &package_name, succeed)
        });

        let build_script_path_message_regex =
            regex::Regex::new(r#"bash -e (.+-build.bash)|--argstr buildScript "(.+build.bash)""#)
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
        let stdout = Command::new("nix")
            .args(["derivation", "show", store_path.to_str().unwrap()])
            .output()
            .unwrap()
            .stdout;
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

    #[test]
    fn build_version_propagated() {
        let pname = "foo".to_string();
        let version = "4.2.0".to_string();

        for sandbox_mode in ["off", "pure"].iter() {
            let manifest = formatdoc! {r#"
                version = 1

                [build.{pname}]
                sandbox = "{sandbox_mode}"
                command = """
                    echo "foo" > $out
                """
                version = "{version}"
            "#};
            let (flox, _temp_dir_handle) = flox_instance();
            let mut env = new_path_environment(&flox, &manifest);
            let env_path = env.parent_path().unwrap();
            let _git = GitCommandProvider::init(&env_path, false).unwrap();
            let collected = assert_build_status(&flox, &mut env, &pname, true);
            let result_path = env_path.join(format!("result-{pname}"));
            let build_results = collected.build_results.unwrap();
            assert_eq!(build_results.len(), 1);
            assert_eq!(build_results[0].version, version);
            let realpath = std::fs::read_link(&result_path).unwrap();
            assert_derivation_metadata_propagated(&["env", "version"], &version, &realpath);
        }
    }
}
