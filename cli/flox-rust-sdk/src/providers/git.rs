use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use log::{debug, error, warn};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum EmptyError {}

pub trait GitDiscoverError {
    fn not_found(&self) -> bool;
}

pub struct OriginInfo {
    pub name: String,
    pub url: String,
    pub reference: String,
    pub revision: Option<String>,
}

pub struct BranchInfo {
    pub name: String,
    pub remote: Option<String>,
    pub rev: String,
    pub description: String,
}

// simple git provider for the tasks we need to provide in
// flox
pub trait GitProvider: Sized + std::fmt::Debug {
    type InitError: std::error::Error;
    type CloneError: std::error::Error;
    type CommitError: std::error::Error;
    type PushError: std::error::Error;

    type CheckoutError: std::error::Error;
    type ListBranchesError: std::error::Error;
    type RenameError: std::error::Error;

    type AddRemoteError: std::error::Error;
    type MvError: std::error::Error;
    type RmError: std::error::Error;
    type AddError: std::error::Error;
    type ShowError: std::error::Error + Send + Sync + 'static;
    type DiscoverError: std::error::Error
        + GitDiscoverError
        + Send
        + Sync
        + std::fmt::Debug
        + 'static;
    type FetchError: std::error::Error;
    type SetOriginError: std::error::Error;
    type GetOriginError: std::error::Error;

    fn discover<P: AsRef<Path>>(path: P) -> Result<Self, Self::DiscoverError>;
    fn init<P: AsRef<Path>>(path: P, bare: bool) -> Result<Self, Self::InitError>;
    fn clone<O: AsRef<OsStr>, P: AsRef<Path>>(
        origin: O,
        path: P,
        bare: bool,
    ) -> Result<Self, Self::CloneError>;

    fn checkout(&self, name: &str, orphan: bool) -> Result<(), Self::CheckoutError>;
    fn list_branches(&self) -> Result<Vec<BranchInfo>, Self::ListBranchesError>;
    fn rename_branch(&self, new_name: &str) -> Result<(), Self::RenameError>;

    fn add_remote(&self, origin_name: &str, url: &str) -> Result<(), Self::AddRemoteError>;
    fn mv(&self, from: &Path, to: &Path) -> Result<(), Self::MvError>;
    fn rm(
        &self,
        paths: &[&Path],
        recursive: bool,
        force: bool,
        cached: bool,
    ) -> Result<(), Self::RmError>;
    fn add(&self, paths: &[&Path]) -> Result<(), Self::AddError>;
    fn commit(&self, message: &str) -> Result<(), Self::CommitError>;

    fn show(&self, object: &str) -> Result<OsString, Self::ShowError>;

    fn fetch(&self) -> Result<(), Self::FetchError>;
    fn push(&self, remote: &str, force: bool) -> Result<(), Self::PushError>;
    fn set_origin(&self, branch: &str, origin_name: &str) -> Result<(), Self::SetOriginError>;

    fn get_origin(&self) -> Result<OriginInfo, Self::GetOriginError>;

    fn workdir(&self) -> Option<&Path>;
    fn path(&self) -> &Path;
}

#[derive(Error, Debug)]
pub enum GitCommandError {
    #[error("Failed to run git: {0}")]
    Command(#[from] std::io::Error),
    #[error("Git failed with: [exit code {0}]\n  stdout: {1}\n  stderr: {2}")]
    BadExit(i32, String, String),
}

/// Configuration options for the git command
///
/// Used by [GitCommandProvider] to create commands with consistent options.
#[derive(Clone, Debug, PartialEq)]
pub struct GitCommandOptions {
    exe: String,
    config: BTreeMap<String, String>,
    envs: BTreeMap<String, String>,
}

impl Default for GitCommandOptions {
    /// By default, use the git binary bundled with flox
    fn default() -> Self {
        Self {
            exe: String::from(env!("GIT_BIN")),
            config: Default::default(),
            envs: Default::default(),
        }
    }
}

/// Modifying options for the git command
///
/// Custom abstractions can be added on top of this through extension traits or functions.
impl GitCommandOptions {
    /// Create a new set of options with default values using the bundled git binary
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the git binary to use
    pub fn set_exe<E: AsRef<str>>(&mut self, exe: E) {
        self.exe = exe.as_ref().to_string();
    }

    /// set a git config flag that is passed to git
    pub fn add_config_flag<V: AsRef<str>>(&mut self, key: &str, value: V) {
        self.config
            .insert(key.to_string(), value.as_ref().to_string());
    }

    /// set an environment variable that is passed to git
    pub fn add_env_var<V: AsRef<str>>(&mut self, var: &str, value: V) {
        self.envs
            .insert(var.to_string(), value.as_ref().to_string());
    }

    /// Create a new [Command] with the current options prepopulated
    ///
    /// For all configuration flags the arguments `-c <flag>=<value>` are added.
    /// All env vars are set on the command.
    pub fn new_command(&self) -> Command {
        let mut c = Command::new(&self.exe);

        for (flag, value) in &self.config {
            c.arg("-c");
            c.arg(format!("{}={}", flag, value));
        }

        for (var, value) in &self.envs {
            c.env(var, value);
        }

        c
    }
}

/// A representation of a git repository using the `git` CLI
#[derive(Clone, Debug, PartialEq)]
pub struct GitCommandProvider {
    options: GitCommandOptions,
    workdir: Option<PathBuf>,
    path: PathBuf,
}

impl GitCommandProvider {
    /// Create a new [Command] with the current [GitCommandOptions]
    /// and the current working directory set to the path of the repo.
    ///
    /// In most cases this should be used over [GitCommandProvider::new_command]
    pub fn new_command(&self) -> Command {
        let mut command = self.options.new_command();
        command.args(["-C", self.path.to_str().unwrap()]);
        command
    }

