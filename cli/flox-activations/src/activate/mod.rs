use std::borrow::Cow;
use std::collections::{HashSet, VecDeque};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{anyhow, Context, Error};
use clap::Args;

use crate::cli::StartOrAttachArgs;

#[derive(Debug, Clone, Default)]
pub enum ActivateMode {
    #[default]
    Dev,
    Run,
}

impl FromStr for ActivateMode {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "dev" => Ok(ActivateMode::Dev),
            "run" => Ok(ActivateMode::Run),
            _ => Err(anyhow!("unrecognized mode: {s}")),
        }
    }
}

impl std::fmt::Display for ActivateMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActivateMode::Dev => write!(f, "dev"),
            ActivateMode::Run => write!(f, "run"),
        }
    }
}

#[derive(Debug)]
pub enum SupportedShell {
    Bash,
    Fish,
    Zsh,
    Tcsh,
}

impl FromStr for SupportedShell {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "bash" => Ok(SupportedShell::Bash),
            "-bash" => Ok(SupportedShell::Bash),
            "zsh" => Ok(SupportedShell::Zsh),
            "-zsh" => Ok(SupportedShell::Zsh),
            "fish" => Ok(SupportedShell::Fish),
            "-fish" => Ok(SupportedShell::Fish),
            "tcsh" => Ok(SupportedShell::Tcsh),
            "-tcsh" => Ok(SupportedShell::Tcsh),
            _ => Err(anyhow!("unsupported shell: {s}")),
        }
    }
}

#[derive(Debug, Args)]
pub struct Phase1Args {
    #[arg(help = "The path to the rendered Flox environment.")]
    #[arg(short, long, value_name = "PATH")]
    pub env: PathBuf,
    #[arg(help = "Whether to skip sourcing the shell-specific files.")]
    #[arg(long)]
    pub turbo: bool,
    #[arg(help = "Whether to skip sourcing the etc/profile.d files.")]
    #[arg(long)]
    pub noprofile: bool,
    #[arg(help = "Which activation mode to use.")]
    #[arg(short, long, value_name = "MODE", default_value = "dev")]
    pub mode: ActivateMode,
    #[arg(help = "The fallback path for FLOX_ENV if it isn't set")]
    #[arg(short, long, value_name = "PATH")]
    pub fallback_flox_env_path: PathBuf,
    #[arg(help = "The PID of the shell invoking the activation script.")]
    #[arg(long, value_name = "PID")]
    pub shell_pid: i32,
    #[arg(help = "The command to run inside the activation.")]
    #[arg(short, long = "command", trailing_var_arg = true, value_name = "CMD")]
    pub cmd: Vec<String>,
}

fn reexport_with_default(
    mut buffer: impl Write,
    var: &str,
    default: &str,
) -> Result<String, Error> {
    let value = std::env::var(var).unwrap_or(default.to_string());
    let line = format!("export {var}=\"{value}\"\n");
    buffer.write_all(line.as_bytes())?;
    Ok(value)
}

fn redeclare_readonly_with_default(
    mut buffer: impl Write,
    var: &str,
    default: &str,
) -> Result<String, Error> {
    let value = std::env::var(var).unwrap_or(default.to_string());
    let line = format!("declare -r {var}=\"{value}\"\n");
    buffer.write_all(line.as_bytes())?;
    Ok(value)
}

fn declare_readonly(mut buffer: impl Write, var: &str, value: &str) -> Result<(), Error> {
    let line = format!("declare -r {var}=\"{value}\"\n");
    buffer.write_all(line.as_bytes())?;
    Ok(())
}

fn export_var(mut buffer: impl Write, var: &str) -> Result<(), Error> {
    let line = format!("export {var}\n");
    buffer.write_all(line.as_bytes())?;
    Ok(())
}

fn export_var_value(mut buffer: impl Write, var: &str, value: &str) -> Result<(), Error> {
    let line = format!("export {var}=\"{value}\"\n");
    buffer.write_all(line.as_bytes())?;
    Ok(())
}

fn unset_var(mut buffer: impl Write, var: &str) -> Result<(), Error> {
    let line = format!("unset {var}\n");
    buffer.write_all(line.as_bytes())?;
    Ok(())
}

fn separate_dir_list(joined: &str) -> Vec<PathBuf> {
    joined
        .split(':')
        .map(|s| Path::new(s).to_path_buf())
        .collect::<Vec<_>>()
}

fn get_flox_env_dirs() -> Result<Vec<PathBuf>, Error> {
    let value = std::env::var("FLOX_ENV_DIRS")?;
    Ok(separate_dir_list(&value))
}

fn get_path_dirs() -> Result<Vec<PathBuf>, Error> {
    let value = std::env::var("PATH")?;
    Ok(separate_dir_list(&value))
}

fn get_manpath_dirs() -> Result<Vec<PathBuf>, Error> {
    let value = std::env::var("MANPATH")?;
    Ok(separate_dir_list(&value))
}

