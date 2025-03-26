#![allow(dead_code)]
#![allow(unused_imports)]
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::{Duration, Instant};

pub mod proptest;

use anyhow::{Context, anyhow, bail};
// use flox_core::proc_status::{pid_is_running, pid_with_var};
use indoc::formatdoc;
use nix::sys::signal::Signal::{SIGKILL, SIGTERM};
use nix::unistd::Pid;
// use rexpect::session::{spawn_specific_bash, PtyReplSession};
use tempfile::TempDir;

pub mod manifests;

type Error = anyhow::Error;

// Modifications to `rexpect`:
// - The built-in reader used by `rexpect` has a hard-coded sleep interval of 100ms,
//   so a command that's even 1ms later than the `wait_for_prompt` call will take
//   100ms even though it's complete much earlier.
// - You need to make sure that the shells used by `rexpect` don't load rc files,
//   otherwise they'll pick up Nix-generated ones (e.g. home-manager) first and wreck
//   the prompt.
// - I changed the read sleep interval from 1000ms to 5ms, otherwise waiting for the
//   prompt always takes 100ms.
// - I added a new function that allows you specify which shell to use in `spawn_bash`.
// - I disabled "bracketed paste mode", which was also breaking `wait_for_prompt`.

// Approaches for test failure on leaked process:
// - During drop you can wait to see if the process terminates with a timeout.
//   If the timeout fails you can panic (if you aren't already panicking). This will
//   fail the test even if it's in the process of cleaning up an otherwise successful
//   test (which is what you want).
// - This depends on drop order though. If you haven't explicitly called `exit`, and
//   you haven't dropped the shell yet, there's no reason for the background process
//   to exit yet, and you'll panic even though everything would have been cleaned up
//   properly had the drop order been different.

// Nifty things:
// - By storing a reference to the isolated home directory, you can ensure that the
//   directories live as long or longer than the watchdog and process-compose, which
//   is the cause of some weird test failures (at cleanup time) in `bats`.
// - By using `tempfile` we ensure that any temporary files we create are cleaned up
//   when the test completes.
// - `bats` creates a file for each test by concatenating the setup/body/teardown
//   scripts, so you get a new tempfile for every test in the suite, every time you
//   run it. Since this is a compiled artifact, you get one artifact for the entire
//   suite.

// Performance:
// - Spawning a shell takes about 60ms
// - `flox init` takes about 30ms
// - Activating an empty environment takes 200-300ms
// - By the time you've created dirs, done `flox init`, and activated, you're
//   sitting pretty consistently around 350ms

/// A collection of temporary directories to be used as an isolated home directory
#[derive(Debug)]
pub struct IsolatedHome {
    _home_temp_dir: TempDir,
    home_dir: PathBuf,
    state_dir: PathBuf,
    cache_dir: PathBuf,
    data_dir: PathBuf,
    config_dir: PathBuf,
    envs: HashMap<String, String>,
}

impl IsolatedHome {
    /// Returns true if the home dir and all of the XDG dirs that were originally created
    /// still exist.
    pub fn all_dirs_exist(&self) -> bool {
        self.home_dir.exists()
            && self.state_dir.exists()
            && self.cache_dir.exists()
            && self.data_dir.exists()
            && self.config_dir.exists()
    }
}

impl std::fmt::Display for IsolatedHome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "IsolatedHome {{ ")?;
        write!(
            f,
            "home_dir: {} (exists: {}), ",
            self.home_dir.display(),
            self.home_dir.exists()
        )?;
        write!(
            f,
            "state_dir: {} (exists: {}), ",
            self.state_dir.display(),
            self.state_dir.exists()
        )?;
        write!(
            f,
            "data_dir: {} (exists: {}), ",
            self.data_dir.display(),
            self.data_dir.exists()
        )?;
        write!(
            f,
            "config_dir: {} (exists: {}), ",
            self.config_dir.display(),
            self.config_dir.exists()
        )?;
        write!(
            f,
            "cache_dir: {} (exists: {}), ",
            self.cache_dir.display(),
            self.cache_dir.exists()
        )?;
        write!(f, " }}")?;

        Ok(())
    }
}