    fn run_command(command: &mut Command) -> Result<OsString, GitCommandError> {
        debug!("{:?}", command);
        let out = command.output()?;

        if !out.status.success() {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();

            return Err(GitCommandError::BadExit(
                out.status.code().unwrap_or(-1),
                stdout,
                stderr,
            ));
        }

        Ok(OsString::from_vec(out.stdout))
    }

    pub fn init_with<P: AsRef<Path>>(
        options: GitCommandOptions,
        path: P,
        bare: bool,
    ) -> Result<GitCommandProvider, GitCommandError> {
        let mut command = options.new_command();
        command.args(["-C", path.as_ref().to_str().unwrap()]);

        command.arg("init");
        if bare {
            command.arg("--bare");
        }

        let _out = GitCommandProvider::run_command(&mut command)?;

        Ok(GitCommandProvider {
            options,
            workdir: Some(path.as_ref().into()),
            path: path.as_ref().into(),
        })
    }

    /// Check if repo is bare. This will error if not in a git repo.
    fn is_bare_repo(path: impl AsRef<Path>) -> Result<bool, GitCommandDiscoverError> {
        let mut command = GitCommandOptions::default().new_command();
        command
            .args(["-C", path.as_ref().to_str().unwrap()])
            .arg("rev-parse")
            .arg("--is-bare-repository");

        let out_str = GitCommandProvider::run_command(&mut command)?
            .to_str()
            .ok_or(GitCommandDiscoverError::GitDirEncoding)?
            .to_string();

        let bare = out_str
            .trim()
            .parse::<bool>()
            .map_err(|_| GitCommandDiscoverError::UnexpectedOutput(out_str))?;
        Ok(bare)
    }

    /// Open a repo, erroring if `path` is not a repo or is a subdirectory of a repo
    pub fn open_with<P: AsRef<Path>>(
        options: GitCommandOptions,
        path: P,
    ) -> Result<Self, GitCommandOpenError> {
        debug!("attempting to open repo: path={}", path.as_ref().display());
        let bare = Self::is_bare_repo(&path)?;

        // resolved and canonicalized path to the git repo
        debug!("determining path to git repo");
        let resolved_path = {
            let toplevel_or_git_dir = if bare {
                let mut command = options.new_command();

                command
                    .args(["-C", path.as_ref().to_str().unwrap()])
                    .arg("rev-parse")
                    .arg("--absolute-git-dir");
                GitCommandProvider::run_command(&mut command)?
            } else {
                let mut command = options.new_command();
                command
                    .args(["-C", path.as_ref().to_str().unwrap()])
                    .arg("rev-parse")
                    .arg("--show-toplevel");
                GitCommandProvider::run_command(&mut command)?
            };

            let toplevel_or_git_dir = toplevel_or_git_dir
                .to_str()
                .ok_or(GitCommandDiscoverError::GitDirEncoding)?
                .trim();

            PathBuf::from(toplevel_or_git_dir)
                .canonicalize()
                .map_err(GitCommandOpenError::Canonicalize)?
        };
        debug!("got non-canonical path: path={}", resolved_path.display());

        let path = path
            .as_ref()
            .canonicalize()
            .map_err(GitCommandOpenError::Canonicalize)?;

        if resolved_path != path {
            return Err(GitCommandOpenError::Subdirectory);
        }
        debug!("canonicalized path: path={}", path.display());

        Ok(GitCommandProvider {
            options,
            workdir: (!bare).then(|| path.clone()),
            path,
        })
    }

    /// Open a repo with default options,
    /// erroring if `path` is not a repo or is a subdirectory of a repo
    //
    // TODO should share more code with discover?
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, GitCommandOpenError> {
        GitCommandProvider::open_with(GitCommandOptions::default(), path)
    }

    /// Checks if the specified revision identifies a commit in the repo
    pub fn contains_commit(&self, rev: &str) -> Result<bool, GitCommandError> {
        let result = GitCommandProvider::run_command(
            self.new_command()
                .arg("rev-parse")
                .arg("--quiet")
                .arg("--verify")
                .arg(format!("{}^{{commit}}", rev)),
        );
        match result {
            Ok(_) => Ok(true),
            Err(GitCommandError::BadExit(_, stdout, stderr))
                if stdout.is_empty() && stderr.is_empty() =>
            {
                Ok(false)
            },
            Err(e) => Err(e),
        }
    }

    /// Check if commit exists and is part of the branch's history
    pub fn branch_contains_commit(
        &self,
        commit: &str,
        branch: &str,
    ) -> Result<bool, GitCommandError> {
        if !self.contains_commit(commit)? {
            return Ok(false);
        }

        let result = GitCommandProvider::run_command(
            self.new_command()
                .arg("merge-base")
                .arg("--is-ancestor")
                .arg(commit)
                .arg(branch),
        );
        match result {
            Ok(_) => Ok(true),
            Err(GitCommandError::BadExit(_, stdout, stderr))
                if stdout.is_empty() && stderr.is_empty() =>
            {
                Ok(false)
            },
            Err(e) => Err(e),
        }
    }

    /// Create branch at a specified revision
    pub fn create_branch(&self, name: &str, rev: &str) -> Result<(), GitCommandError> {
        GitCommandProvider::run_command(self.new_command().arg("branch").arg(name).arg(rev))?;
        Ok(())
    }

    /// Reset branch to rev or create it if it does not exist
    pub fn reset_branch(&self, name: &str, rev: &str) -> Result<(), GitCommandError> {
        GitCommandProvider::run_command(
            self.new_command()
                .arg("branch")
                .arg("--force")
                .arg(name)
                .arg(rev),
        )?;
        Ok(())
    }

