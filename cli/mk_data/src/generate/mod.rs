use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Duration;
use std::vec;

use anyhow::{Context, bail};
use custom::CustomJob;
use duct::Expression;
use env::EnvJob;
use indicatif::{ProgressBar, ProgressStyle};
use init::InitJob;
use lock::LockJob;
use resolve::ResolveJob;
use search::SearchJob;
use serde::Deserialize;
use show::ShowJob;
use tempfile::TempDir;
use tracing::{debug, trace};
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
    /// Specs for the resolve endpoint
    pub resolve: Option<HashMap<String, JobSpec>>,
    /// Specs for the search command
    pub search: Option<HashMap<String, JobSpec>>,
    /// Specs for the show command
    pub show: Option<HashMap<String, JobSpec>>,
    /// Specs for the init command
    pub init: Option<HashMap<String, JobSpec>>,
    /// Specs for manifest/lockfile pairs
    pub envs: Option<HashMap<String, JobSpec>>,
    /// Specs for build environments
    pub build: Option<HashMap<String, JobSpec>>,
}

/// A spec for a single generated response file.
///
/// This is what's taken straight from the config file, so it's the "value" in a "name": "value" pair.
#[derive(Debug, Clone, Deserialize)]
pub struct JobSpec {
    /// Check a specific path relative to the output directory rather than looking in the default
    /// location when determining whether this job needs to be re-run.
    pub skip_if_output_exists: Option<PathBuf>,
    /// A command to run before the command that generates the response.
    pub pre_cmd: Option<String>,
    /// The command that generates the response.
    pub cmd: String,
    /// A command that runs after generating the response, receives a `$RESPONSE_FILE` variable
    /// so it can modify the response after the fact.
    pub post_cmd: Option<String>,
    /// Files to copy into the temp directory of the job before running any commands.
    /// These are specified relative to the `input` directory. You may also specify directories
    /// here, in which case the entire directory will be copied.
    pub files: Option<Vec<PathBuf>>,
    /// Doesn't fail the job if there is an error running `pre_cmd`
    pub ignore_pre_cmd_errors: Option<bool>,
    /// Doesn't fail the job if there is an error running `cmd`
    pub ignore_cmd_errors: Option<bool>,
    /// Doesn't fail the job if there is an error running `post_cmd`
    pub ignore_post_cmd_errors: Option<bool>,
}

pub trait ToJob {
    fn to_job(&self, name: &str) -> Job;
}

/// A spec for a single generated response file.
#[derive(Debug, Clone, Deserialize)]
pub struct Job {
    /// The name of the file without the extension.
    pub name: String,
    /// A command to run before the command that generates the response.
    pub pre_cmd: Option<String>,
    /// The command that generates the response.
    pub cmd: String,
    /// A command that runs after generating the response, receives a `$RESPONSE_FILE` variable
    /// so it can modify the response after the fact.
    pub post_cmd: Option<String>,
    /// Files to copy into the temp directory of the job before running any commands.
    /// These are specified relative to the `input` directory. You may also specify directories
    /// here, in which case the entire directory will be copied.
    pub files: Option<Vec<PathBuf>>,
    /// Doesn't fail the job if there is an error running `pre_cmd`
    pub ignore_pre_cmd_errors: Option<bool>,
    /// Doesn't fail the job if there is an error running `cmd`
    pub ignore_cmd_errors: Option<bool>,
    /// Doesn't fail the job if there is an error running `post_cmd`
    pub ignore_post_cmd_errors: Option<bool>,
}

impl Job {
    pub fn new(name: &str, raw_spec: &JobSpec) -> Self {
        Self {
            name: name.into(),
            pre_cmd: raw_spec.pre_cmd.clone(),
            cmd: raw_spec.cmd.clone(),
            post_cmd: raw_spec.post_cmd.clone(),
            files: raw_spec.files.clone(),
            ignore_pre_cmd_errors: raw_spec.ignore_pre_cmd_errors,
            ignore_cmd_errors: raw_spec.ignore_cmd_errors,
            ignore_post_cmd_errors: raw_spec.ignore_post_cmd_errors,
        }
    }
}