impl IsolatedHome {
    pub fn new() -> Result<Self, Error> {
        let home_tmp =
            tempfile::TempDir::new().context("failed to create temporary home directory")?;
        let home_dir = home_tmp.path().to_path_buf();
        let data_dir = home_tmp.path().join(".local/share");
        let state_dir = home_tmp.path().join(".local/state");
        let config_dir = home_tmp.path().join(".config");
        let cache_dir = home_tmp.path().join(".cache");

        std::fs::create_dir_all(&data_dir).context("failed to create temp data directory")?;
        std::fs::create_dir_all(&state_dir).context("failed to create temp state directory")?;
        std::fs::create_dir_all(&config_dir).context("failed to create temp config directory")?;
        std::fs::create_dir_all(&cache_dir).context("failed to create temp cache directory")?;

        // Don't want to accidentally get prompted to enable/disable metrics
        std::fs::create_dir(config_dir.join("flox")).context("failed to create flox config dir")?;
        std::fs::write(config_dir.join("flox/flox.toml"), "disable_metrics = true")
            .context("failed to write flox config file")?;

        // NOTE: This turned out to make no difference, so it's either not working properly
        //       or the bottleneck isn't Nix evaluation.
        // Symlink the host's eval cache into this set of directories to speed up tests
        if let Some(path) = dirs::cache_dir().map(|p| p.join("nix")) {
            let _ = std::os::unix::fs::symlink(path, cache_dir.join("nix"));
        }

        // Create the environment variables that will point to these
        // temporary directories
        let mut envs = HashMap::new();
        envs.insert(String::from("HOME"), home_dir.to_string_lossy().to_string());
        envs.insert(
            String::from("XDG_DATA_HOME"),
            data_dir.to_string_lossy().to_string(),
        );
        envs.insert(
            String::from("XDG_STATE_HOME"),
            state_dir.to_string_lossy().to_string(),
        );
        envs.insert(
            String::from("XDG_CONFIG_HOME"),
            config_dir.to_string_lossy().to_string(),
        );
        envs.insert(
            String::from("XDG_CACHE_HOME"),
            cache_dir.to_string_lossy().to_string(),
        );
        // TODO: this doesn't belong here, put it in the shell config
        envs.insert(String::from("NO_COLOR"), String::from("1"));

        Ok(Self {
            _home_temp_dir: home_tmp,
            home_dir,
            data_dir,
            state_dir,
            config_dir,
            cache_dir,
            envs,
        })
    }
}

// /// A Bash shell connected to a PTY and a set of temporary directories.
// pub struct ShellProcess<'dirs> {
//     pty: PtyReplSession,
//     /// This being borrowed ensures that the directories don't get destroyed while
//     /// the shell is still running.
//     dirs: &'dirs IsolatedHome,
// }

// impl std::fmt::Debug for ShellProcess<'_> {
//     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//         <IsolatedHome as std::fmt::Debug>::fmt(&self.dirs, f)
//     }
// }

// impl Deref for ShellProcess<'_> {
//     type Target = PtyReplSession;

//     fn deref(&self) -> &Self::Target {
//         &self.pty
//     }
// }

// impl DerefMut for ShellProcess<'_> {
//     fn deref_mut(&mut self) -> &mut Self::Target {
//         &mut self.pty
//     }
// }

// impl<'dirs> ShellProcess<'dirs> {
//     pub fn spawn(dirs: &'dirs IsolatedHome, timeout_millis: Option<u64>) -> Result<Self, Error> {
//         // TODO: change this to FLOX_TEST_SHELL_BASH
//         let test_shell = std::env::var("FLOX_SHELL_BASH")?;
//         let mut shell =
//             spawn_specific_bash(&test_shell, timeout_millis).context("failed to spawn bash")?;
//         for (var, value) in dirs.envs.iter() {
//             shell
//                 .send_line(&format!(r#"export {var}="{value}""#))
//                 .with_context(|| format!("failed to set environment variable: {var}"))?;
//             shell.wait_for_prompt().context("prompt never appeared")?;
//         }
//         shell
//             .send_line("export _FLOX_NO_PROMPT=1")
//             .context("failed to set no-prompt var")?;
//         shell.wait_for_prompt().context("prompt never appeared")?;
//         shell
//             .send_line(&format!("export FLOX_SHELL={}", test_shell))
//             .context("failed to set FLOX_SHELL")?;
//         shell.wait_for_prompt().context("prompt never appeared")?;

//         // I don't know why this is necessary
//         shell.execute("echo howdy", "howdy")?;

//         // Double check that the environment variables were set.
//         shell
//             .send_line(r#"echo "$HOME""#)
//             .context("failed to send line")?;
//         shell.wait_for_prompt().context("prompt never appeared")?;
//         let output = shell.read_line().context("failed to get line")?;
//         let string_home_dir = dirs.home_dir.to_string_lossy().to_string();
//         if output != string_home_dir {
//             bail!(
//                 "setting vars failed: expected $HOME to be '{}' but found '{:?}'",
//                 string_home_dir,
//                 output
//             );
//         }

//         // `cd` into the new home directory
//         shell
//             .send_line(r#"cd "$HOME""#)
//             .context("failed to cd to temp home")?;
//         shell.wait_for_prompt().context("cd HOME failed")?;

//         Ok(Self { pty: shell, dirs })
//     }

//     pub fn pid(&self) -> i32 {
//         self.pty_session.process.child_pid.as_raw()
//     }