    /// Return the hash of a branch or error if it does not exist
    pub fn branch_hash(&self, name: &str) -> Result<String, GitCommandBranchHashError> {
        let result = GitCommandProvider::run_command(
            self.new_command()
                .arg("show-ref")
                .arg("--hash")
                .arg(format!("refs/heads/{}", name)),
        );
        match result {
            Ok(hash) => hash
                .to_str()
                .ok_or(GitCommandBranchHashError::HashNotUnicode)
                .map(|hash| hash.trim().to_string()),
            Err(GitCommandError::BadExit(1, stdout, stderr))
                if stdout.is_empty() && stderr.is_empty() =>
            {
                Err(GitCommandBranchHashError::DoesNotExist)
            },
            Err(e) => Err(e.into()),
        }
    }

    pub fn has_branch(&self, name: &str) -> Result<bool, GitCommandBranchHashError> {
        match self.branch_hash(name) {
            Ok(_) => Ok(true),
            Err(GitCommandBranchHashError::DoesNotExist) => Ok(false),
            Err(err) => Err(err),
        }
    }

    /// Clone a branch from a remote repository
    pub fn clone_branch_with(
        options: GitCommandOptions,
        origin: impl AsRef<OsStr>,
        path: impl AsRef<Path>,
        branch: impl AsRef<OsStr>,
        bare: bool,
    ) -> Result<GitCommandProvider, GitRemoteCommandError> {
        let mut command = options.new_command();

        command
            .arg("clone")
            .arg("--single-branch")
            .arg("--no-tags")
            .arg("--branch")
            .arg(branch)
            .arg(origin)
            .arg(path.as_ref());

        if bare {
            command.arg("--bare");
        }
        GitCommandProvider::run_command(&mut command)?;

        Ok(GitCommandProvider {
            options,
            workdir: (!bare).then(|| path.as_ref().to_path_buf()),
            path: path.as_ref().into(),
        })
    }

    /// Clone a branch from a remote repository using default options
    pub fn clone_branch(
        origin: impl AsRef<OsStr>,
        path: impl AsRef<Path>,
        branch: &str,
        bare: bool,
    ) -> Result<GitCommandProvider, GitRemoteCommandError> {
        Self::clone_branch_with(GitCommandOptions::default(), origin, path, branch, bare)
    }

    /// Fetch branch and update the corresponding local ref
    pub fn fetch_branch(
        &self,
        repository: &str,
        branch: &str,
    ) -> Result<(), GitRemoteCommandError> {
        self.fetch_ref(
            repository,
            &format!("refs/heads/{branch}:refs/heads/{branch}"),
        )
    }

