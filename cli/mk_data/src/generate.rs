use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use std::vec;

use anyhow::Context;
use indicatif::{ProgressBar, ProgressStyle};
use serde::Deserialize;
use tempfile::TempDir;
use tracing::debug;

use crate::{Cli, Error};

/// The config file for the mock data to generate.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// Environment variables you want set during the generation process.
    ///
    /// You might use this to use the production vs. preview server, etc.
    pub vars: Option<HashMap<String, String>>,
    /// Specs for the resolve endpoint
    pub resolve: Option<HashMap<String, RawSpec>>,
    /// Specs for the search command
    pub search: Option<HashMap<String, RawSpec>>,
    /// Specs for the show command
    pub show: Option<HashMap<String, RawSpec>>,
    /// Specs for the init command
    pub init: Option<HashMap<String, RawSpec>>,
    // /// Specs for manifest/lockfile pairs
    pub envs: Option<HashMap<String, RawSpec>>,
}

/// A spec for a single generated response file.
///
/// This is what's taken straight from the config file, so it's the "value" in a "name": "value" pair.
#[derive(Debug, Clone, Deserialize)]
pub struct RawSpec {
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

/// A spec for a single generated response file.
#[derive(Debug, Clone, Deserialize)]
pub struct Spec {
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

impl Spec {
    pub fn new(name: &str, raw_spec: &RawSpec) -> Self {
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
pub struct Job {
    pub category: String,
    pub tmp_dir: TempDir,
    pub spec: Spec,
    pub output_file: PathBuf,
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
    let dirs = [&init_dir, &resolve_dir, &search_dir, &show_dir, &envs_dir];
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
        Ok(output.clone())
    } else {
        Ok(std::env::current_dir()
            .context("couldn't read current dir, was it deleted?")?
            .join("generated"))
    }
}

/// Determines the input data directory
pub fn get_input_dir(args: &Cli) -> Result<PathBuf, Error> {
    if let Some(input) = &args.input {
        Ok(input.clone())
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
) -> Result<impl Iterator<Item = Job>, Error> {
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
    Ok(jobs.into_iter().flatten())
}

/// Generates the jobs for a given category.
pub fn generate_category_jobs<'a>(
    category: &str,
    raw_specs: impl Iterator<Item = (&'a String, &'a RawSpec)>,
    output_dir: &Path,
    force: bool,
) -> Result<Vec<Job>, Error> {
    let mut jobs: Vec<Job> = vec![];
    for (name, raw_spec) in raw_specs {
        let filename = output_dir.join(category).join(format!("{}.json", name));
        if !force && filename.exists() {
            continue;
        }
        if !force {
            if let Some(explicit_check_path) = &raw_spec.skip_if_output_exists {
                let filename = output_dir.join(explicit_check_path);
                if filename.exists() {
                    continue;
                }
            }
        }
        let tmp_dir = TempDir::new_in(output_dir)?;
        jobs.push(Job {
            category: category.into(),
            tmp_dir,
            spec: Spec::new(name, raw_spec),
            output_file: filename,
        });
    }
    Ok(jobs)
}

/// Executes the provided jobs
pub fn execute_jobs(
    jobs: impl Iterator<Item = Job>,
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
    job: &Job,
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
    cmd.arg("-c");
    cmd.arg(gen_cmd);
    cmd.current_dir(dir);
    if let Some(vars) = vars {
        for (key, value) in vars.iter() {
            cmd.env(key, value);
        }
    }

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
    cmd.arg("-c");
    cmd.arg(post_cmd);
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

/// Returns a `tracing`-compatible form of a [Path]
pub fn traceable_path(p: impl AsRef<Path>) -> impl tracing::Value {
    let path = p.as_ref();
    path.display().to_string()
}