//     pub fn reconfigure_prompt(&mut self) -> Result<(), Error> {
//         // This last command is to turn off whatever bracketed paste mode is about
//         self.pty.send_line(
//             r#"PS1="[REXPECT_PROMPT>" && unset PROMPT_COMMAND && bind 'set enable-bracketed-paste off'"#,
//         )?;
//         Ok(())
//     }

//     /// Send a string, adding a newline.
//     pub fn send_line(&mut self, line: impl AsRef<str>) -> Result<(), Error> {
//         let _ = self
//             .pty
//             .send_line(line.as_ref())
//             .context("failed to send line to pty")?;
//         Ok(())
//     }

//     /// Creates `$HOME/<name>`, `cd`s into the directory, then does a `flox init`.
//     pub fn init_env_with_name(&mut self, name: impl AsRef<str>) -> Result<(), Error> {
//         self.pty
//             .send_line(&format!(r#"mkdir "{}""#, name.as_ref()))
//             .context("failed to send mkdir command")?;
//         self.pty
//             .wait_for_prompt()
//             .context("prompt never appeared")?;
//         self.pty
//             .send_line(&format!(r#"cd "{}""#, name.as_ref()))
//             .context("failed to send cd command")?;
//         self.pty
//             .wait_for_prompt()
//             .context("prompt never appeared")?;
//         self.pty
//             .send_line("flox init")
//             .with_context(|| format!("failed to cd into directory: {}", name.as_ref()))?;
//         self.pty
//             .exp_string(&format!("Created environment '{}'", name.as_ref()))?;
//         self.pty
//             .wait_for_prompt()
//             .context("prompt never appeared")?;
//         Ok(())
//     }

//     /// Does a `flox edit -f` with a file containing the provided string.
//     ///
//     /// This will fail if the shell can't find an environment to edit.
//     pub fn edit_with_manifest_contents(
//         &mut self,
//         manifest_contents: impl AsRef<str>,
//     ) -> Result<(), Error> {
//         let file =
//             tempfile::NamedTempFile::new().context("failed to create temporary manifest file")?;
//         std::fs::write(file.path(), manifest_contents.as_ref())
//             .context("failed to write temporary manifest")?;
//         let cmd = format!("flox edit -f {}", file.path().display());
//         let res = self
//             .pty
//             .send_line(&cmd)
//             .context("failed to run 'flox edit -f'");
//         self.pty
//             .exp_string("successfully updated")
//             .context("edit was unsuccessful")?;
//         self.pty.wait_for_prompt().context("never got prompt")?;
//         // Remove the tempfile first so it isn't left laying around if there's an error
//         std::fs::remove_file(file.path()).context("failed to delete temp manifest")?;
//         let _ = res?;
//         Ok(())
//     }

//     /// Copies one of the manifests/lockfile generated from the catalog client
//     /// and uses it to initialize an environment.
//     ///
//     /// `copy_from`: The directory containing the `manifest.toml` and `manifest.lock`
//     pub fn init_from_generated_env(
//         &mut self,
//         name: impl AsRef<str>,
//         copy_from: impl AsRef<Path>,
//     ) -> Result<(), Error> {
//         // Create the .flox directory
//         let env_dir = self.dirs.home_dir.join(name.as_ref()).join(".flox/env");
//         std::fs::create_dir_all(&env_dir).context("couldn't create .flox/env")?;

//         // Copy the manifest and set permissions
//         let manifest_src_path = copy_from.as_ref().join("manifest.toml");
//         let manifest_dest_path = env_dir.join("manifest.toml");
//         std::fs::copy(&manifest_src_path, &manifest_dest_path)
//             .context("failed to copy manifest")?;
//         let mut manifest_perms = std::fs::metadata(&manifest_dest_path)
//             .context("failed to get file metadata for manifest")?
//             .permissions();
//         manifest_perms.set_readonly(false);
//         std::fs::set_permissions(&manifest_dest_path, manifest_perms)
//             .context("failed to set manifest permissions")?;

//         // Copy the lockfile and set permissions
//         let lockfile_src_path = copy_from.as_ref().join("manifest.lock");
//         let lockfile_dest_path = env_dir.join("manifest.lock");
//         std::fs::copy(&lockfile_src_path, &lockfile_dest_path)
//             .context("failed to copy lockfile")?;
//         let mut lockfile_perms = std::fs::metadata(&lockfile_dest_path)
//             .context("failed to get file metadata for lockfile")?
//             .permissions();
//         lockfile_perms.set_readonly(false);
//         std::fs::set_permissions(&lockfile_dest_path, lockfile_perms)
//             .context("failed to set lockfile permissions")?;

//         // Create a `.flox/env.json` file since those aren't stored in the
//         // generated data
//         let contents = formatdoc! {r#"
//             {{
//                 "version": 1,
//                 "name": "{}"
//             }}
//         "#, name.as_ref()};
//         std::fs::write(
//             env_dir.parent().unwrap().join("env.json"),
//             contents.as_bytes(),
//         )
//         .context("failed to write env.json")?;

