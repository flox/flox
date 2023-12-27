use anyhow::anyhow;
use flox_rust_sdk::models::environment::managed_environment::{
    ManagedEnvironmentError,
    GENERATION_LOCK_FILENAME,
};
use flox_rust_sdk::models::environment::remote_environment::RemoteEnvironmentError;
use flox_rust_sdk::models::environment::{
    CanonicalizeError,
    CoreEnvironmentError,
    EnvironmentError2,
    ENVIRONMENT_POINTER_FILENAME,
};
use flox_rust_sdk::models::pkgdb::CallPkgDbError;
use indoc::formatdoc;
use itertools::Itertools;

pub fn format_error(err: &'static EnvironmentError2) -> String {
    match err {
        EnvironmentError2::DotFloxNotFound => display_chain(err),

        // todo: enrich with a path?
        EnvironmentError2::EnvNotFound => formatdoc! {"
            Found a '.flox' directory, but were unable to locate an environment directory.

            This is likely due to a corrupt environment.

            Try deleting the '.flox' directory and reinitializing the environment.
            If you cloned this environment from a remote repository, verify that
            `.flox/env/maifest.toml` is commited to the version control system.
        "},

        // todo: enrich with a path?
        EnvironmentError2::ManifestNotFound => formatdoc! {"
            Found a '.flox' directory, but were unable to locate a manifest file.

            This is likely due to a corrupt environment.

            Try deleting the '.flox' directory and reinitializing the environment.
            If you cloned this environment from a remote repository, verify that
            `.flox/env/maifest.toml` is commited to the version control system.
        "},

        // todo: enrich with a path?
        // see also the notes on [EnvironmentError2::InitEnv]
        EnvironmentError2::InitEnv(err) => formatdoc! {"
            Failed to initialize environment.
            Could not prepare a '.flox' directory: {err}

            Please ensure that you have write permissions to the current directory.
        "},

        // todo: update when we implement `flox init --force`
        EnvironmentError2::EnvironmentExists(path) => formatdoc! {"
            Found an existing environment at {path:?}.

            Please initialize a new environment in a different directory.

            If you are trying to reinitialize an existing environment,
            delete the existing environment using 'flox delete -d {path:?}' and try again.
        "},

        // This should rarely happen.
        // At this point we already proved that we can write to the directory.
        EnvironmentError2::WriteGitignore(_) => display_chain(err),

        // todo: enrich with a path?
        EnvironmentError2::ReadEnvironmentMetadata(error) => formatdoc! {"
            Failed to read environment metadata: {error}

            This is likely due to a corrupt environment.

            Try deleting the '.flox' directory and reinitializing the environment.
            If you cloned this environment from a remote repository, verify that
            `.flox/env.json` is commited to the version control system.
        "},
        // todo: enrich with a path?
        // todo: when can this happen:
        //   * user manually edited this
        //   * user pushed environment but did not commit the changes to env.json
        //   * new version of the file format and we don't support it yet
        //     or not anymore with the current version of flox
        //     (this should be catched earlier but you never know...)
        EnvironmentError2::ParseEnvJson(error) => formatdoc! {"
            Failed to parse environment metadata: {error}

            This is likely due to a corrupt environment.

            Try deleting the '.flox' directory and reinitializing the environment.
            If you cloned this environment from a remote repository, verify that
            the latest changes to `.flox/env.json` are commited to the version control system.
        "},
        // this should always never be a problem and if it is, it's a bug
        // the user can likely not do anything about it
        // todo: add a note to user to report this as a bug?
        // todo: enrich with path
        EnvironmentError2::SerializeEnvJson(_) => display_chain(err),
        EnvironmentError2::WriteEnvJson(error) => formatdoc! {"
            Failed to write environment metadata: {error}

            Please ensure that you have write permissions to write '.flox/env.json'.
        "},

        // todo: enrich with global manifest path
        EnvironmentError2::InitGlobalManifest(err) => formatdoc! {"
            Failed to initialize global manifest: {err}

            Please ensure that you have write permissions
            to write '~/.config/flox/global-manifest.toml'.
        "},
        EnvironmentError2::ReadGlobalManifestTemplate(err) => formatdoc! {"
            Failed to read global manifest template: {err}

            Please ensure that you have read permissions
            to read '~/.config/flox/global-manifest.toml'.
        "},
        // todo: where in the control flow does this happen?
        //       do we want a separate error type for this (likely)
        EnvironmentError2::StartDiscoveryDir(CanonicalizeError { path, err }) => formatdoc! {"
            Failed to start discovery for flox environments in {path:?}: {err}

            Please ensure that the path exists and that you have read permissions.
        "},
        // unreachable when using the cli
        EnvironmentError2::InvalidPath(_) => display_chain(err),

        // todo: where in the control flow does this happen?
        //       do we want a separate error type for this (likely)
        // Its also a somewhat weird to downcast to this error type
        // better to separate this into a separate error types.
        EnvironmentError2::InvalidDotFlox { path, source } => {
            let source = if let Some(source) = source.downcast_ref::<EnvironmentError2>() {
                format_error(source)
            } else {
                display_chain(&**source)
            };

            formatdoc! {"
                Found a '.flox' directory at {path:?},
                but it is not a valid flox environment:

                {source}
            "}
        },
        // todo: how to surface these internal errors?
        EnvironmentError2::DiscoverGitDirectory(_) => formatdoc! {"
            Failed to discover git directory.

            See the '--debug' log for more information.
        "},
        // todo: enrich with path
        EnvironmentError2::DeleteEnvironment(err) => formatdoc! {"
            Failed to delete environment .flox directory: {err}

            Try manually deleting the '.flox' directory.
        "},
        // todo: enrich with path
        EnvironmentError2::ReadManifest(err) => formatdoc! {"
            Failed to read manifest: {err}

            Please make sure that '.flox/env/manifest.toml' exists
            and that you have read permissions.
        "},
        // todo: enrich with path
        EnvironmentError2::CreateGcRootDir(err) => format! {"
            Failed to create '.flox/run' directory: {err}

            Please make sure that you have write permissions to '.flox'.
        "},
        EnvironmentError2::Core(core_error) => format_core_error(core_error),
        EnvironmentError2::ManagedEnvironment(managed_error) => format_managed_error(managed_error),
        EnvironmentError2::RemoteEnvironment(remote_error) => format_remote_error(remote_error),
    }
}

fn format_core_error(err: &'static CoreEnvironmentError) -> String {
    match err {
        CoreEnvironmentError::ModifyToml(_) => todo!(),
        // todo: enrich with path
        // raised during edit
        CoreEnvironmentError::DeserializeManifest(err) => formatdoc! {
            "Failed to parse manifest: {err}

            Please ensure that '.flox/env/manifest.toml' is a valid TOML file.
        "},
        CoreEnvironmentError::MakeSandbox(_) => display_chain(err),
        // witin transaction, user should not see this and likely can't do anything about it
        CoreEnvironmentError::WriteLockfile(_) => display_chain(err),
        CoreEnvironmentError::MakeTemporaryEnv(_) => display_chain(err),
        CoreEnvironmentError::PriorTransaction(backup) => {
            let mut env_path = backup.clone();
            env_path.set_file_name("env");
            formatdoc! {"
                Found a transaction backup at {backup:?}.

                This indicates that a previous transaction was interrupted.

                Please restore the backup by moving {backup:?} -> {env_path:?}
                or delete the {backup:?} directory.
            "}
        },
        CoreEnvironmentError::BackupTransaction(err) => formatdoc! {"
            Failed to backup current environment directory: {err}

            Please ensure that you have write permissions to '.flox/*'."
        },
        CoreEnvironmentError::AbortTransaction(err) => formatdoc! {"
            Failed to abort transaction: {err}

            Please ensure that you have write permissions to '.flox/*'."
        },
        CoreEnvironmentError::Move(err) => formatdoc! {"
            Failed to commit transaction: {err}

            Could not move modified environment directory to original location.
        "},
        CoreEnvironmentError::RemoveBackup(err) => formatdoc! {"
            Failed to remove transaction backup: {err}

            Please ensure that you have write permissions to '.flox/*'.
        "},

        // these are out of our user's control as these errors are within the transaction
        // todo: adapt wordnig?
        // todo: enrich with path
        CoreEnvironmentError::OpenManifest(err) => formatdoc! {"
            Failed to open manifest for reading: {err}

            Please ensure that you have read permissions to '.flox/env/manifest.toml'.
        "},
        CoreEnvironmentError::UpdateManifest(err) => formatdoc! {"
            Failed to write to manifest file: {err}

            Please ensure that you have write permissions to '.flox/env/manifest.toml'.
        "},
        // internal error, a bug if this happens to users
        CoreEnvironmentError::BadManifestPath(_, _) => display_chain(err),
        CoreEnvironmentError::BadLockfilePath(_, _) => display_chain(err),
        // error returned by pkgdb
        // we do minimal formatting here as the error message is supposed to be
        // already user friendly.
        CoreEnvironmentError::LockManifest(pkgdb_error) => {
            format_pkgdb_error(pkgdb_error, err, "Failed to lock manifest.")
        },
        // other pkgdb call errors are unexpected
        CoreEnvironmentError::ParseUpdateOutput(_) => display_chain(err),
        CoreEnvironmentError::ParseUpgradeOutput(_) => display_chain(err),
        CoreEnvironmentError::UpdateFailed(pkgdb_error) => {
            format_pkgdb_error(pkgdb_error, err, "Failed to update environment.")
        },
        CoreEnvironmentError::UpgradeFailed(pkgdb_error) => {
            format_pkgdb_error(pkgdb_error, err, "Failed to upgrade environment.")
        },
        CoreEnvironmentError::BuildEnv(pkgdb_error) => {
            format_pkgdb_error(pkgdb_error, err, "Failed to build environment.")
        },
        CoreEnvironmentError::ParseBuildEnvOutput(_) => display_chain(err),
    }
}

fn format_managed_error(err: &'static ManagedEnvironmentError) -> String {
    match err {
        // todo: communicate reasons for this error
        // git auth errors may be catched separately or reported
        ManagedEnvironmentError::OpenFloxmeta(err) => formatdoc! {"
            Failed to fetch updates environment: {err}
        "},

        // todo: merge errors or make more specific
        // now they represent the same thing.
        ManagedEnvironmentError::Fetch(err) | ManagedEnvironmentError::FetchUpdates(err) => {
            formatdoc! {"
            Failed to fetch updates for environment: {err}

            Please ensure that you have network connectivity
            and access to the remote environment.
        "}
        },
        ManagedEnvironmentError::CheckGitRevision(_) => display_chain(err),
        ManagedEnvironmentError::CheckBranchExists(_) => display_chain(err),
        ManagedEnvironmentError::LocalRevDoesNotExist => formatdoc! {"
            The environment lockfile refers to a version of the environment
            that does not exist locally.

            This can happen if the environment is modified on another machine,
            and the lockfile is committed to the version control system
            before the environment is pushed.

            To resolve this issue, either
             * remove '.flox/{GENERATION_LOCK_FILENAME}' (this will reset the environment to the latest version)
             * push the environment on the remote machine and commit the updated lockfile
        "},
        ManagedEnvironmentError::RevDoesNotExist => formatdoc! {"
            The environment lockfile refers to a version of the environment
            that does not exist locally or on the remote.

            This can happen if the environment was force-pushed
            after the lockfile was committed to the version control system.

            To resolve this issue, remove '.flox/{GENERATION_LOCK_FILENAME}' (this will reset the environment to the latest version)
        "},
        ManagedEnvironmentError::InvalidLock(err) => formatdoc! {"
            The environment lockfile is invalid: {err}

            This can happen if the lockfile was manually edited.

            To resolve this issue, remove '.flox/{GENERATION_LOCK_FILENAME}' (this will reset the environment to the latest version)
        "},
        ManagedEnvironmentError::ReadPointerLock(err) => formatdoc! {"
            Failed to read pointer lockfile: {err}

            Please ensure that you have read permissions to '.flox/{GENERATION_LOCK_FILENAME}'.
        "},
        // various internal git errors while acting on the floxmeta repo
        ManagedEnvironmentError::Git(_) => display_chain(err),
        ManagedEnvironmentError::GitBranchHash(_) => display_chain(err),
        ManagedEnvironmentError::WriteLock(err) => formatdoc! {"
            Failed to write to lockfile: {err}

            Please ensure that you have write permissions to '.flox/{GENERATION_LOCK_FILENAME}'
        "},
        ManagedEnvironmentError::SerializeLock(_) => display_chain(err),

        // the following two errors are related to create reverse links to the .flox directory
        // those are internal errors but may arise if the user does not have write permissions to
        // xdg_data_home
        // todo: expose as rich error or unexpected error?
        ManagedEnvironmentError::ReverseLink(_) => display_chain(err),
        ManagedEnvironmentError::CreateLinksDir(_) => display_chain(err),

        ManagedEnvironmentError::BadBranchName(_) => display_chain(err),

        // currently unused
        ManagedEnvironmentError::ProjectNotFound { .. } => display_chain(err),

        // todo: enrich with url
        ManagedEnvironmentError::InvalidFloxhubBaseUrl(err) => formatdoc! {"
            The floxhub base url set in the config is invalid: {err}

            Please ensure that the url
            * is either a valid http or https url
            * has a valid domain name
            * is not an IP address or 'localhost'
        "},

        ManagedEnvironmentError::Diverged => formatdoc! {"
            The environment has diverged from the remote.

            This can happen if the environment is modified and pushed from another machine.

            To resolve this issue, either
             * run 'flox pull --force'
               to discard local changes
               and reset the environment to the latest upstream version.
             * run 'flox push --force'
               to overwrite the remote environment with the local changes.
               Attention: this will discard any changes made on the remote machine
               and cause conflicts when the remote machine tries to pull or push!
        "},
        ManagedEnvironmentError::AccessDenied => formatdoc! {"
            Access denied to the remote environment.

            This can happen if the remote is not owned by you
            or the owner did not grant you access.

            Please check the spelling of the remote environment
            and make sure that you have access to it.
        "},
        // acces denied is catched early as ManagedEnvironmentError::AccessDenied
        ManagedEnvironmentError::Push(_) => display_chain(err),
        ManagedEnvironmentError::DeleteBranch(_) => display_chain(err),
        ManagedEnvironmentError::DeleteEnvironment(path, err) => formatdoc! {"
            Failed to delete remote environment at {path:?}: {err}

            Please ensure that you have write permissions to {path:?}.
        "},
        ManagedEnvironmentError::DeleteEnvironmentLink(_, _) => display_chain(err),
        ManagedEnvironmentError::DeleteEnvironmentReverseLink(_, _) => display_chain(err),
        ManagedEnvironmentError::ApplyUpdates(_) => display_chain(err),
        // todo: unwrap this error to report more precisely?
        //       this should this is a bug if this happens to users.
        ManagedEnvironmentError::InitializeFloxmeta(_) => display_chain(err),
        ManagedEnvironmentError::SerializePointer(_) => display_chain(err),
        ManagedEnvironmentError::WritePointer(err) => formatdoc! {"
            Failed to write to pointer: {err}

            Please ensure that you have write permissions to '.flox/{ENVIRONMENT_POINTER_FILENAME}'.
        "},
        ManagedEnvironmentError::CreateFloxmetaDir(_) => display_chain(err),
        ManagedEnvironmentError::CreateGenerationFiles(_) => display_chain(err),
        ManagedEnvironmentError::CommitGeneration(err) => formatdoc! {"
            Failed to create a new generation: {err}

            This may be due to a corrupt environment
            or another process modifying the environment.

            Please try again later.
        "},
        ManagedEnvironmentError::Link(CoreEnvironmentError::LockManifest(pkgdb_err)) => {
            format_pkgdb_error(pkgdb_err, err, "Failed to lock manifest.")
        },
        &ManagedEnvironmentError::Link(CoreEnvironmentError::BuildEnv(ref pkgdb_err)) => {
            format_pkgdb_error(pkgdb_err, err, "Failed to build environment.")
        },
        ManagedEnvironmentError::Link(_) => display_chain(err),

        ManagedEnvironmentError::ReadManifest(_) => todo!(),
        ManagedEnvironmentError::CanonicalizePath(canonicalize_err) => formatdoc! {"
            Invalid path to environment: {canonicalize_err}

            Please ensure that the path exists and that you have read permissions.
        "},
    }
}

fn format_remote_error(err: &'static RemoteEnvironmentError) -> String {
    match err {
        RemoteEnvironmentError::OpenManagedEnvironment(_) => todo!(),
        RemoteEnvironmentError::GetLatestVersion(_) => todo!(),
        RemoteEnvironmentError::UpdateUpstream(_) => todo!(),
        RemoteEnvironmentError::InvalidTempPath(_) => todo!(),
    }
}

fn format_pkgdb_error(
    err: &CallPkgDbError,
    parent: impl Into<anyhow::Error>,
    context: &str,
) -> String {
    match err {
        CallPkgDbError::PkgDbError(err) => formatdoc! {"
            {context}

            {err}
        ", err = display_chain(err)},
        _ => display_chain(parent),
    }
}

fn display_chain(err: impl Into<anyhow::Error>) -> String {
    anyhow!(err).chain().join(": ")
}