    pub fn fetch_ref(&self, repository: &str, r#ref: &str) -> Result<(), GitRemoteCommandError> {
        GitCommandProvider::run_command(
            self.new_command().arg("fetch").arg(repository).arg(r#ref),
        )?;
        Ok(())
    }

    /// Like [GitCommandProvider::push] but allows to specify the refspec explicitly
    pub fn push_ref(
        &self,
        repository: impl AsRef<str>,
        push_spec: impl AsRef<str>,
        force: bool,
    ) -> Result<(), GitRemoteCommandError> {
        let mut command = self.new_command();
        command
            .arg("push")
            .arg("--porcelain")
            .arg(repository.as_ref())
            .arg(push_spec.as_ref());

        if force {
            command.arg("--force");
        }

        match GitCommandProvider::run_command(&mut command) {
            Ok(_) => Ok(()),
            Err(ref err @ GitCommandError::BadExit(_, _, ref stderr))
                if stderr.contains("DENIED") || stderr.contains("Authentication failed") =>
            {
                debug!("Access denied: {err}");
                Err(GitRemoteCommandError::AccessDenied)
            },
            Err(ref err @ GitCommandError::BadExit(_, ref stdout, _))
                if stdout.contains("[rejected] (fetch first)") =>
            {
                debug!("Branches diverged: {err}");
                Err(GitRemoteCommandError::Diverged)
            },
            Err(e) => Err(e.into()),
        }?;
        Ok(())
    }

    /// Deletes the specified branch
    pub fn delete_branch(&self, branch: &str, force: bool) -> Result<(), GitCommandError> {
        let mut command = {
            let mut command = self.new_command();
            command.arg("branch");
            command.arg("--delete");
            if force {
                command.arg("--force");
            }
            command.arg(branch);
            command
        };
        GitCommandProvider::run_command(&mut command)?;
        Ok(())
    }

    /// Update the options used by this provider.
    ///
    /// It is preferable to set the options when creating the provider
    /// via [GitCommandProvider::open_with] or [GitCommandProvider::clone_branch_with].
    pub fn set_options(&mut self, options: GitCommandOptions) {
        self.options = options;
    }

    /// Get the options used by this provider
    ///
    /// This can be used to create a new provider with the same options
    /// or modify the options of this provider.
    pub fn get_options(&self) -> &GitCommandOptions {
        &self.options
    }

    /// Get a mutable reference to the options used by this provider
    ///
    /// This can be used to create a new provider with the same options
    /// or modify the options of this provider without cloning.
    pub fn get_options_mut(&mut self) -> &mut GitCommandOptions {
        &mut self.options
    }
}

#[derive(Error, Debug)]
pub enum GitCommandDiscoverError {
    #[error(transparent)]
    Command(#[from] GitCommandError),
    #[error("Git directory is not valid unicode")]
    GitDirEncoding,
    #[error("Git returned an uexpected output: {0}")]
    UnexpectedOutput(String),
}

#[derive(Error, Debug)]
pub enum GitCommandOpenError {
    #[error(transparent)]
    Command(#[from] GitCommandError),
    #[error(transparent)]
    Discover(#[from] GitCommandDiscoverError),
    #[error("Path is subdirectory of a git repository")]
    Subdirectory,
    #[error("Could not canonicalize path: {0}")]
    Canonicalize(std::io::Error),
}

#[derive(Error, Debug)]
pub enum GitCommandGetOriginError {
    #[error(transparent)]
    Command(#[from] GitCommandError),
    #[error("Couldn't determine upstream remote name for the current HEAD")]
    NoUpstream,
}

#[derive(Error, Debug)]
pub enum GitCommandBranchHashError {
    #[error(transparent)]
    Command(#[from] GitCommandError),
    #[error("Could not convert hash to unicode")]
    HashNotUnicode,
    #[error("Branch does not exist")]
    DoesNotExist,
}

impl GitDiscoverError for GitCommandDiscoverError {
    fn not_found(&self) -> bool {
        match self {
            // TODO: handle errors
            GitCommandDiscoverError::Command(_) => true,
            _ => false,
        }
    }
}

#[derive(Error, Debug)]
pub enum GitRemoteCommandError {
    #[error(transparent)]
    Command(GitCommandError),
    #[error("access denied")]
    AccessDenied,
    #[error("branches diverged")]
    Diverged,
    #[error("ref not found")]
    RefNotFound(String),
}

const REF_NOT_FOUND_ERR_PREFIX: &str = "fatal: couldn't find remote ref ";
impl From<GitCommandError> for GitRemoteCommandError {
    fn from(err: GitCommandError) -> Self {
        match err {
            GitCommandError::BadExit(_, _, ref stderr)
                if stderr.contains("DENIED") || stderr.contains("Authentication failed") =>
            {
                debug!("Access denied: {err}");
                GitRemoteCommandError::AccessDenied
            },
            GitCommandError::BadExit(_, ref stdout, _)
                if stdout.contains("[rejected] (fetch first)") =>
            {
                debug!("Branches diverged: {err}");
                GitRemoteCommandError::Diverged
            },
            GitCommandError::BadExit(_, _, ref stderr)
                if stderr.starts_with(REF_NOT_FOUND_ERR_PREFIX) =>
            {
                let ref_name = stderr.strip_prefix(REF_NOT_FOUND_ERR_PREFIX).unwrap();
                debug!("Ref not found: {ref_name}");
                GitRemoteCommandError::RefNotFound(ref_name.to_string())
            },
            e => GitRemoteCommandError::Command(e),
        }
    }
}

/// A simple Git Provider that uses the git
/// command. This would require that git is installed.
impl GitProvider for GitCommandProvider {
    type AddError = GitCommandError;
    type AddRemoteError = GitCommandError;
    type CheckoutError = GitCommandError;
    type CloneError = GitRemoteCommandError;
    type CommitError = GitCommandError;
    type DiscoverError = GitCommandDiscoverError;
    type FetchError = GitRemoteCommandError;
    type GetOriginError = GitCommandGetOriginError;
    type InitError = GitCommandError;
    type ListBranchesError = GitCommandError;
    type MvError = GitCommandError;
    type PushError = GitRemoteCommandError;
    type RenameError = GitCommandError;
    type RmError = GitCommandError;
    type SetOriginError = GitCommandError;
    type ShowError = GitCommandError;

    /// Discover a git repository at `path` and return a provider with default options
    ///
    /// Use DiscoverError::not_found() to check if the path is not a git repo.
    fn discover<P: AsRef<Path>>(path: P) -> Result<Self, Self::DiscoverError> {
        let options = GitCommandOptions::default();
        let bare = Self::is_bare_repo(&path)?;

        if bare {
            return Ok(GitCommandProvider {
                options: GitCommandOptions::default(),
                workdir: None,
                path: path.as_ref().to_path_buf(),
            });
        }

        let out = GitCommandProvider::run_command(
            options
                .new_command()
                .current_dir(&path)
                .arg("rev-parse")
                .arg("--show-toplevel"),
        )?;

        let out_str = out
            .to_str()
            .ok_or(GitCommandDiscoverError::GitDirEncoding)?;

        let workdir = PathBuf::from(out_str.trim());

        Ok(GitCommandProvider {
            options,
            workdir: Some(workdir.clone()),
            path: workdir,
        })
    }

    fn init<P: AsRef<Path>>(path: P, bare: bool) -> Result<GitCommandProvider, Self::InitError> {
        let options = GitCommandOptions::default();
        Self::init_with(options, path, bare)
    }

    fn clone<O: AsRef<OsStr>, P: AsRef<Path>>(
        origin: O,
        path: P,
        bare: bool,
    ) -> Result<Self, Self::CloneError> {
        let options = GitCommandOptions::default();
        let mut command = options.new_command();
        command.current_dir(&path);
        command.arg("clone");
        if bare {
            command.arg("--bare");
        }

        command.arg(origin.as_ref());
        command.arg("./");

        let _out = GitCommandProvider::run_command(&mut command)?;
        Ok(GitCommandProvider {
            options,
            workdir: (!bare).then(|| path.as_ref().to_path_buf()),
            path: path.as_ref().into(),
        })
    }

    fn checkout(&self, name: &str, orphan: bool) -> Result<(), Self::CheckoutError> {
        let mut command = self.new_command();
        command.arg("checkout");
        if orphan {
            command.arg("--orphan");
        }

        command.arg(name);

        let _out = GitCommandProvider::run_command(&mut command)?;
        Ok(())
    }

    fn add_remote(&self, origin_name: &str, url: &str) -> Result<(), Self::AddRemoteError> {
        let _out = GitCommandProvider::run_command(
            self.new_command()
                .arg("remote")
                .arg("add")
                .arg(origin_name)
                .arg(url),
        )?;

        Ok(())
    }

    fn rename_branch(&self, new_name: &str) -> Result<(), Self::RenameError> {
        let _out = GitCommandProvider::run_command(
            self.new_command().arg("branch").arg("-m").arg(new_name),
        )?;
        Ok(())
    }

    fn set_origin(&self, branch: &str, origin_name: &str) -> Result<(), Self::SetOriginError> {
        let _out = GitCommandProvider::run_command(
            self.new_command()
                .arg("branch")
                .arg("--set-upstream-to")
                .arg(format!("{origin_name}/{branch}")),
        )?;

        Ok(())
    }

    /// Retrieve information about the remote origin for the current branch/repo
    ///
    /// Return a tuple containing
    ///
    /// 1. the remote name of the current branch
    /// 2. the remote url
    /// 3. the upstream branch name
    /// 4. the current revision of the upstream branch
    ///
    /// This is essentialy
    ///
    ///   upstream_ref = git rev-parse @{u}
    ///   (remote_name, branch_name) = split_once "/" upstream_ref
    ///   upstream_url = git remote get-url ${remote_name}
    ///   upstream_rev = git ls-remote ${remote_name} ${branch_name}
    fn get_origin(&self) -> Result<OriginInfo, Self::GetOriginError> {
        let (remote_name, remote_branch) = {
            let reference = GitCommandProvider::run_command(
                self.new_command()
                    .arg("rev-parse")
                    .arg("--abbrev-ref")
                    .arg("--symbolic-full-name")
                    .arg("@{u}"),
            )
            .map_err(|_| GitCommandGetOriginError::NoUpstream)?;
            let as_str = reference.to_string_lossy();
            let (remote_name, remote_branch) = as_str.trim().split_once('/').unwrap();
            (remote_name.to_string(), remote_branch.to_string())
        };

        let url = GitCommandProvider::run_command(
            self.new_command()
                .arg("remote")
                .arg("get-url")
                .arg(&remote_name),
        )?
        .to_string_lossy()
        .trim()
        .to_string();

        let remote_revision = {
            let remote_revision = GitCommandProvider::run_command(
                self.new_command()
                    .arg("ls-remote")
                    .arg(&remote_name)
                    .arg(&remote_branch),
            )?;

            let remote_revision = if remote_revision.len() < 40 {
                warn!("No commit found found upstream for ref {remote_branch}");
                None
            } else {
                Some(remote_revision.to_string_lossy()[..40].to_string())
            };
            remote_revision
        };

        Ok(OriginInfo {
            name: remote_name,
            url,
            reference: remote_branch,
            revision: remote_revision,
        })
    }

    fn mv(&self, from: &Path, to: &Path) -> Result<(), Self::MvError> {
        let _out = GitCommandProvider::run_command(
            self.new_command()
                .arg("mv")
                .arg(format!("{}", from.as_os_str().to_string_lossy()))
                .arg(format!("{}", to.as_os_str().to_string_lossy())),
        )?;

        Ok(())
    }

    fn rm(
        &self,
        paths: &[&Path],
        recursive: bool,
        force: bool,
        cached: bool,
    ) -> Result<(), Self::MvError> {
        let mut command = self.new_command();

        command.arg("rm");

        if recursive {
            command.arg("-r");
        }
        if force {
            command.arg("--force");
        }
        if cached {
            command.arg("--cached");
        }

        for path in paths {
            command.arg(format!("{}", path.as_os_str().to_string_lossy()));
        }

        let _out = GitCommandProvider::run_command(&mut command)?;

        Ok(())
    }

    fn add(&self, paths: &[&Path]) -> Result<(), Self::MvError> {
        let mut command = self.new_command();
        command.arg("add");
        for path in paths {
            command.arg(path);
        }

        let _out = GitCommandProvider::run_command(&mut command)?;

        Ok(())
    }

    fn commit(&self, message: &str) -> Result<(), Self::CommitError> {
        let mut command = self.new_command();
        command.arg("commit");
        command.args(["-m", message]);

        let _out = GitCommandProvider::run_command(&mut command)?;
        Ok(())
    }

    fn show(&self, object: &str) -> Result<OsString, Self::ShowError> {
        let mut command = self.new_command();
        command.arg("show");
        command.arg(object);

        GitCommandProvider::run_command(&mut command)
    }

    fn list_branches(&self) -> Result<Vec<BranchInfo>, Self::ListBranchesError> {
        let mut command = self.new_command();
        command.arg("branch");
        command.args(["--all", "--verbose"]);

        let info = GitCommandProvider::run_command(&mut command)?
            .to_string_lossy()
            .lines()
            .map(|line| {
                // split all lines into three parts (undoing git's default format)
                // using the `--format` option failed on me to produce any useful output at all
                // If using the git cli that would probably be the better way
                // of providing parseable data.
                //
                // the git putput is formatted as
                // [*] <name> <whitespace> <rev hash> <whitespace> <subject>
                //  L present iff branch is currently checked out

                // the active branch is denoted by a leadinf '*', which cannot be disabled?
                let (full_name, rest) =
                    line.trim_start_matches('*').trim().split_once(' ').unwrap();
                // hash part
                let (hash, rest) = rest.trim().split_once(' ').unwrap();
                // description
                let description = rest.trim_start();
                (full_name, hash, description)
            })
            .map(|(full_name, hash, description)| {
                // discard unknown remotes
                let (remote, name) = match full_name
                    .strip_prefix("remotes/")
                    .and_then(|remote| remote.split_once('/'))
                {
                    Some((remote, name)) => (Some(remote), name),
                    None => (None, full_name),
                };
                BranchInfo {
                    name: name.to_string(),
                    remote: remote.map(String::from),
                    rev: hash.to_string(),
                    description: description.to_string(),
                }
            })
            .collect();

        Ok(info)
    }

    fn fetch(&self) -> Result<(), Self::FetchError> {
        GitCommandProvider::run_command(self.new_command().arg("fetch").arg("--all"))?;
        Ok(())
    }

    fn push(&self, remote: &str, force: bool) -> Result<(), Self::PushError> {
        let mut command = self.new_command();
        command.arg("push");
        command.arg("--porcelain");
        command.arg("-u");
        command.arg(remote);
        command.arg("HEAD");

        if force {
            command.arg("--force");
        }

        let _out = GitCommandProvider::run_command(&mut command)?;
        Ok(())
    }

    fn workdir(&self) -> Option<&Path> {
        self.workdir.as_ref().map(|x| x.as_ref())
    }

    fn path(&self) -> &Path {
        self.path.as_ref()
    }
}

#[cfg(test)]
pub mod tests {

    use std::fs;

    use pretty_assertions::assert_eq;

    use super::*;

    /// A provider with path set to /does-not-exist for use in tests
    pub fn mock_provider() -> GitCommandProvider {
        GitCommandProvider {
            options: GitCommandOptions::default(),
            workdir: None,
            path: PathBuf::from("/does-not-exist"),
        }
    }

    fn init_temp_repo(bare: bool) -> (GitCommandProvider, tempfile::TempDir) {
        let tempdir_handle = tempfile::tempdir_in(std::env::temp_dir()).unwrap();

        let git_command_provider = GitCommandProvider::init(tempdir_handle.path(), bare).unwrap();
        (git_command_provider, tempdir_handle)
    }

    pub fn commit_file(repo: &GitCommandProvider, filename: &str) {
        let file = repo.path.join(filename);
        fs::write(&file, filename).unwrap();
        repo.add(&[&file]).unwrap();
        repo.commit(filename).unwrap();
    }

    #[test]
    fn discover() {
        let (_, tempdir_handle) = init_temp_repo(false);
        let path = tempdir_handle.path().canonicalize().unwrap();
        assert_eq!(
            GitCommandProvider::discover(&path).unwrap(),
            GitCommandProvider {
                options: GitCommandOptions::default(),
                workdir: Some(path.clone()),
                path
            }
        );
    }

    #[test]
    fn discover_subdirectory() {
        let (_, tempdir_handle) = init_temp_repo(false);
        let path = tempdir_handle.path().canonicalize().unwrap();
        let subdirectory = path.join("subdirectory");
        std::fs::create_dir(&subdirectory).unwrap();
        assert_eq!(
            GitCommandProvider::discover(&subdirectory).unwrap(),
            GitCommandProvider {
                options: GitCommandOptions::default(),
                workdir: Some(path.clone()),
                path
            }
        );
    }

    #[test]
    fn discover_bare() {
        let (_, tempdir_handle) = init_temp_repo(true);
        let path = tempdir_handle.path().to_path_buf();
        assert_eq!(
            GitCommandProvider::discover(&path).unwrap(),
            GitCommandProvider {
                options: GitCommandOptions::default(),
                workdir: None,
                path
            }
        );
    }

    #[test]
    fn discover_not_git_repo() {
        let tempdir_handle = tempfile::tempdir_in(std::env::temp_dir()).unwrap();
        let path = tempdir_handle.path().to_path_buf();
        assert!(GitCommandProvider::discover(path)
            .err()
            .unwrap()
            .not_found());
    }

    #[test]
    fn test_open() {
        let (_, tempdir_handle) = init_temp_repo(false);
        let path = tempdir_handle.path().to_path_buf();
        assert_eq!(
            GitCommandProvider::open(&path).unwrap(),
            GitCommandProvider {
                options: GitCommandOptions::default(),
                workdir: Some(path.canonicalize().unwrap()),
                path: path.canonicalize().unwrap()
            }
        );
    }

    // test opening a bare repo succeeds
    #[test]
    fn test_open_bare() {
        let (_, tempdir_handle) = init_temp_repo(true);
        let path = tempdir_handle.path().to_path_buf();
        assert_eq!(
            GitCommandProvider::open(&path).unwrap(),
            GitCommandProvider {
                options: GitCommandOptions::default(),
                workdir: None,
                path: path.canonicalize().unwrap()
            }
        );
    }

    // test opening a subdirectory of a repo fails
    #[test]
    fn test_open_subdirectory() {
        let (_, tempdir_handle) = init_temp_repo(false);
        let path = tempdir_handle.path().to_path_buf();

        let subdirectory = path.join("subdirectory");
        std::fs::create_dir(&subdirectory).unwrap();

        assert!(matches!(
            GitCommandProvider::open(subdirectory),
            Err(GitCommandOpenError::Subdirectory),
        ));
    }

    // test opening a subdirectory of a bare repo fails
    #[test]
    fn test_open_subdirectory_bare() {
        let (_, tempdir_handle) = init_temp_repo(true);

        assert!(matches!(
            GitCommandProvider::open(tempdir_handle.path().join("branches")),
            Err(GitCommandOpenError::Subdirectory),
        ));
    }

    #[test]
    fn test_open_nonexistent() {
        let a = GitCommandProvider::open(PathBuf::from("/does-not-exist"));
        println!("{:?}", a);
        assert!(matches!(
            GitCommandProvider::open(PathBuf::from("/does-not-exist")),
            Err(GitCommandOpenError::Discover(
                GitCommandDiscoverError::Command(GitCommandError::BadExit(128, _, _))
            )),
        ));
    }

    #[test]
    fn test_branch_contains_commit() {
        let (repo, _tempdir_handle) = init_temp_repo(false);
        repo.checkout("branch_1", true).unwrap();
        commit_file(&repo, "dummy");
        let hash_1 = repo.branch_hash("branch_1").unwrap();
        commit_file(&repo, "dummy_2");
        let hash_2 = repo.branch_hash("branch_1").unwrap();

        assert_ne!(repo.branch_hash("branch_1").unwrap(), hash_1);
        assert!(repo.branch_contains_commit(&hash_1, "branch_1").unwrap());
        assert!(repo.branch_contains_commit(&hash_2, "branch_1").unwrap());
    }

    #[test]
    fn test_commit_not_on_branch() {
        let (repo, _tempdir_handle) = init_temp_repo(false);
        repo.checkout("branch_1", true).unwrap();
        commit_file(&repo, "dummy");
        let hash_1 = repo.branch_hash("branch_1").unwrap();

        repo.checkout("branch_2", true).unwrap();
        commit_file(&repo, "dummy_2");

        assert!(!repo.branch_contains_commit(&hash_1, "branch_2").unwrap());
    }

    #[test]
    fn test_commit_not_on_any_branch() {
        let (repo, _tempdir_handle) = init_temp_repo(false);
        repo.checkout("branch_1", true).unwrap();
        commit_file(&repo, "dummy");

        assert!(!repo.branch_contains_commit("XXX", "branch_1").unwrap());
    }

    #[test]
    fn test_branch_hash() {
        let (repo, _tempdir_handle) = init_temp_repo(false);
        repo.checkout("branch_1", true).unwrap();

        commit_file(&repo, "dummy");

        assert!(repo.branch_hash("branch_1").unwrap().len() == 40);
    }

    #[test]
    fn test_branch_does_not_exist() {
        let (repo, _tempdir_handle) = init_temp_repo(false);

        assert!(!repo.has_branch("branch_1").unwrap());
    }

    #[test]
    fn test_create_branch() {
        let (repo, _tempdir_handle) = init_temp_repo(false);
        repo.checkout("branch_1", true).unwrap();
        commit_file(&repo, "dummy");
        let hash = repo.branch_hash("branch_1").unwrap();

        repo.create_branch("test", &hash).unwrap();
        assert_eq!(repo.branch_hash("test").unwrap(), hash)
    }

    // test that clone_branch only clones the specified branch
    #[test]
    fn test_clone_branch() {
        // create two branches in repo: branch_1 and branch_2
        let (repo, _tempdir_handle) = init_temp_repo(false);
        repo.checkout("branch_1", true).unwrap();
        commit_file(&repo, "dummy");
        let hash_branch_1 = repo.branch_hash("branch_1").unwrap();

        repo.checkout("branch_2", true).unwrap();
        commit_file(&repo, "dummy_2");
        let hash_branch_2 = repo.branch_hash("branch_2").unwrap();

        // clone only branch_1 branch to repo_2
        let tempdir_handle_2 = tempfile::tempdir_in(std::env::temp_dir()).unwrap();
        // Specify file:// so that extra commits aren't copied
        // "If you specify file://, Git fires up the processes that it normally
        // uses to transfer data over a network"
        // https://git-scm.com/book/en/v2/Git-on-the-Server-The-Protocols
        let repo_2 = GitCommandProvider::clone_branch(
            format!("file://{}", &repo.path.to_str().unwrap()),
            tempdir_handle_2.path(),
            "branch_1",
            true,
        )
        .unwrap();

        // assert repo_2 has branch_1 branch with the correct hash, but does not have
        // branch_2 or the commit on branch_2
        assert_eq!(repo_2.branch_hash("branch_1").unwrap(), hash_branch_1);
        assert!(!repo_2.has_branch("branch_2").unwrap());
        assert!(!repo_2.contains_commit(&hash_branch_2).unwrap());
    }

    #[test]
    fn test_fetch_branch() {
        // create three branches in repo: branch_1, branch_2, and branch_3
        let (repo, _tempdir_handle) = init_temp_repo(false);
        repo.checkout("branch_1", true).unwrap();
        commit_file(&repo, "dummy");
        let hash_branch_1 = repo.branch_hash("branch_1").unwrap();

        repo.checkout("branch_2", true).unwrap();
        commit_file(&repo, "dummy_2");
        let hash_branch_2 = repo.branch_hash("branch_2").unwrap();

        repo.checkout("branch_3", true).unwrap();
        commit_file(&repo, "dummy_3");
        let hash_branch_3 = repo.branch_hash("branch_3").unwrap();

        // clone only branch_1 branch to repo_2
        let tempdir_handle_2 = tempfile::tempdir_in(std::env::temp_dir()).unwrap();
        // Specify file:// so that extra commits aren't copied
        let repo_2 = GitCommandProvider::clone_branch(
            format!("file://{}", &repo.path.to_str().unwrap()),
            tempdir_handle_2.path(),
            "branch_1",
            false,
        )
        .unwrap();

        // repo_2 has branch_1 but not the commit on branch_2
        assert_eq!(repo_2.branch_hash("branch_1").unwrap(), hash_branch_1);
        assert!(!repo_2.contains_commit(&hash_branch_2).unwrap());

        // fetch branch_2
        repo_2.fetch_branch("origin", "branch_2").unwrap();
        assert_eq!(repo_2.branch_hash("branch_2").unwrap(), hash_branch_2);
        // repo_2 has branch_2 but not the commit on branch_3
        assert_eq!(repo_2.branch_hash("branch_2").unwrap(), hash_branch_2);
        assert!(!repo_2.contains_commit(&hash_branch_3).unwrap());
    }

    #[test]
    fn test_fetch_ref() {
        // create three branches in repo: branch_1, branch_2, and branch_3
        let (repo, _tempdir_handle) = init_temp_repo(false);
        repo.checkout("branch_1", true).unwrap();
        commit_file(&repo, "dummy");

        repo.checkout("branch_2", true).unwrap();
        commit_file(&repo, "dummy_2");
        let hash_branch_2 = repo.branch_hash("branch_2").unwrap();

        repo.checkout("branch_3", true).unwrap();
        commit_file(&repo, "dummy_3");
        let hash_branch_3 = repo.branch_hash("branch_3").unwrap();

        // clone only branch_1 to repo_2
        let tempdir_handle_2 = tempfile::tempdir_in(std::env::temp_dir()).unwrap();
        // Specify file:// so that extra commits aren't copied
        let repo_2 = GitCommandProvider::clone_branch(
            format!("file://{}", &repo.path.to_str().unwrap()),
            tempdir_handle_2.path(),
            "branch_1",
            false,
        )
        .unwrap();

        assert!(!repo_2.contains_commit(&hash_branch_2).unwrap());
        repo_2.fetch_ref("origin", &hash_branch_2).unwrap();
        assert!(repo_2.contains_commit(&hash_branch_2).unwrap());
        assert!(!repo_2.contains_commit(&hash_branch_3).unwrap());
    }

    #[test]
    fn test_fetch_bad_ref() {
        let (repo, _tempdir_handle) = init_temp_repo(false);
        repo.checkout("branch_1", true).unwrap();
        commit_file(&repo, "dummy");

        let tempdir_handle_2 = tempfile::tempdir_in(std::env::temp_dir()).unwrap();
        let repo_2 =
            GitCommandProvider::clone_branch(&repo.path, tempdir_handle_2.path(), "branch_1", true)
                .unwrap();

        assert!(matches!(
            repo_2.fetch_ref("origin", "does-not-exist"),
            Err(GitRemoteCommandError::RefNotFound(_))
        ));
    }

    #[test]
    fn test_reset_branch_existing() {
        // create two branches in repo: branch_1 and branch_2
        let (repo, _tempdir_handle) = init_temp_repo(false);
        repo.checkout("branch_1", true).unwrap();
        commit_file(&repo, "dummy");

        repo.checkout("branch_2", true).unwrap();
        commit_file(&repo, "dummy_2");
        let hash_branch_2 = repo.branch_hash("branch_2").unwrap();

        // reset branch_1 to branch_2
        assert_ne!(repo.branch_hash("branch_1").unwrap(), hash_branch_2);
        repo.reset_branch("branch_1", &hash_branch_2).unwrap();
        assert_eq!(repo.branch_hash("branch_1").unwrap(), hash_branch_2)
    }

    #[test]
    fn test_reset_branch_new() {
        // create two branches in repo: branch_1 and branch_2
        let (repo, _tempdir_handle) = init_temp_repo(false);
        repo.checkout("branch_1", true).unwrap();
        commit_file(&repo, "dummy");

        repo.checkout("branch_2", true).unwrap();
        commit_file(&repo, "dummy_2");
        let hash_branch_2 = repo.branch_hash("branch_2").unwrap();

        // reset branch_1 to branch_2
        repo.reset_branch("branch_3", &hash_branch_2).unwrap();
        assert_eq!(repo.branch_hash("branch_3").unwrap(), hash_branch_2)
    }

    /// Test that we pushing to a read only repo fails with [GitRemoteCommandError::AccessDenied]
    #[test]
    fn test_push_access_denied() {
        let (mut repo, _tempdir_handle) = init_temp_repo(false);
        repo.add_remote("origin", "https://github.com/torvalds/linux")
            .unwrap();

        {
            let options = repo.get_options_mut();
            options.add_env_var("GIT_CONFIG_SYSTEM", "/dev/null");
            options.add_env_var("GIT_CONFIG_GLOBAL", "/dev/null");
            options.add_config_flag(
                "credential.helper",
                r#"!f(){ echo "username="; echo "password="; }; f"#,
            );
            options.add_config_flag("user.name", "testuser");
            options.add_config_flag("user.email", "testuser@localhost");
        }

        repo.checkout("branch_1", true).unwrap();
        commit_file(&repo, "dummy");
        let err = repo.push("origin", false).unwrap_err();
        assert!(matches!(dbg!(err), GitRemoteCommandError::AccessDenied));
    }

    /// Test that we pushing to a read only repo fails with [GitRemoteCommandError::AccessDenied]
    #[test]
    fn test_fetch_access_denied() {
        let (mut repo, _tempdir_handle) = init_temp_repo(false);
        repo.add_remote("origin", "https://github.com/flox/flox-private")
            .unwrap();

        repo.get_options_mut()
            .add_env_var("GIT_CONFIG_SYSTEM", "/dev/null");
        repo.get_options_mut()
            .add_env_var("GIT_CONFIG_GLOBAL", "/dev/null");
        repo.get_options_mut().add_config_flag(
            "credential.helper",
            r#"!f(){ echo "username="; echo "password="; }; f"#,
        );

        let err = repo.fetch().unwrap_err();

        assert!(matches!(dbg!(err), GitRemoteCommandError::AccessDenied));
    }

    /// Test that we pushing to a read only repo fails with [GitRemoteCommandError::AccessDenied]
    #[test]
    fn test_clone_access_denied() {
        let tempdir = tempfile::tempdir().unwrap();
        let mut options = GitCommandOptions::default();

        options.add_env_var("GIT_CONFIG_SYSTEM", "/dev/null");
        options.add_env_var("GIT_CONFIG_GLOBAL", "/dev/null");
        options.add_config_flag(
            "credential.helper",
            r#"!f(){ echo "username="; echo "password="; }; f"#,
        );

        let err: GitRemoteCommandError = GitCommandProvider::clone_branch_with(
            options,
            "https://github.com/flox/flox-private",
            tempdir,
            "main",
            false,
        )
        .unwrap_err();

        assert!(matches!(dbg!(err), GitRemoteCommandError::AccessDenied));
    }
}