//         self.pty
//             .send_line(&format!(r#"cd "{}""#, name.as_ref()))
//             .context("failed to send cd command")?;
//         self.pty
//             .wait_for_prompt()
//             .context("prompt never appeared")?;

//         Ok(())
//     }

//     /// Activates the environment in the current directory
//     pub fn activate(&mut self, args: &[&str]) -> Result<(), Error> {
//         let cmd = Self::make_activation_command(args, true);
//         self.pty.send_line(&cmd)?;
//         self.pty.exp_string("bash-5.2$")?;
//         self.reconfigure_prompt()?;
//         self.pty.wait_for_prompt()?;
//         Ok(())
//     }

//     /// Activates the environment in the current directory
//     pub fn activate_with_unchecked_args(&mut self, args: &[&str]) -> Result<(), Error> {
//         let cmd = Self::make_activation_command(args, false);
//         self.pty.send_line(&cmd)?;
//         self.pty.exp_string("bash-5.2$")?;
//         self.reconfigure_prompt()?;
//         self.pty.wait_for_prompt()?;
//         Ok(())
//     }

//     /// Performs an activation and returns handles to the watchdog and process-compose
//     pub fn activate_with_services(&mut self, args: &[&str]) -> Result<(ProcToGC, ProcToGC), Error> {
//         // TODO: remove the timing prints
//         let start = start_timer();
//         let mut all_args = vec!["--start-services"];
//         all_args.extend_from_slice(args);
//         let cmd = Self::make_activation_command(&all_args, false);
//         print_elapsed_with_prefix(start, "activate", "make cmd");
//         self.pty.send_line(&cmd)?;
//         print_elapsed_with_prefix(start, "activate", "send activation cmd");
//         self.pty.exp_string("bash-5.2$")?;
//         print_elapsed_with_prefix(start, "activate", "wait for default prompt");
//         self.reconfigure_prompt()?;
//         print_elapsed_with_prefix(start, "activate", "reconfigure prompt");
//         self.pty.wait_for_prompt()?;
//         print_elapsed_with_prefix(start, "activate", "wait for prompt");
//         let uuid = self.read_activation_uuid()?;
//         print_elapsed_with_prefix(start, "activate", "read activation uuid");
//         let watchdog =
//             watchdog_with_uuid(&uuid).context("activation with services didn't spawn watchdog")?;
//         print_elapsed_with_prefix(start, "activate", "get watchdog");
//         let process_compose = process_compose_with_uuid(&uuid)
//             .context("activation with services didn't spawn process-compose")?;
//         print_elapsed_with_prefix(start, "activate", "get process-compose");
//         Ok((watchdog, process_compose))
//     }

//     /// Constructs the `flox activate` command from the provided arguments
//     fn make_activation_command(
//         args_without_flox_activate: &[&str],
//         check_service_arg: bool,
//     ) -> String {
//         let mut buf = String::from("flox activate");
//         for arg in args_without_flox_activate.iter() {
//             if check_service_arg {
//                 if (*arg == "-s") || (*arg == "--start-services") {
//                     // This ensures we always get handles to the processes
//                     // we want to GC at the end of a test
//                     panic!("use ShellProcess::activate_with_services to activate with services");
//                 }
//             }
//             buf.push_str(" ");
//             buf.push_str(arg);
//         }
//         buf
//     }

//     /// Reads the _FLOX_ACTIVATION_UUID value from an activated shell
//     pub fn read_activation_uuid(&mut self) -> Result<String, Error> {
//         self.pty
//             .send_line("echo $_FLOX_ACTIVATION_UUID")
//             .context("failed to send command")?;
//         sleep(Duration::from_millis(100));
//         let value = self.pty.read_line().context("failed to read line")?;
//         self.pty
//             .wait_for_prompt()
//             .context("prompt never appeared")?;
//         Ok(value)
//     }

//     /// Exports an environment variable
//     pub fn set_var(&mut self, name: impl AsRef<str>, value: impl AsRef<str>) -> Result<(), Error> {
//         self.pty
//             .send_line(&format!(r#"export {}="{}""#, name.as_ref(), value.as_ref()))?;
//         self.pty.wait_for_prompt()?;
//         Ok(())
//     }

//     /// Returns an error if the shell is not inside an activation.
//     pub fn assert_is_activated(&mut self) -> Result<(), Error> {
//         self.pty.send_line(r#"echo $FLOX_ENV_DIRS"#)?;
//         let output = self.wait_for_prompt()?;
//         let trimmed = output.as_str().trim();
//         if trimmed.is_empty() {
//             bail!("assert activated failed");
//         }
//         Ok(())
//     }

