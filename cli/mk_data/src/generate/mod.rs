use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::Duration;
use std::vec;

use anyhow::{Context, bail};
use custom::{CustomJob, run_custom_job};
use env::{EnvJob, run_env_job};
use indicatif::{ProgressBar, ProgressStyle};
use init::{InitJob, run_init_job};
use lock::{LockJob, run_lock_job};
use resolve::{ResolveJob, run_resolve_job};
use search::{SearchJob, run_search_job};
use serde::Deserialize;
use show::{ShowJob, run_show_job};
use tempfile::TempDir;
use tracing::debug;
use walkdir::WalkDir;

use crate::{Cli, Error};
mod custom;
mod env;
mod init;
mod lock;
mod resolve;
mod search;
mod show;

/// The config file for the mock data to generate.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// Environment variables you want set during the generation process.
    ///
    /// You might use this to use the production vs. preview server, etc.
    pub vars: Option<HashMap<String, String>>,
    /// Jobs for the resolve endpoint
    pub resolve: Option<HashMap<String, ResolveJob>>,
    /// Jobs for the search command
    pub search: Option<HashMap<String, SearchJob>>,
    /// Jobs for the show command
    pub show: Option<HashMap<String, ShowJob>>,
    /// Jobs for the init command
    pub init: Option<HashMap<String, InitJob>>,
    /// Jobs for building environments from manifests
    pub env: Option<HashMap<String, EnvJob>>,
    /// Jobs for producing lockfiles from manifests
    pub lock: Option<HashMap<String, LockJob>>,
    /// Jobs with custom handling
    pub custom: Option<HashMap<String, CustomJob>>,
}

#[derive(Debug)]
pub enum JobKind {
    Resolve(ResolveJob),
    Search(SearchJob),
    Show(ShowJob),
    Init(InitJob),
    Env(EnvJob),
    Lock(LockJob),
    Custom(CustomJob),
}

/// All of the information and state necessary to run a particular job.
#[derive(Debug)]
pub struct JobCtx {
    pub name: String,
    pub job: JobKind,
    pub category: String,
    pub tmp_dir: TempDir,
    pub input_dir: PathBuf,
    pub category_dir: PathBuf,
    pub vars: HashMap<String, String>,
}

/// All of the information and state for a job that can be prepared before
/// runtime e.g. all of `JobCtx` minus the temporary directory, which is only
/// generated right before running the job.
#[derive(Debug)]
pub struct ProtoJobCtx {
    pub name: String,
    pub job: JobKind,
    pub category: String,
    pub input_dir: PathBuf,
    pub category_dir: PathBuf,
    pub vars: HashMap<String, String>,
}

impl ProtoJobCtx {
    pub fn run(self) -> Result<(), Error> {
        let tmp_dir =
            TempDir::new_in(&self.category_dir).context("failed to create tempdir for job")?;
        let ctx = JobCtx {
            name: self.name,
            job: self.job,
            category: self.category,
            tmp_dir,
            input_dir: self.input_dir,
            category_dir: self.category_dir,
            vars: self.vars,
        };
        match ctx.job {
            JobKind::Resolve(ref resolve_job) => run_resolve_job(resolve_job, &ctx),
            JobKind::Search(ref search_job) => run_search_job(search_job, &ctx),
            JobKind::Show(ref show_job) => run_show_job(show_job, &ctx),
            JobKind::Init(ref init_job) => run_init_job(init_job, &ctx),
            JobKind::Env(ref env_job) => run_env_job(env_job, &ctx),
            JobKind::Lock(ref lock_job) => run_lock_job(lock_job, &ctx),
            JobKind::Custom(ref custom_job) => run_custom_job(custom_job, &ctx),
        }
    }
}

/// Returns an error containing `stderr` if the `Output` was not a success.
pub fn err_with_stderr_if_err(
    Output { status, stderr, .. }: Output,
    ignore: bool,
) -> Result<(), Error> {
    if ignore {
        return Ok(());
    }
    if !status.success() {
        bail!(String::from_utf8_lossy(&stderr).to_string())
    } else {
        Ok(())
    }
}

/// Moves the response file from `<workdir>/resp.yaml` to
/// `test_data/<category>/<name>.yaml`
pub fn move_response_file(resp_path: &Path, ctx: &JobCtx) -> Result<(), Error> {
    let dest = ctx.category_dir.join(format!("{}.yaml", ctx.name));
    debug!(category = ctx.category, name = ctx.name, src = %resp_path.display(), dest = %dest.display(), "moving response file");
    std::fs::copy(resp_path, dest).context("failed to move response file")?;
    Ok(())
}

