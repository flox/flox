use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread::sleep;
use std::time::Duration;

use anyhow::{bail, Context};
use rexpect::session::{spawn_specific_bash, PtyReplSession};
use sysinfo::{
    Pid,
    ProcessRefreshKind,
    ProcessStatus,
    ProcessesToUpdate,
    Signal,
    System,
    UpdateKind,
};
use tempfile::TempDir;

type Error = anyhow::Error;

// Notes:
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

/// A Bash shell connected to a PTY and a set of temporary directories.
pub struct ShellProcess<'dirs> {
    pty: PtyReplSession,
    /// This being borrowed ensures that the directories don't get destroyed while
    /// the shell is still running.
    dirs: &'dirs IsolatedHome,
}

impl std::fmt::Debug for ShellProcess<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        <IsolatedHome as std::fmt::Debug>::fmt(&self.dirs, f)
    }
}

impl Deref for ShellProcess<'_> {
    type Target = PtyReplSession;

    fn deref(&self) -> &Self::Target {
        &self.pty
    }
}

impl DerefMut for ShellProcess<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.pty
    }
}

impl<'dirs> ShellProcess<'dirs> {
    pub fn spawn(dirs: &'dirs IsolatedHome, timeout_millis: Option<u64>) -> Result<Self, Error> {
        let test_shell = std::env::var("FLOX_SHELL_BASH")?;
        let mut shell =
            spawn_specific_bash(&test_shell, timeout_millis).context("failed to spawn bash")?;
        for (var, value) in dirs.envs.iter() {
            shell
                .send_line(&format!(r#"export {var}="{value}""#))
                .with_context(|| format!("failed to set environment variable: {var}"))?;
            shell.wait_for_prompt().context("prompt never appeared")?;
        }
        shell
            .send_line("export _FLOX_NO_PROMPT=1")
            .context("failed to set no-prompt var")?;
        shell.wait_for_prompt().context("prompt never appeared")?;
        shell
            .send_line(&format!("export FLOX_SHELL={}", test_shell))
            .context("failed to set FLOX_SHELL")?;
        shell.wait_for_prompt().context("prompt never appeared")?;

        // I don't know why this is necessary
        shell.execute("echo howdy", "howdy")?;

        // Double check that the environment variables were set.
        shell
            .send_line(r#"echo "$HOME""#)
            .context("failed to send line")?;
        shell.wait_for_prompt().context("prompt never appeared")?;
        let output = shell.read_line().context("failed to get line")?;
        let string_home_dir = dirs.home_dir.to_string_lossy().to_string();
        if output != string_home_dir {
            bail!(
                "setting vars failed: expected $HOME to be '{}' but found '{:?}'",
                string_home_dir,
                output
            );
        }

        // `cd` into the new home directory
        shell
            .send_line(r#"cd "$HOME""#)
            .context("failed to cd to temp home")?;
        shell.wait_for_prompt().context("cd HOME failed")?;

        Ok(Self { pty: shell, dirs })
    }

    pub fn shell_reconfig(&mut self) -> Result<(), Error> {
        // This last command is to turn off whatever bracketed paste mode is about
        self.pty.send_line(
            r#"PS1="[REXPECT_PROMPT>" && unset PROMPT_COMMAND && bind 'set enable-bracketed-paste off'"#,
        )?;
        Ok(())
    }

    /// Send a string, adding a newline.
    pub fn send_line(&mut self, line: impl AsRef<str>) -> Result<(), Error> {
        let _ = self
            .pty
            .send_line(line.as_ref())
            .context("failed to send line to pty")?;
        Ok(())
    }

    /// Creates `$HOME/<name>`, `cd`s into the directory, then does a `flox init`.
    pub fn init_env_with_name(&mut self, name: impl AsRef<str>) -> Result<(), Error> {
        self.pty
            .send_line(&format!(r#"mkdir "{}""#, name.as_ref()))
            .context("failed to send mkdir command")?;
        self.pty
            .wait_for_prompt()
            .context("prompt never appeared")?;
        self.pty
            .send_line(&format!(r#"cd "{}""#, name.as_ref()))
            .context("failed to send cd command")?;
        self.pty
            .wait_for_prompt()
            .context("prompt never appeared")?;
        self.pty
            .send_line("flox init")
            .with_context(|| format!("failed to cd into directory: {}", name.as_ref()))?;
        self.pty
            .exp_string(&format!("Created environment '{}'", name.as_ref()))?;
        self.pty
            .wait_for_prompt()
            .context("prompt never appeared")?;
        Ok(())
    }

    /// Does a `flox edit -f` with a file containing the provided string.
    ///
    /// This will fail if the shell can't find an environment to edit.
    pub fn edit_with_manifest_contents(
        &mut self,
        manifest_contents: impl AsRef<str>,
    ) -> Result<(), Error> {
        let file =
            tempfile::NamedTempFile::new().context("failed to create temporary manifest file")?;
        std::fs::write(file.path(), manifest_contents.as_ref())
            .context("failed to write temporary manifest")?;
        let cmd = format!("flox edit -f {}", file.path().display());
        let res = self
            .pty
            .send_line(&cmd)
            .context("failed to run 'flox edit -f'");
        self.pty
            .exp_string("successfully updated")
            .context("edit was unsuccessful")?;
        self.pty.wait_for_prompt().context("never got prompt")?;
        // Remove the tempfile first so it isn't left laying around if there's an error
        std::fs::remove_file(file.path()).context("failed to delete temp manifest")?;
        let _ = res?;
        Ok(())
    }

    /// Activates the environment in the current directory
    pub fn activate(&mut self) -> Result<(), Error> {
        self.pty.send_line("flox activate")?;
        self.pty.exp_string("bash-5.2$")?;
        self.shell_reconfig()?;
        self.pty.wait_for_prompt()?;
        Ok(())
    }

    /// Activates the environment in the current directory
    pub fn activate_with_args(&mut self, args: &[&str]) -> Result<(), Error> {
        let cmd = {
            let mut buf = String::from("flox activate");
            for arg in args.iter() {
                buf.push_str(" ");
                buf.push_str(arg);
            }
            buf
        };
        self.pty.send_line(&cmd)?;
        self.pty.exp_string("bash-5.2$")?;
        self.shell_reconfig()?;
        self.pty.wait_for_prompt()?;
        Ok(())
    }

    /// Reads the _FLOX_ACTIVATION_UUID value from an activated shell
    pub fn read_activation_uuid(&mut self) -> Result<String, Error> {
        self.pty
            .send_line("echo $_FLOX_ACTIVATION_UUID")
            .context("failed to send command")?;
        sleep(Duration::from_millis(100));
        let value = self.pty.read_line().context("failed to read line")?;
        self.pty
            .wait_for_prompt()
            .context("prompt never appeared")?;
        Ok(value)
    }
}

/// Locates the watchdog fingerprinted with the provided UUID
pub fn find_watchdog_pid_with_uuid(uuid: impl AsRef<str>, system: &mut System) -> Option<u32> {
    find_pid_with_name_and_uuid("flox-watchdog", uuid, system)
}

/// Locates the watchdog fingerprinted with the provided UUID
pub fn find_process_compose_pid_with_uuid(
    uuid: impl AsRef<str>,
    system: &mut System,
) -> Option<u32> {
    find_pid_with_name_and_uuid("process-compose", uuid, system)
}

/// Data that's global to a single test
pub struct TestGlobals {
    pub dirs: IsolatedHome,
    pub system: Arc<Mutex<System>>,
}

impl TestGlobals {
    pub fn new() -> Self {
        Self {
            dirs: IsolatedHome::new().unwrap(),
            system: Arc::new(Mutex::new(System::new())),
        }
    }

    pub fn new_bash_shell(&self, expect_timeout: Option<u64>) -> Result<ShellProcess, Error> {
        ShellProcess::spawn(&self.dirs, expect_timeout)
    }

    pub fn watchdog_with_uuid(&mut self, uuid: impl AsRef<str>) -> Option<ProcToGC<WatchdogProc>> {
        let mut system = self.system.lock().expect("system lock was poisoned");
        find_watchdog_pid_with_uuid(uuid, &mut system)
            .map(|pid| ProcToGC::new_with_pid(pid, self.system.clone(), WatchdogProc))
    }

    pub fn process_compose_with_uuid(
        &mut self,
        uuid: impl AsRef<str>,
    ) -> Option<ProcToGC<ProcessComposeProc>> {
        let mut system = self.system.lock().expect("system lock was poisoned");
        find_process_compose_pid_with_uuid(uuid, &mut system)
            .map(|pid| ProcToGC::new_with_pid(pid, self.system.clone(), ProcessComposeProc))
    }
}

#[derive(Debug)]
pub struct ProcToGC<T> {
    is_terminated: bool,
    pid: u32,
    system: Arc<Mutex<System>>,
    _kind: T,
}

pub struct ProcessComposeProc;
pub struct WatchdogProc;
pub struct OtherProc;

impl<T> ProcToGC<T> {
    pub fn new_with_pid(pid: u32, system: Arc<Mutex<System>>, kind: T) -> Self {
        Self {
            is_terminated: false,
            pid,
            system,
            _kind: kind,
        }
    }

    pub fn is_running(&mut self) -> bool {
        if self.is_terminated {
            return false;
        }
        let pid = Pid::from_u32(self.pid);
        let mut system = self.system.lock().unwrap();
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            false,
            ProcessRefreshKind::new(),
        );
        let Some(proc) = system.process(pid) else {
            self.is_terminated = true;
            return false;
        };
        let status = proc.status();
        (status != ProcessStatus::Dead) && (status != ProcessStatus::Zombie)
    }

    pub fn send_sigterm(&mut self) {
        if self.is_terminated {
            return;
        }
        if self.is_running() {
            // If the lock is poisoned there's literally nothing we can
            // do about it
            let Ok(system) = self.system.lock() else {
                return;
            };
            let pid = Pid::from_u32(self.pid);
            if let Some(proc) = system.process(pid) {
                proc.kill_with(Signal::Term);
            }
        }
    }

    pub fn send_sigkill(&mut self) {
        if self.is_terminated {
            return;
        }
        if self.is_running() {
            // If the lock is poisoned there's literally nothing we can
            // do about it
            let Ok(system) = self.system.lock() else {
                return;
            };
            let pid = Pid::from_u32(self.pid);
            if let Some(proc) = system.process(pid) {
                proc.kill_with(Signal::Kill);
            }
        }
    }

    pub fn wait_for_termination_with_timeout(&mut self, millis: u64) -> Result<(), Error> {
        let mut remaining = millis;
        let interval = 10;
        let mut next_sleep = interval.min(remaining);
        loop {
            if !self.is_running() {
                return Ok(());
            }
            if remaining == 0 {
                bail!("timed out waiting for termination");
            }
            sleep(Duration::from_millis(next_sleep));
            if let Some(new_remaining) = remaining.checked_sub(interval) {
                next_sleep = interval;
                remaining = new_remaining;
            } else {
                next_sleep = remaining;
                remaining = 0;
            }
        }
    }
}

impl<T> Drop for ProcToGC<T> {
    fn drop(&mut self) {
        self.send_sigterm();
    }
}

/// Locates a process with a given name and the provided UUID fingerprint
pub fn find_pid_with_name_and_uuid(
    name: &str,
    uuid: impl AsRef<str>,
    system: &mut System,
) -> Option<u32> {
    let var = format!("_FLOX_ACTIVATION_UUID={}", uuid.as_ref());
    let update_kind = ProcessRefreshKind::new()
        .with_exe(UpdateKind::Always)
        .with_environ(UpdateKind::Always);
    system.refresh_processes_specifics(ProcessesToUpdate::All, false, update_kind);
    for proc in system
        .processes()
        .values()
        .filter(|proc| proc.exe().is_some_and(|p| p.ends_with(name)))
    {
        for env_var in proc.environ().iter() {
            if env_var.to_string_lossy() == var {
                return Some(proc.pid().as_u32());
            }
        }
    }
    None
}

/// Locates all processes with a given name and the provided UUID fingerprint
pub fn find_all_pids_with_uuid(uuid: impl AsRef<str>) -> Option<u32> {
    let var = format!("_FLOX_ACTIVATION_UUID={}", uuid.as_ref());
    let mut system = System::new();
    let update_kind = ProcessRefreshKind::new().with_environ(UpdateKind::Always);
    system.refresh_processes_specifics(ProcessesToUpdate::All, false, update_kind);
    for proc in system.processes().values() {
        for env_var in proc.environ().iter() {
            if env_var.to_string_lossy() == var {
                return Some(proc.pid().as_u32() as u32);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use indoc::formatdoc;

    use super::*;

    #[test]
    fn can_construct_shell() {
        let globals = TestGlobals::new();
        let _shell = globals.new_bash_shell(Some(1000)).unwrap();
    }

    #[test]
    fn can_activate() {
        let globals = TestGlobals::new();
        let mut shell = globals.new_bash_shell(Some(1000)).unwrap();
        shell.init_env_with_name("myenv").unwrap();
        shell.send_line("flox activate").unwrap();
        shell.exp_string("flox [myenv]").unwrap();
        shell.send_line(r#"echo "$_activate_d""#).unwrap();
        shell.exp_string("/nix/store").unwrap();
    }

    #[test]
    fn update_env_with_manifest() {
        let globals = TestGlobals::new();
        let mut shell = globals.new_bash_shell(Some(2000)).unwrap();
        shell.init_env_with_name("myenv").unwrap();
        shell
            .edit_with_manifest_contents(formatdoc! {r#"
            version = 1

            [hook]
            on-activate = '''
                echo howdy
            '''

            [options]
            systems = ["aarch64-darwin", "x86_64-darwin", "aarch64-linux", "x86_64-linux"]
        "#})
            .unwrap();
        let manifest_contents =
            std::fs::read_to_string(shell.dirs.home_dir.join("myenv/.flox/env/manifest.toml"))
                .unwrap();
        assert!(manifest_contents.find("howdy").is_some());
        let cmd = format!(
            "FLOX_SHELL={} flox activate",
            std::env::var("FLOX_SHELL_BASH").unwrap()
        );
        shell.send_line(&cmd).unwrap();
        shell.exp_string("howdy").unwrap();
    }

    #[test]
    fn read_activation_uuid() {
        let globals = TestGlobals::new();
        let mut shell = globals.new_bash_shell(Some(1000)).unwrap();
        shell.init_env_with_name("myenv").unwrap();
        shell.activate().unwrap();
        shell
            .execute(
                "echo $_FLOX_ACTIVATION_UUID",
                r#"\w{8}-\w{4}-\w{4}-\w{4}-\w{12}"#,
            )
            .unwrap();
    }

    #[test]
    fn can_locate_watchdog() {
        let globals = TestGlobals::new();
        let mut shell = globals.new_bash_shell(Some(1000)).unwrap();
        shell.init_env_with_name("myenv").unwrap();
        shell.activate().unwrap();
        let uuid = shell.read_activation_uuid().unwrap();
        let watchdog = find_watchdog_pid_with_uuid(uuid, &mut globals.system.lock().unwrap());
        assert!(watchdog.is_some());
    }

    #[test]
    fn can_locate_process_compose() {
        let globals = TestGlobals::new();
        let mut shell = globals.new_bash_shell(Some(1000)).unwrap();
        shell.init_env_with_name("myenv").unwrap();
        shell
            .edit_with_manifest_contents(formatdoc! {r#"
            version = 1

            [services.sleep_forever]
            command = "sleep 999999"
        "#})
            .unwrap();
        shell.activate_with_args(&["--start-services"]).unwrap();
        let uuid = shell.read_activation_uuid().unwrap();
        let process_compose =
            find_process_compose_pid_with_uuid(uuid, &mut globals.system.lock().unwrap());
        assert!(process_compose.is_some());
    }

    #[test]
    fn cleans_up_watchdog() {
        let mut globals = TestGlobals::new();
        let mut shell = globals.new_bash_shell(Some(1000)).unwrap();
        shell.init_env_with_name("myenv").unwrap();
        shell.activate().unwrap();
        let uuid = shell.read_activation_uuid().unwrap();
        let watchdog_proc = globals.watchdog_with_uuid(uuid);
        assert!(watchdog_proc.is_some());
        let Some(mut watchdog_proc) = watchdog_proc else {
            panic!("we literally just checked that it was Some(_)");
        };
        assert!(watchdog_proc.is_running());
        shell.send_line("exit").unwrap();
        shell.wait_for_prompt().unwrap();
        watchdog_proc
            .wait_for_termination_with_timeout(1000)
            .unwrap();
    }
}