//     /// Returns whether the previous command succeeded
//     pub fn succeeded(&mut self) -> Result<bool, Error> {
//         self.pty.send_line("echo $?")?;
//         let output = self.wait_for_prompt()?;
//         // `output` contains the trailing newline and carriage return
//         match output.as_str().trim() {
//             "0" => Ok(true),
//             "1" => Ok(false),
//             _ => Err(anyhow!(
//                 "unexpected output while checking status: {:?}",
//                 output
//             )),
//         }
//     }

//     /// Throws an error if the previous command didn't succeed
//     pub fn assert_success(&mut self) -> Result<(), Error> {
//         if self.succeeded().is_ok_and(|value| value) {
//             return Ok(());
//         }
//         bail!("previous command failed");
//     }

//     pub fn exit_shell(&mut self) {
//         self.pty.send_line("exit").unwrap();
//         self.pty.wait_for_prompt().unwrap();
//     }
// }

// /// Locates the watchdog fingerprinted with the provided UUID
// pub fn find_watchdog_pid_with_uuid(uuid: impl AsRef<str>) -> Option<u32> {
//     pid_with_var("flox-watchdog", "_FLOX_ACTIVATION_UUID", uuid)
//         .unwrap_or_default()
//         .map(|pid_i32| pid_i32 as u32)
// }

// /// Locates the watchdog fingerprinted with the provided UUID
// pub fn find_process_compose_pid_with_uuid(uuid: impl AsRef<str>) -> Option<u32> {
//     pid_with_var("process-compose", "_FLOX_ACTIVATION_UUID", uuid)
//         .unwrap_or_default()
//         .map(|pid_i32| pid_i32 as u32)
// }

// fn watchdog_with_uuid(uuid: impl AsRef<str>) -> Option<ProcToGC> {
//     find_watchdog_pid_with_uuid(uuid).map(|pid| ProcToGC::new_with_pid(pid))
// }

// pub fn process_compose_with_uuid(uuid: impl AsRef<str>) -> Option<ProcToGC> {
//     find_process_compose_pid_with_uuid(uuid).map(|pid| ProcToGC::new_with_pid(pid))
// }

// #[derive(Debug)]
// pub struct ProcToGC {
//     /// Whether this process has already terminated. Used to short-circuit
//     /// waiting for termination or looking up process status.
//     is_terminated: bool,
//     /// The PID of the process.
//     pub pid: u32,
//     /// How long to wait for the process to terminate while it's being dropped.
//     drop_timeout_millis: u64,
// }

// impl ProcToGC {
//     pub fn new_with_pid(pid: u32) -> Self {
//         Self {
//             is_terminated: false,
//             pid,
//             drop_timeout_millis: 1000,
//         }
//     }

//     pub fn is_running(&mut self) -> bool {
//         if self.is_terminated {
//             return false;
//         }
//         pid_is_running(self.pid as i32)
//     }

//     pub fn set_drop_timeout(&mut self, millis: u64) {
//         self.drop_timeout_millis = millis;
//     }

//     pub fn send_sigterm(&mut self) {
//         if self.is_terminated {
//             return;
//         }
//         if self.is_running() {
//             nix::sys::signal::kill(Pid::from_raw(self.pid as i32), SIGTERM)
//                 .expect("failed to deliver signal");
//         } else {
//             self.is_terminated = true;
//         }
//     }

//     pub fn send_sigkill(&mut self) {
//         if self.is_terminated {
//             return;
//         }
//         if self.is_running() {
//             nix::sys::signal::kill(Pid::from_raw(self.pid as i32), SIGKILL)
//                 .expect("failed to deliver signal");
//         } else {
//             self.is_terminated = true;
//         }
//     }

//     pub fn wait_for_termination_with_timeout(&mut self, millis: u64) -> Result<(), Error> {
//         let mut remaining = millis;
//         let interval = 25;
//         let mut next_sleep = interval.min(remaining);
//         loop {
//             if !self.is_running() {
//                 self.is_terminated = true;
//                 return Ok(());
//             }
//             if remaining == 0 {
//                 bail!("timed out waiting for termination");
//             }
//             sleep(Duration::from_millis(next_sleep));
//             if let Some(new_remaining) = remaining.checked_sub(interval) {
//                 next_sleep = interval;
//                 remaining = new_remaining;
//             } else {
//                 next_sleep = remaining;
//                 remaining = 0;
//             }
//         }
//     }
// }

// impl Drop for ProcToGC {
//     fn drop(&mut self) {
//         use std::thread::panicking;

//         // No need to wait if it's already terminated
//         if self.is_terminated {
//             return;
//         }

//         // A panic inside of a panic will cause an immediate abort,
//         // check if we're already panicking.
//         if panicking() {
//             self.send_sigterm();
//         } else {
//             if self
//                 .wait_for_termination_with_timeout(self.drop_timeout_millis)
//                 .is_err()
//             {
//                 self.send_sigterm();
//                 panic!("background process was leaked");
//             }
//         }
//     }
// }