/// Mostly copied from `flox_rust_sdk`
pub(crate) fn copy_dir_recursive(
    from: impl AsRef<Path>,
    to: impl AsRef<Path>,
) -> Result<(), Error> {
    debug!(FROM = %from.as_ref().display(), "XXXXXX");
    debug!(TO = %to.as_ref().display(), "XXXXXX");
    if !to.as_ref().exists() {
        std::fs::create_dir_all(&to).unwrap();
    }
    for entry in WalkDir::new(&from).into_iter().skip(1) {
        let entry = entry.unwrap();
        debug!(path = %entry.path().display(), "handling dir entry");
        let stripped = entry.path().strip_prefix(&from).unwrap();
        debug!(fragment = %stripped.display(), "stripped path prefix");
        let new_path = to.as_ref().join(entry.path().strip_prefix(&from).unwrap());
        match entry.file_type() {
            file_type if file_type.is_dir() => {
                debug!(path = %new_path.display(), "creating new directory");
                std::fs::create_dir(new_path).context("failed to create new directory")?;
            },
            file_type if file_type.is_file() => {
                debug!(path = %new_path.display(), "copying file");
                std::fs::copy(entry.path(), &new_path).context("failed to copy file")?;
            },
            _ => {
                // Skip symlinks
            },
        }
    }
    Ok(())
}

/// Unpacks the contents of the specified directories directly into the working
/// directory. This is analogous to `cp <input>/* .`
pub fn unpack_inputs(
    input_data_dir: &Path,
    inputs: &[String],
    workdir: &Path,
    _ctx: &JobCtx,
) -> Result<(), Error> {
    for input_path in inputs.iter() {
        let full_input_path = input_data_dir.join(input_path);
        if !full_input_path.exists() {
            bail!("path does not exist: {}", full_input_path.display());
        }
        copy_dir_recursive(&full_input_path, workdir).with_context(|| {
            format!(
                "failed to copy contents of {} to {}",
                full_input_path.display(),
                workdir.display()
            )
        })?;
    }
    Ok(())
}

pub trait JobCommand {
    /// Applies common options for command execution.
    fn apply_common_options(&mut self, workdir: &Path) -> &mut Command;
    /// Applies any global variables, then clears the FloxHub token
    fn apply_vars(&mut self, vars: &HashMap<String, String>) -> &mut Command;
    /// Applies the variable that specifies the output path for the recording.
    fn apply_recording_vars(&mut self, resp_path: &Path) -> &mut Command;
}

impl JobCommand for Command {
    fn apply_common_options(&mut self, workdir: &Path) -> &mut Command {
        self.stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .current_dir(workdir)
    }

    fn apply_vars(&mut self, vars: &HashMap<String, String>) -> &mut Command {
        for (name, value) in vars.iter() {
            self.env(name, value);
        }
        self.env("FLOX_FLOXHUB_TOKEN", "")
    }

    fn apply_recording_vars(&mut self, resp_path: &Path) -> &mut Command {
        self.env("_FLOX_CATALOG_DUMP_RESPONSE_FILE", resp_path)
    }
}

/// Creates the directory structure for the output files.
pub fn create_output_dir(output_dir: &Path) -> Result<(), Error> {
    if !output_dir.exists() {
        std::fs::create_dir_all(output_dir)?;
    }
    let init_dir = output_dir.join("init");
    let resolve_dir = output_dir.join("resolve");
    let search_dir = output_dir.join("search");
    let show_dir = output_dir.join("show");
    let envs_dir = output_dir.join("env");
    let lock_dir = output_dir.join("lock");
    let custom_dir = output_dir.join("custom");
    let dirs = [
        &init_dir,
        &resolve_dir,
        &search_dir,
        &show_dir,
        &envs_dir,
        &lock_dir,
        &custom_dir,
    ];
    for dir in dirs.iter() {
        if !dir.exists() {
            std::fs::create_dir_all(dir)?;
        }
    }
    Ok(())
}

/// Determines the output directory for the mock data
pub fn get_output_dir(args: &Cli) -> Result<PathBuf, Error> {
    if let Some(output) = &args.output {
        if output.is_absolute() {
            Ok(output.clone())
        } else {
            let path = std::env::current_dir()
                .context("couldn't read current directory")?
                .join(output);
            Ok(path)
        }
    } else {
        Ok(std::env::current_dir()
            .context("couldn't read current dir, was it deleted?")?
            .join("generated"))
    }
}

/// Determines the input data directory
pub fn get_input_dir(args: &Cli) -> Result<PathBuf, Error> {
    if let Some(input) = &args.input {
        if input.is_absolute() {
            Ok(input.clone())
        } else {
            let path = std::env::current_dir()
                .context("couldn't read current directory")?
                .join(input);
            Ok(path)
        }
    } else {
        Ok(std::env::current_dir()
            .context("couldn't read current dir, was it deleted?")?
            .join("input_data"))
    }
}