#[derive(Debug)]
pub struct JobCtx {
    pub category: String,
    pub tmp_dir: TempDir,
    pub spec: Job,
    pub output_file: PathBuf,
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

#[derive(Debug)]
pub struct JobCtx2 {
    pub tmp_dir: TempDir,
    pub category_dir: PathBuf,
    pub category: String,
    pub name: String,
    pub vars: HashMap<String, String>,
}

/// Returns an error containing `stderr` if the `Output` was not a success.
pub fn stderr_if_err(Output { status, stderr, .. }: Output) -> Result<(), Error> {
    if !status.success() {
        bail!(String::from_utf8_lossy(&stderr).to_string())
    } else {
        Ok(())
    }
}

/// Moves the response file from `<workdir>/resp.yaml` to
/// `test_data/<category>/<name>.yaml`
pub fn move_response_file(resp_path: &Path, ctx: &JobCtx2) -> Result<(), Error> {
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
    if !to.as_ref().exists() {
        std::fs::create_dir(&to).unwrap();
    }
    for entry in WalkDir::new(&from).into_iter().skip(1) {
        let entry = entry.unwrap();
        let new_path = to.as_ref().join(entry.path().strip_prefix(&from).unwrap());
        match entry.file_type() {
            file_type if file_type.is_dir() => {
                std::fs::create_dir(new_path).context("failed to create new directory")?;
            },
            file_type if file_type.is_file() => {
                std::fs::copy(entry.path(), &new_path).context("failed to copy file")?;
            },
            _ => {
                bail!("don't try to copy symlinks, fancy pants");
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
    ctx: &JobCtx2,
) -> Result<(), Error> {
    for input_path in inputs.iter() {
        let path = input_data_dir.join(input_path);
        if !path.exists() {
            bail!("path does not exist: {}", path.display());
        }
        for item in path
            .read_dir()
            .with_context(|| format!("failed to read directory: {}", path.display()))?
        {
            let item = item.context("failed to dir entry")?;
            let item_path = item.path();
            let suffix = item_path.strip_prefix(&path).with_context(|| {
                format!(
                    "failed to strip prefix {} from {}",
                    path.display(),
                    item_path.display()
                )
            })?;
            let dest = workdir.join(suffix);
            let file_type = item.file_type().context("failed to get file type")?;
            if file_type.is_file() {
                debug!(category = "init", name = ctx.name, src = %item.path().display(), dest = %dest.display(), "copying input data");
                std::fs::copy(&path, &dest).with_context(|| {
                    format!("failed to copy {} to {}", path.display(), dest.display())
                })?;
            } else if file_type.is_dir() {
                debug!(category = "init", name = ctx.name, src = %item.path().display(), dest = %dest.display(), "copying input data");
                copy_dir_recursive(&path, &dest).with_context(|| {
                    format!("failed to copy {} to {}", path.display(), dest.display())
                })?;
            }
        }
    }
    Ok(())
}

pub trait JobCommand {
    /// Applies common options for command execution.
    fn apply_common_options(self, workdir: &Path) -> Expression;
    /// Applies any global variables, then clears the FloxHub token
    fn apply_vars(self, vars: &HashMap<String, String>) -> Expression;
    /// Applies the variable that specifies the output path for the recording.
    fn apply_recording_vars(self, resp_path: &Path) -> Expression;
}

impl JobCommand for Expression {
    fn apply_common_options(self, workdir: &Path) -> Expression {
        self.stdout_capture().stderr_capture().dir(workdir)
    }

    fn apply_vars(mut self, vars: &HashMap<String, String>) -> Expression {
        for (name, value) in vars.iter() {
            self = self.env(name, value);
        }
        self.env("FLOX_FLOXHUB_TOKEN", "")
    }

    fn apply_recording_vars(self, resp_path: &Path) -> Expression {
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
    let envs_dir = output_dir.join("envs");
    let build_dir = output_dir.join("build");
    let dirs = [
        &init_dir,
        &resolve_dir,
        &search_dir,
        &show_dir,
        &envs_dir,
        &build_dir,
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
    output_dir: &Path,
    force: bool,
) -> Result<impl Iterator<Item = JobCtx>, Error> {
    let mut jobs = vec![];
    if let Some(init) = config.init.as_ref() {
        jobs.push(
            generate_category_jobs("init", init.iter(), output_dir, force)
                .context("failed to generate init jobs")?,
        );
    }
    if let Some(resolve) = config.resolve.as_ref() {
        jobs.push(
            generate_category_jobs("resolve", resolve.iter(), output_dir, force)
                .context("failed to generate resolve jobs")?,
        );
    }
    if let Some(search) = config.search.as_ref() {
        jobs.push(
            generate_category_jobs("search", search.iter(), output_dir, force)
                .context("failed to generate search jobs")?,
        );
    }
    if let Some(show) = config.show.as_ref() {
        jobs.push(
            generate_category_jobs("show", show.iter(), output_dir, force)
                .context("failed to generate show jobs")?,
        );
    }
    if let Some(envs) = config.envs.as_ref() {
        jobs.push(
            generate_category_jobs("envs", envs.iter(), output_dir, force)
                .context("failed to generate envs jobs")?,
        );
    }
    if let Some(build) = config.build.as_ref() {
        jobs.push(
            generate_category_jobs("build", build.iter(), output_dir, force)
                .context("failed to generate build jobs")?,
        );
    }
    Ok(jobs.into_iter().flatten())
}

/// Generates the jobs for a given category.
pub fn generate_category_jobs<'a>(
    category: &str,
    raw_specs: impl Iterator<Item = (&'a String, &'a JobSpec)>,
    output_dir: &Path,
    force: bool,
) -> Result<Vec<JobCtx>, Error> {
    let mut jobs: Vec<JobCtx> = vec![];
    for (name, raw_spec) in raw_specs {
        let response_filename = output_dir.join(category).join(format!("{}.yaml", name));
        match (force, raw_spec.skip_if_output_exists.as_ref()) {
            (false, Some(path)) => {
                let check_path = output_dir.join(path);
                if check_path.exists() {
                    trace!(name, explicit = true, path = %check_path.display(), "skipping job because output exists");
                    continue;
                }
            },
            (false, None) => {
                if response_filename.exists() {
                    trace!(name, explicit = false, path = %response_filename.display(), "skiping job because output exists");
                    continue;
                }
            },
            (true, _) => {},
        }
        debug!(category, name, "adding job to queue");
        let tmp_dir = TempDir::new_in(output_dir)?;
        jobs.push(JobCtx {
            category: category.into(),
            tmp_dir,
            spec: Job::new(name, raw_spec),
            output_file: response_filename,
        });
    }
    Ok(jobs)
}

/// Executes the provided jobs
pub fn execute_jobs(
    jobs: impl Iterator<Item = JobCtx>,
    vars: &Option<HashMap<String, String>>,
    input_dir: &Path,
    quiet: bool,
) -> Result<(), Error> {
    let mut spinner: Option<ProgressBar> = None;
    if !quiet {
        let s = indicatif::ProgressBar::new_spinner();
        s.set_style(ProgressStyle::with_template("{spinner} {wide_msg} {prefix:>}").unwrap());
        s.enable_steady_tick(Duration::from_millis(50));
        spinner = Some(s);
    }
    for job in jobs {
        debug!(
            category = job.category,
            name = job.spec.name,
            "executing job"
        );
        if !quiet {
            if let Some(ref s) = spinner {
                s.set_message(format!(
                    "Running job: category={}, name={}",
                    &job.category, &job.spec.name
                ));
            }
        }
        execute_job(&job, vars, input_dir)
            .with_context(|| format!("failed to execute job: {}", job.spec.name))?;
    }
    if !quiet {
        if let Some(s) = spinner {
            s.finish_and_clear();
        }
    }
    Ok(())
}

/// Executes a single job
pub fn execute_job(
    job: &JobCtx,
    vars: &Option<HashMap<String, String>>,
    input_dir: &Path,
) -> Result<(), Error> {
    if let Some(files) = job.spec.files.as_ref() {
        debug!(
            category = job.category,
            name = job.spec.name,
            "copying files"
        );
        copy_files(files, input_dir, job.tmp_dir.path()).context("copying files failed")?;
    }
    if let Some(pre_cmd) = job.spec.pre_cmd.as_ref() {
        let ignore_errors = job.spec.ignore_pre_cmd_errors.unwrap_or(false);
        debug!(
            category = job.category,
            name = job.spec.name,
            ignore_errors,
            "running pre_cmd"
        );
        run_pre_cmd(pre_cmd, vars, job.tmp_dir.path(), ignore_errors).context("pre_cmd failed")?;
    }
    let ignore_errors = job.spec.ignore_cmd_errors.unwrap_or(false);
    debug!(
        category = job.category,
        name = job.spec.name,
        ignore_errors,
        "running cmd"
    );
    run_cmd(
        job.spec.cmd.as_ref(),
        vars,
        job.tmp_dir.path(),
        &job.output_file,
        ignore_errors,
    )
    .context("cmd failed")?;
    if let Some(post_cmd) = job.spec.post_cmd.as_ref() {
        let ignore_errors = job.spec.ignore_post_cmd_errors.unwrap_or(false);
        debug!(
            category = job.category,
            name = job.spec.name,
            ignore_errors,
            "running post_cmd"
        );
        run_post_cmd(
            post_cmd,
            vars,
            job.tmp_dir.path(),
            &job.output_file,
            ignore_errors,
        )
        .context("post_cmd failed")?;
    }
    Ok(())
}

/// Runs the `pre_cmd` for a given job.
fn run_pre_cmd(
    pre_cmd: &str,
    vars: &Option<HashMap<String, String>>,
    dir: &Path,
    ignore_errors: bool,
) -> Result<(), Error> {
    let mut cmd = Command::new("bash");
    if !ignore_errors {
        cmd.arg("-eu");
    }
    cmd.arg("-c").arg(pre_cmd);
    cmd.current_dir(dir);
    if let Some(vars) = vars {
        for (key, value) in vars.iter() {
            cmd.env(key, value);
        }
    }
    debug!("pre_cmd: {:?}", cmd);
    let output = cmd.output().context("couldn't call command")?;
    if !output.status.success() && !ignore_errors {
        let stderr = String::from_utf8_lossy(output.stderr.as_slice());
        anyhow::bail!("{}", stderr);
    }
    Ok(())
}

/// Runs the `pre_cmd` for a given job.
fn run_pre_cmd2(
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

/// Copies the files from the spec to the temp directory.
fn copy_files(files: &[PathBuf], input_dir: &Path, working_dir: &Path) -> Result<(), Error> {
    for rel_path in files.iter() {
        let src = input_dir.join(rel_path);
        if !src.exists() {
            anyhow::bail!("file does not exist: {:?}", src);
        }
        if src.is_file() {
            let dest = working_dir.join(src.file_name().unwrap());
            debug!(
                src = traceable_path(&src),
                dest = traceable_path(&dest),
                "copying file"
            );
            std::fs::copy(&src, &dest).with_context(|| {
                format!(
                    "couldn't copy file: '{}' -> '{}'",
                    src.display(),
                    working_dir.display()
                )
            })?;
            continue;
        }
        if src.is_dir() {
            debug!(
                src = traceable_path(&src),
                dest = traceable_path(&working_dir),
                "copying directory"
            );
            fs_extra::dir::copy(&src, working_dir, &fs_extra::dir::CopyOptions::default())
                .with_context(|| {
                    format!(
                        "failed to copy directory: '{}' -> '{}'",
                        src.display(),
                        working_dir.display()
                    )
                })?;
            continue;
        }
    }
    Ok(())
}

/// Runs the `cmd` for a given job.
pub fn run_cmd(
    gen_cmd: &str,
    vars: &Option<HashMap<String, String>>,
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
    if let Some(vars) = vars {
        for (key, value) in vars.iter() {
            cmd.env(key, value);
        }
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

/// Runs the `cmd` for a given job.
pub fn run_cmd2(
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
    vars: &Option<HashMap<String, String>>,
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
    if let Some(vars) = vars {
        for (key, value) in vars.iter() {
            cmd.env(key, value);
        }
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

/// Runs the `post_cmd` for a given job.
pub fn run_post_cmd2(
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

/// Returns a `tracing`-compatible form of a [Path]
pub fn traceable_path(p: impl AsRef<Path>) -> impl tracing::Value {
    let path = p.as_ref();
    path.display().to_string()
}