#[allow(dead_code)]
fn start_timer() -> Instant {
    // eprintln!("starting clock");
    Instant::now()
}

#[allow(dead_code)]
fn print_elapsed(start: Instant, msg: &str) {
    eprintln!(
        "elapsed: {} ({msg})",
        Instant::now().duration_since(start).as_millis()
    );
}

#[allow(dead_code)]
fn print_elapsed_with_prefix(start: Instant, prefix: &str, msg: &str) {
    eprintln!(
        "[{prefix}] elapsed: {} ({msg})",
        Instant::now().duration_since(start).as_millis()
    );
}

/// Returns the path to a `mkdata`-generated environment
#[allow(dead_code)]
fn path_to_generated_env(name: &str) -> PathBuf {
    let base_dir = PathBuf::from(std::env::var("GENERATED_DATA").unwrap());
    base_dir.join("envs").join(name)
}

// #[cfg(test)]
// mod tests {
//     use indoc::formatdoc;
//     use serde_json::Value;

//     use super::*;

//     // Nothing should hit this in normal operation,
//     // it's only there if you need to build an environment for
//     // the first time on the host machine.
//     const DEFAULT_EXPECT_TIMEOUT: u64 = 30_000;

//     // Just a helper function for less typing
//     #[allow(dead_code)]
//     fn sleep_millis(millis: u64) {
//         sleep(Duration::from_millis(millis));
//     }

//     #[test]
//     fn can_construct_shell() {
//         let dirs = IsolatedHome::new().unwrap();
//         let _shell = ShellProcess::spawn(&dirs, Some(DEFAULT_EXPECT_TIMEOUT)).unwrap();
//     }

//     #[test]
//     fn can_activate() {
//         let dirs = IsolatedHome::new().unwrap();
//         let mut shell = ShellProcess::spawn(&dirs, Some(DEFAULT_EXPECT_TIMEOUT)).unwrap();
//         // shell.init_env_with_name("myenv").unwrap();
//         shell
//             .init_from_generated_env("myenv", path_to_generated_env("hello"))
//             .unwrap();
//         shell.activate(&[]).unwrap();
//         shell.send_line(r#"echo "$_activate_d""#).unwrap();
//         shell.exit_shell(); // once for the activation
//     }

//     #[test]
//     fn update_env_with_manifest() {
//         let dirs = IsolatedHome::new().unwrap();
//         let mut shell = ShellProcess::spawn(&dirs, Some(DEFAULT_EXPECT_TIMEOUT)).unwrap();
//         shell.init_env_with_name("myenv").unwrap();
//         shell
//             .edit_with_manifest_contents(formatdoc! {r#"
//             version = 1

//             [hook]
//             on-activate = '''
//                 echo howdy
//             '''

//             [options]
//             systems = ["aarch64-darwin", "x86_64-darwin", "aarch64-linux", "x86_64-linux"]
//         "#})
//             .unwrap();
//         let manifest_contents =
//             std::fs::read_to_string(shell.dirs.home_dir.join("myenv/.flox/env/manifest.toml"))
//                 .unwrap();
//         assert!(manifest_contents.find("howdy").is_some());
//         shell.send_line("flox activate").unwrap();
//         shell.exp_string("howdy").unwrap();
//         shell.exit_shell(); // once for the activation
//     }

//     #[test]
//     fn read_activation_uuid() {
//         let dirs = IsolatedHome::new().unwrap();
//         let mut shell = ShellProcess::spawn(&dirs, Some(DEFAULT_EXPECT_TIMEOUT)).unwrap();
//         shell
//             .init_from_generated_env("myenv", path_to_generated_env("empty"))
//             .unwrap();
//         shell.activate(&[]).unwrap();
//         shell
//             .execute(
//                 "echo $_FLOX_ACTIVATION_UUID",
//                 r#"\w{8}-\w{4}-\w{4}-\w{4}-\w{12}"#,
//             )
//             .unwrap();
//         shell.exit_shell(); // once for the activation
//     }

//     #[test]
//     fn can_locate_watchdog() {
//         let dirs = IsolatedHome::new().unwrap();
//         let mut shell = ShellProcess::spawn(&dirs, Some(DEFAULT_EXPECT_TIMEOUT)).unwrap();
//         shell
//             .init_from_generated_env("myenv", path_to_generated_env("empty"))
//             .unwrap();
//         shell.activate(&[]).unwrap();
//         let uuid = shell.read_activation_uuid().unwrap();
//         let watchdog = find_watchdog_pid_with_uuid(uuid);
//         assert!(watchdog.is_some());
//         shell.exit_shell(); // once for the activation
//     }