/// Generates all the jobs from the spec file.
pub fn generate_jobs(
    config: &Config,
    input_dir: &Path,
    output_dir: &Path,
    force: bool,
) -> Result<Vec<ProtoJobCtx>, Error> {
    let mut jobs = vec![];

    let resolve_dir = output_dir.join("resolve");
    let resolve_jobs = enumerate_output_file_jobs_to_run(
        &config.resolve.clone().unwrap_or_default(),
        force,
        &resolve_dir,
        "yaml",
    );
    for (name, job) in resolve_jobs {
        let kind = JobKind::Resolve(job);
        let ctx = ProtoJobCtx {
            name,
            job: kind,
            category: "resolve".to_string(),
            input_dir: input_dir.to_path_buf(),
            category_dir: resolve_dir.clone(),
            vars: config.vars.clone().unwrap_or_default(),
        };
        jobs.push(ctx);
    }

    let search_dir = output_dir.join("search");
    let search_jobs = enumerate_output_file_jobs_to_run(
        &config.search.clone().unwrap_or_default(),
        force,
        &search_dir,
        "yaml",
    );
    for (name, job) in search_jobs {
        let kind = JobKind::Search(job);
        let ctx = ProtoJobCtx {
            name,
            job: kind,
            category: "search".to_string(),
            input_dir: input_dir.to_path_buf(),
            category_dir: search_dir.clone(),
            vars: config.vars.clone().unwrap_or_default(),
        };
        jobs.push(ctx);
    }

    let show_dir = output_dir.join("show");
    let show_jobs = enumerate_output_file_jobs_to_run(
        &config.show.clone().unwrap_or_default(),
        force,
        &show_dir,
        "yaml",
    );
    for (name, job) in show_jobs {
        let kind = JobKind::Show(job);
        let ctx = ProtoJobCtx {
            name,
            job: kind,
            category: "show".to_string(),
            input_dir: input_dir.to_path_buf(),
            category_dir: show_dir.clone(),
            vars: config.vars.clone().unwrap_or_default(),
        };
        jobs.push(ctx);
    }

    let lock_dir = output_dir.join("lock");
    let lock_jobs = enumerate_output_file_jobs_to_run(
        &config.lock.clone().unwrap_or_default(),
        force,
        &lock_dir,
        "lock",
    );
    for (name, job) in lock_jobs {
        let kind = JobKind::Lock(job);
        let ctx = ProtoJobCtx {
            name,
            job: kind,
            category: "lock".to_string(),
            input_dir: input_dir.to_path_buf(),
            category_dir: lock_dir.clone(),
            vars: config.vars.clone().unwrap_or_default(),
        };
        jobs.push(ctx);
    }

    let env_dir = output_dir.join("env");
    let env_jobs =
        enumerate_output_dir_jobs_to_run(&config.env.clone().unwrap_or_default(), force, &env_dir);
    for (name, job) in env_jobs {
        let kind = JobKind::Env(job);
        let ctx = ProtoJobCtx {
            name,
            job: kind,
            category: "env".to_string(),
            input_dir: input_dir.to_path_buf(),
            category_dir: env_dir.clone(),
            vars: config.vars.clone().unwrap_or_default(),
        };
        jobs.push(ctx);
    }

    let init_dir = output_dir.join("init");
    let init_jobs = enumerate_output_dir_jobs_to_run(
        &config.init.clone().unwrap_or_default(),
        force,
        &init_dir,
    );
    for (name, job) in init_jobs {
        let kind = JobKind::Init(job);
        let ctx = ProtoJobCtx {
            name,
            job: kind,
            category: "init".to_string(),
            input_dir: input_dir.to_path_buf(),
            category_dir: init_dir.clone(),
            vars: config.vars.clone().unwrap_or_default(),
        };
        jobs.push(ctx);
    }

    let custom_dir = output_dir.join("custom");
    let custom_jobs = enumerate_output_dir_jobs_to_run(
        &config.custom.clone().unwrap_or_default(),
        force,
        &custom_dir,
    );
    for (name, job) in custom_jobs {
        let kind = JobKind::Custom(job);
        let ctx = ProtoJobCtx {
            name,
            job: kind,
            category: "custom".to_string(),
            input_dir: input_dir.to_path_buf(),
            category_dir: custom_dir.clone(),
            vars: config.vars.clone().unwrap_or_default(),
        };
        jobs.push(ctx);
    }

    Ok(jobs)
}