fn prepend_bin_dirs_to_path(flox_env_dirs: &[PathBuf], path_dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut dir_set = HashSet::new();
    let mut dirs = path_dirs.iter().cloned().collect::<VecDeque<_>>();
    for existing_dir in path_dirs.iter() {
        dir_set.insert(existing_dir.clone());
    }
    // Directories at the front of the list have been activated most recently,
    // so their directories should go at the front of the list. However, if
    // we just iterate in the typical order and prepend those directories to
    // PATH, you'll get those directories in reverse order of activation, so
    // we iterate in reverse order while prepending.
    for dir in flox_env_dirs.iter().rev() {
        let bin_dir = dir.join("bin");
        let sbin_dir = dir.join("sbin");
        if dir_set.insert(sbin_dir.clone()) {
            dirs.push_front(sbin_dir);
        }
        if dir_set.insert(bin_dir.clone()) {
            dirs.push_front(bin_dir);
        }
    }
    let dirs = dirs.into_iter().collect::<Vec<_>>();
    dirs
}

fn prepend_man_dirs_to_manpath(flox_env_dirs: &[PathBuf], path_dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut dir_set = HashSet::new();
    let mut dirs = path_dirs.iter().cloned().collect::<VecDeque<_>>();
    for existing_dir in path_dirs.iter() {
        dir_set.insert(existing_dir.clone());
    }
    // Directories at the front of the list have been activated most recently,
    // so their directories should go at the front of the list. However, if
    // we just iterate in the typical order and prepend those directories to
    // PATH, you'll get those directories in reverse order of activation, so
    // we iterate in reverse order while prepending.
    for dir in flox_env_dirs.iter().rev() {
        let man_dir = dir.join("share/man");
        if dir_set.insert(man_dir.clone()) {
            dirs.push_front(man_dir);
        }
    }
    let dirs = dirs.into_iter().collect::<Vec<_>>();
    dirs
}

pub fn phase_one(args: &Phase1Args) -> Result<Vec<u8>, Error> {
    let mut buffer = Vec::new();
    reexport_with_default(&mut buffer, "FLOX_PROMPT_ENVIRONMENTS", "")?;
    reexport_with_default(&mut buffer, "_FLOX_SET_PROMPT", "true")?;
    reexport_with_default(&mut buffer, "FLOX_PROMPT_COLOR_1", "99")?;
    reexport_with_default(&mut buffer, "FLOX_PROMPT_COLOR_2", "141")?;
    export_var_value(
        &mut buffer,
        "_FLOX_ENV_ACTIVATION_MODE",
        args.mode.to_string().as_str(),
    )?;
    export_var_value(&mut buffer, "FLOX_TURBO", args.turbo.to_string().as_str())?;
    let flox_env_str = reexport_with_default(
        &mut buffer,
        "FLOX_ENV",
        args.fallback_flox_env_path.to_string_lossy().as_ref(),
    )?;
    let flox_env = Path::new(&flox_env_str);
    let flox_env_realpath =
        std::fs::read_link(flox_env).context("FLOX_ENV points to invalid path")?;
    let flox_activate_store_path = reexport_with_default(
        &mut buffer,
        "_FLOX_ACTIVATE_STORE_PATH",
        flox_env_realpath.to_string_lossy().as_ref(),
    )?;
    let flox_shell = std::env::var("FLOX_SHELL")
        .or_else(|_| std::env::var("SHELL"))
        .unwrap_or("bash".to_string());
    declare_readonly(&mut buffer, "_flox_shell", &flox_shell)?;
    let start_or_attach_args = StartOrAttachArgs {
        pid: args.shell_pid,
        flox_env: flox_env.to_path_buf(),
        store_path: flox_activate_store_path.clone(),
    };
    let runtime_dir_str = std::env::var("FLOX_RUNTIME_DIR")?;
    let runtime_dir = Path::new(runtime_dir_str.as_str());
    start_or_attach_args.handle_with_retries(3, runtime_dir, &mut buffer)?;
    export_var(&mut buffer, "_FLOX_ACTIVATION_STATE_DIR")?;
    export_var(&mut buffer, "_FLOX_ACTIVATION_ID")?;
    let flox_env_dirs = get_flox_env_dirs()?;
    let path_dirs = get_path_dirs()?;
    let manpath_dirs = get_manpath_dirs()?;
    let new_path_dirs = prepend_bin_dirs_to_path(&flox_env_dirs, &path_dirs)
        .into_iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join(":");
    let new_manpath_dirs = prepend_man_dirs_to_manpath(&flox_env_dirs, &manpath_dirs)
        .into_iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join(":");
    export_var_value(&mut buffer, "PATH", new_path_dirs.as_str())?;
    export_var_value(&mut buffer, "MANPATH", new_manpath_dirs.as_str())?;
    Ok(buffer)
}