//     #[test]
//     fn can_locate_process_compose() {
//         let dirs = IsolatedHome::new().unwrap();
//         let mut shell = ShellProcess::spawn(&dirs, Some(DEFAULT_EXPECT_TIMEOUT)).unwrap();
//         shell
//             .init_from_generated_env("myenv", path_to_generated_env("sleeping_services"))
//             .unwrap();
//         shell
//             .activate_with_unchecked_args(&["--start-services"])
//             .unwrap();
//         let uuid = shell.read_activation_uuid().unwrap();
//         let process_compose = find_process_compose_pid_with_uuid(uuid);
//         assert!(process_compose.is_some());
//         shell.exit_shell(); // once for the activation
//     }

//     #[test]
//     fn cleans_up_watchdog() {
//         let dirs = IsolatedHome::new().unwrap();
//         let mut shell = ShellProcess::spawn(&dirs, Some(DEFAULT_EXPECT_TIMEOUT)).unwrap();
//         shell
//             .init_from_generated_env("myenv", path_to_generated_env("sleeping_services"))
//             .unwrap();
//         shell.activate(&[]).unwrap();
//         let uuid = shell.read_activation_uuid().unwrap();
//         let watchdog_proc = watchdog_with_uuid(uuid);
//         assert!(watchdog_proc.is_some());
//         let Some(mut watchdog_proc) = watchdog_proc else {
//             panic!("we literally just checked that it was Some(_)");
//         };
//         assert!(watchdog_proc.is_running());
//         shell.exit_shell(); // once for the activation
//         watchdog_proc
//             .wait_for_termination_with_timeout(1000)
//             .unwrap();
//     }

//     #[test]
//     fn cleans_up_process_compose() {
//         let dirs = IsolatedHome::new().unwrap();
//         let mut shell = ShellProcess::spawn(&dirs, Some(DEFAULT_EXPECT_TIMEOUT)).unwrap();
//         shell
//             .init_from_generated_env("myenv", path_to_generated_env("sleeping_services"))
//             .unwrap();
//         let (_watchdog, mut process_compose_proc) = shell.activate_with_services(&[]).unwrap();
//         shell.assert_is_activated().unwrap();
//         assert!(process_compose_proc.is_running());
//         shell.exit_shell(); // once for the activation
//         process_compose_proc
//             .wait_for_termination_with_timeout(1000)
//             .unwrap();
//     }

//     #[test]
//     fn background_procs_exit_cleanly() {
//         let dirs = IsolatedHome::new().unwrap();
//         let mut shell = ShellProcess::spawn(&dirs, Some(DEFAULT_EXPECT_TIMEOUT)).unwrap();
//         shell
//             .init_from_generated_env("myenv", path_to_generated_env("sleeping_services"))
//             .unwrap();
//         let (mut watchdog, mut process_compose) = shell.activate_with_services(&[]).unwrap();
//         shell.assert_is_activated().unwrap();
//         shell.exit_shell(); // once for the activation

//         watchdog.wait_for_termination_with_timeout(1000).unwrap();
//         process_compose
//             .wait_for_termination_with_timeout(1000)
//             .unwrap();
//     }

//     #[test]
//     fn detects_leaked_process() {
//         let dirs = IsolatedHome::new().unwrap();
//         let mut shell = ShellProcess::spawn(&dirs, Some(DEFAULT_EXPECT_TIMEOUT)).unwrap();
//         shell
//             .init_from_generated_env("myenv", path_to_generated_env("sleeping_services"))
//             .unwrap();
//         let (mut watchdog, mut process_compose) = shell.activate_with_services(&[]).unwrap();
//         watchdog.send_sigkill(); // kill this first so it doesn't kill process-compose
//         watchdog.wait_for_termination_with_timeout(1000).unwrap();
//         shell.exit_shell();
//         let timed_out = process_compose
//             .wait_for_termination_with_timeout(250)
//             .is_err();
//         process_compose.send_sigterm(); // nothing else is going to clean it up
//         if !timed_out {
//             panic!("process-compose terminated early");
//         }
//     }

//     #[test]
//     #[should_panic]
//     fn automatically_fails_on_leaked_process() {
//         let dirs = IsolatedHome::new().unwrap();
//         let mut shell = ShellProcess::spawn(&dirs, Some(DEFAULT_EXPECT_TIMEOUT)).unwrap();
//         shell
//             .init_from_generated_env("myenv", path_to_generated_env("sleeping_services"))
//             .unwrap();
//         let (mut watchdog, mut process_compose) = shell.activate_with_services(&[]).unwrap();
//         shell.exit_shell(); // once for the activation
//         watchdog.send_sigkill();
//         // The Drop impl for the process-compose struct should cause
//         // a panic because nothing is cleaning up the process. We set
//         // a short drop timeout because we expect it to time out and don't
//         // need to wait an unnecessarily long time.
//         process_compose.set_drop_timeout(100);
//     }

//     ////////////////////////////////////////////////////////////////////////////
//     // These are the actual service tests
//     ////////////////////////////////////////////////////////////////////////////