/// Scans the list of jobs to see which ones need to be re-run based on whether
/// an output from the previous run exists.
pub fn enumerate_output_file_jobs_to_run<T: Clone>(
    all_jobs: &HashMap<String, T>,
    force: bool,
    category_dir: &Path,
    extension: &str,
) -> HashMap<String, T> {
    if force {
        return all_jobs.clone();
    }
    all_jobs
        .iter()
        .filter_map(|(name, job)| {
            let output_file = category_dir.join(format!("{name}.{extension}"));
            if output_file.exists() {
                None
            } else {
                Some((name.clone(), job.clone()))
            }
        })
        .collect::<HashMap<_, _>>()
}

/// Scans the list of jobs to see which ones need to be re-run based on whether
/// an output from the previous run exists.
pub fn enumerate_output_dir_jobs_to_run<T: Clone>(
    all_jobs: &HashMap<String, T>,
    force: bool,
    category_dir: &Path,
) -> HashMap<String, T> {
    if force {
        return all_jobs.clone();
    }
    all_jobs
        .iter()
        .filter_map(|(name, job)| {
            let output_file = category_dir.join(name);
            if output_file.exists() {
                None
            } else {
                Some((name.clone(), job.clone()))
            }
        })
        .collect::<HashMap<_, _>>()
}

/// Executes the provided jobs
pub fn execute_jobs(jobs: Vec<ProtoJobCtx>, quiet: bool) -> Result<(), Error> {
    let mut spinner: Option<ProgressBar> = None;
    if !quiet {
        let s = indicatif::ProgressBar::new_spinner();
        s.set_style(ProgressStyle::with_template("{spinner} {wide_msg} {prefix:>}").unwrap());
        s.enable_steady_tick(Duration::from_millis(50));
        spinner = Some(s);
    }
    for job in jobs {
        if !quiet {
            if let Some(ref s) = spinner {
                s.set_message(format!(
                    "Running job: category={}, name={}",
                    &job.category, &job.name
                ));
            }
        }
        job.run()?;
    }
    if !quiet {
        if let Some(s) = spinner {
            s.finish_and_clear();
        }
    }
    Ok(())
}

/// Runs the `pre_cmd` for a given job.
fn run_pre_cmd(
    pre_cmd: &str,
    vars: &HashMap<String, String>,
    dir: &Path,
    ignore_errors: bool,
) -> Result<(), Error> {
    let mut cmd = Command::new("bash");
    if !ignore_errors {
        cmd.arg("-eu");
    }
    cmd.arg("-c").arg(pre_cmd);
    cmd.current_dir(dir);
    for (key, value) in vars.iter() {
        cmd.env(key, value);
    }
    debug!("pre_cmd: {:?}", cmd);
    let output = cmd.output().context("couldn't call command")?;
    if !output.status.success() && !ignore_errors {
        let stderr = String::from_utf8_lossy(output.stderr.as_slice());
        anyhow::bail!("{}", stderr);
    }
    Ok(())
}

/// Runs the `cmd` for a given job.
pub fn run_cmd(
    gen_cmd: &str,
    vars: &HashMap<String, String>,
    dir: &Path,
    output_file: &Path,
    ignore_errors: bool,
) -> Result<(), Error> {
    let mut cmd = Command::new("bash");
    if !ignore_errors {
        cmd.arg("-eu");
    }
    cmd.arg("-c").arg(gen_cmd);
    cmd.current_dir(dir);
    for (key, value) in vars.iter() {
        cmd.env(key, value);
    }

    // Don't leak custom catalogs from the current user.
    cmd.env("FLOX_FLOXHUB_TOKEN", "");
    cmd.env("_FLOX_CATALOG_DUMP_RESPONSE_FILE", output_file);
    debug!("cmd: {:?}", cmd);
    let output = cmd.output().context("couldn't call command")?;
    if !output.status.success() && !ignore_errors {
        let stderr = String::from_utf8_lossy(output.stderr.as_slice());
        anyhow::bail!("{}", stderr);
    }
    Ok(())
}

/// Runs the `post_cmd` for a given job.
pub fn run_post_cmd(
    post_cmd: &str,
    vars: &HashMap<String, String>,
    dir: &Path,
    output_file: &Path,
    ignore_errors: bool,
) -> Result<(), Error> {
    let mut cmd = Command::new("bash");
    if !ignore_errors {
        cmd.arg("-eu");
    }
    cmd.arg("-c").arg(post_cmd);
    cmd.current_dir(dir);
    for (key, value) in vars.iter() {
        cmd.env(key, value);
    }
    cmd.env("RESPONSE_FILE", output_file);
    debug!("post_cmd: {:?}", cmd);
    let output = cmd.output().context("couldn't call command")?;
    if !output.status.success() && !ignore_errors {
        let stderr = String::from_utf8_lossy(output.stderr.as_slice());
        anyhow::bail!("{}", stderr);
    }
    Ok(())
}