//     #[test]
//     fn services_arent_started_unless_requested() {
//         let dirs = IsolatedHome::new().unwrap();
//         let mut shell = ShellProcess::spawn(&dirs, Some(DEFAULT_EXPECT_TIMEOUT)).unwrap();
//         shell
//             .init_from_generated_env("myenv", path_to_generated_env("sleeping_services"))
//             .unwrap();
//         shell.activate(&[]).unwrap();
//         // If services _were_ going to start, give them a chance to do so
//         sleep_millis(100);
//         let process_compose = process_compose_with_uuid(shell.read_activation_uuid().unwrap());
//         assert!(process_compose.is_none());
//         shell.exit_shell(); // once for the activation
//     }

//     #[test]
//     fn imperative_commands_error_when_no_services_defined() {
//         let dirs = IsolatedHome::new().unwrap();
//         let mut shell = ShellProcess::spawn(&dirs, Some(DEFAULT_EXPECT_TIMEOUT)).unwrap();
//         shell
//             .init_from_generated_env("myenv", path_to_generated_env("empty"))
//             .unwrap();
//         shell.activate(&[]).unwrap();
//         shell.send_line("flox services start").unwrap();
//         shell.wait_for_prompt().unwrap();
//         assert!(shell.succeeded().is_ok_and(|r| r == false));
//         shell.exit_shell(); // once for the activation
//     }

//     #[test]
//     fn warns_about_restarting_services() {
//         let start = start_timer();
//         let dirs = IsolatedHome::new().unwrap();
//         let mut shell = ShellProcess::spawn(&dirs, Some(DEFAULT_EXPECT_TIMEOUT)).unwrap();
//         print_elapsed(start, "spawn shell");
//         shell
//             .init_from_generated_env("myenv", path_to_generated_env("sleeping_services"))
//             .unwrap();
//         print_elapsed(start, "init env");
//         let (_w, _pc) = shell.activate_with_services(&[]).unwrap();
//         print_elapsed(start, "activated");
//         shell
//             .set_var(
//                 "_FLOX_USE_CATALOG_MOCK",
//                 "$GENERATED_DATA/resolve/hello.json",
//             )
//             .unwrap();
//         print_elapsed(start, "set var");
//         shell.send_line("flox install hello").unwrap();
//         print_elapsed(start, "send line");
//         shell.exp_string("flox services restart").unwrap();
//         print_elapsed(start, "wait for string");
//         shell.wait_for_prompt().unwrap();
//         print_elapsed(start, "wait for prompt");
//         shell.exit_shell(); // once for the activation
//         print_elapsed(start, "exit shell");
//     }

//     #[test]
//     fn restart_fails_fast_on_invalid_service_name() {
//         let dirs = IsolatedHome::new().unwrap();
//         let mut shell = ShellProcess::spawn(&dirs, Some(DEFAULT_EXPECT_TIMEOUT)).unwrap();
//         shell
//             .init_from_generated_env("myenv", path_to_generated_env("sleeping_services"))
//             .unwrap();
//         let (_w, _pc) = shell.activate_with_services(&[]).unwrap();
//         shell.send_line("flox services status --json").unwrap();
//         let output = shell.wait_for_prompt().unwrap();
//         let mut service_pids = vec![];
//         for line in output.lines() {
//             let json: Value = serde_json::from_str(line).unwrap();
//             let pid: u32 = json.get("pid").unwrap().as_u64().unwrap() as u32;
//             service_pids.push(pid);
//         }
//         shell
//             .send_line("flox services restart sleeper_1 sleeper_2 invalid")
//             .unwrap();
//         shell
//             .exp_string("Service 'invalid' does not exist")
//             .unwrap();
//         // Assert that the previous processes are still running
//         for pid in service_pids.into_iter() {
//             assert!(pid_is_running(pid as i32));
//         }
//         shell.exit_shell(); // once for the activation
//     }

//     #[test]
//     fn attach_doesnt_start_second_watchdog() {
//         let dirs = IsolatedHome::new().unwrap();
//         let mut shell1 = ShellProcess::spawn(&dirs, Some(DEFAULT_EXPECT_TIMEOUT)).unwrap();
//         shell1
//             .init_from_generated_env("myenv", path_to_generated_env("empty"))
//             .unwrap();
//         shell1.activate(&[]).unwrap();
//         let mut shell2 = ShellProcess::spawn(&dirs, Some(DEFAULT_EXPECT_TIMEOUT)).unwrap();
//         shell2.send_line("cd myenv").unwrap();
//         shell2.wait_for_prompt().unwrap();
//         shell2.activate(&[]).unwrap();
//         let uuid = shell2.read_activation_uuid().unwrap();
//         sleep_millis(50);
//         assert!(watchdog_with_uuid(&uuid).is_none());
//         shell1.exit_shell(); // once for the activation
//         shell2.exit_shell();
//     }
// }
