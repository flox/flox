use flox_rust_sdk::data::CanonicalizeError;
use flox_rust_sdk::models::environment::managed_environment::{
    ManagedEnvironmentError,
    GENERATION_LOCK_FILENAME,
};
use flox_rust_sdk::models::environment::remote_environment::RemoteEnvironmentError;
use flox_rust_sdk::models::environment::{
    CoreEnvironmentError,
    EnvironmentError,
    ENVIRONMENT_POINTER_FILENAME,
};
use flox_rust_sdk::models::lockfile::LockedManifestError;
use flox_rust_sdk::models::pkgdb::{error_codes, CallPkgDbError, ContextMsgError, PkgDbError};
use indoc::formatdoc;
use log::{debug, trace};

/// Convert to an error variant that directs the user to the docs if the provided error is
/// due to a package not being supported on the current system.
pub fn apply_doc_link_for_unsupported_packages(err: EnvironmentError) -> EnvironmentError {
    if let EnvironmentError::Core(CoreEnvironmentError::LockedManifest(
        LockedManifestError::BuildEnv(CallPkgDbError::PkgDbError(PkgDbError {
            exit_code: error_codes::PACKAGE_EVAL_INCOMPATIBLE_SYSTEM,
            category_message,
            context_message,
        })),
    )) = err
    {
        debug!("incompatible package, directing user to docs");
        EnvironmentError::Core(CoreEnvironmentError::LockedManifest(
            LockedManifestError::UnsupportedPackageWithDocLink(CallPkgDbError::PkgDbError(
                PkgDbError {
                    exit_code: error_codes::PACKAGE_EVAL_INCOMPATIBLE_SYSTEM,
                    category_message,
                    context_message,
                },
            )),
        ))
    } else {
        // Not the type of error we're concerned with, just pass it through
        err
    }
}

pub fn format_error(err: &EnvironmentError) -> String {
    trace!("formatting environment_error: {err:?}");

    match err {
        EnvironmentError::DotFloxNotFound(_) => display_chain(err),

        // todo: enrich with a path?
        EnvironmentError::EnvDirNotFound => formatdoc! {"
            Found a '.flox' directory but unable to locate an environment directory.

            This is likely due to a corrupt environment.

            Try deleting the '.flox' directory and reinitializing the environment.
            If you cloned this environment from a remote repository, verify that
            `.flox/env/maifest.toml` is commited to the version control system.
        "},
        // todo: enrich with a path?
        EnvironmentError::EnvPointerNotFound => formatdoc! {"
            Found a '.flox' directory but unable to locate an 'env.json' in it.

            This is likely due to a corrupt environment.

            Try deleting the '.flox' directory and reinitializing the environment.
            If you cloned this environment from a remote repository, verify that
            `.flox/env.json` is commited to the version control system.
        "},

        // todo: enrich with a path?
        EnvironmentError::ManifestNotFound => formatdoc! {"
            Found a '.flox' directory but unable to locate a manifest file.

            This is likely due to a corrupt environment.

            Try deleting the '.flox' directory and reinitializing the environment.
            If you cloned this environment from a remote repository, verify that
            `.flox/env/maifest.toml` is commited to the version control system.
        "},

        // todo: enrich with a path?
        // see also the notes on [EnvironmentError2::InitEnv]
        EnvironmentError::InitEnv(err) => formatdoc! {"
            Failed to initialize environment.
            Could not prepare a '.flox' directory: {err}

            Please ensure that you have write permissions to the current directory.
        "},

        // todo: update when we implement `flox init --force`
        EnvironmentError::EnvironmentExists(path) => formatdoc! {"
            Found an existing environment at {path:?}.

            Please initialize a new environment in a different directory.

            If you are trying to reinitialize an existing environment,
            delete the existing environment using 'flox delete -d {path:?}' and try again.
        "},

        // This should rarely happen.
        // At this point we already proved that we can write to the directory.
        EnvironmentError::WriteGitignore(_) => display_chain(err),

        // todo: enrich with a path?
        EnvironmentError::ReadEnvironmentMetadata(error) => formatdoc! {"
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
        EnvironmentError::ParseEnvJson(error) => formatdoc! {"
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
        EnvironmentError::SerializeEnvJson(_) => display_chain(err),
        EnvironmentError::WriteEnvJson(error) => formatdoc! {"
            Failed to write environment metadata: {error}

            Please ensure that you have write permissions to write '.flox/env.json'.
        "},

        // todo: enrich with global manifest path
        EnvironmentError::InitGlobalManifest(err) => formatdoc! {"
            Failed to initialize global manifest: {err}

            Please ensure that you have write permissions
            to write '~/.config/flox/global-manifest.toml'.
        "},
        EnvironmentError::ReadGlobalManifestTemplate(err) => formatdoc! {"
            Failed to read global manifest template: {err}

            Please ensure that you have read permissions
            to read '~/.config/flox/global-manifest.toml'.
        "},
        // todo: where in the control flow does this happen?
        //       do we want a separate error type for this (likely)
        EnvironmentError::StartDiscoveryDir(CanonicalizeError { path, err }) => formatdoc! {"
            Failed to start discovery for flox environments in {path:?}: {err}

            Please ensure that the path exists and that you have read permissions.
        "},
        // unreachable when using the cli
        EnvironmentError::InvalidPath(_) => display_chain(err),

        // todo: where in the control flow does this happen?
        //       do we want a separate error type for this (likely)
        // Its also a somewhat weird to downcast to this error type
        // better to separate this into a separate error types.
        EnvironmentError::InvalidDotFlox { path, source } => {
            let source = if let Some(source) = source.downcast_ref::<EnvironmentError>() {
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
        EnvironmentError::DiscoverGitDirectory(_) => formatdoc! {"
            Failed to discover git directory.

            See the run again with `--verbose` for more information.
        "},
        // todo: enrich with path
        EnvironmentError::DeleteEnvironment(err) => formatdoc! {"
            Failed to delete environment .flox directory: {err}

            Try manually deleting the '.flox' directory.
        "},
        // todo: enrich with path
        EnvironmentError::ReadManifest(err) => formatdoc! {"
            Failed to read manifest: {err}

            Please make sure that '.flox/env/manifest.toml' exists
            and that you have read permissions.
        "},

        // todo: enrich with path
        EnvironmentError::WriteManifest(err) => formatdoc! {"
            Failed to write manifest: {err}

            Please make sure that '.flox/env/manifest.toml' exists
            and that you have write permissions.
        "},

        // todo: enrich with path
        EnvironmentError::CreateGcRootDir(err) => format! {"
            Failed to create '.flox/run' directory: {err}

            Please make sure that you have write permissions to '.flox'.
        "},
        EnvironmentError::Core(core_error) => format_core_error(core_error),
        EnvironmentError::ManagedEnvironment(managed_error) => format_managed_error(managed_error),
        EnvironmentError::RemoteEnvironment(remote_error) => format_remote_error(remote_error),
        _ => display_chain(err),
    }
}

pub fn format_core_error(err: &CoreEnvironmentError) -> String {
    trace!("formatting core_error: {err:?}");

    match err {
        CoreEnvironmentError::ModifyToml(toml_error) => formatdoc! {"
            Failed to modify manifest.

            {toml_error}
        "},
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
        // todo: enrich with path
        CoreEnvironmentError::UpdateManifest(err) => formatdoc! {"
            Failed to write to manifest file: {err}

            Please ensure that you have write permissions to '.flox/env/manifest.toml'.
        "},

        // internal error, a bug if this happens to users!
        CoreEnvironmentError::BadLockfilePath(_) => display_chain(err),

        // todo: should be in LockedManifesterror
        CoreEnvironmentError::UpgradeFailed(pkgdb_error) => {
            format_pkgdb_error(pkgdb_error, err, "Failed to upgrade environment.")
        },
        // other pkgdb call errors are unexpected
        CoreEnvironmentError::ParseUpgradeOutput(_) => display_chain(err),

        CoreEnvironmentError::LockedManifest(locked_manifest_error) => {
            format_locked_manifest_error(locked_manifest_error)
        },

        CoreEnvironmentError::ContainerizeUnsupportedSystem(system) => formatdoc! {"
            'containerize' is currently only supported on linux (found {system}).
        "},

        CoreEnvironmentError::CatalogClientMissing => formatdoc! {"
            The current manifest requires the (experimental) catalog feature.

            Please enable the catalog feature and try again.
        "},
    }
}

pub fn format_managed_error(err: &ManagedEnvironmentError) -> String {
    trace!("formatting managed_environment_error: {err:?}");

    match err {
        // todo: communicate reasons for this error
        // git auth errors may be catched separately or reported
        ManagedEnvironmentError::OpenFloxmeta(err) => formatdoc! {"
            Failed to fetch environment: {err}
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
            The FloxHub base url set in the config is invalid: {err}

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
        // todo: mark as bug?
        ManagedEnvironmentError::UpstreamNotFound(_, _) => display_chain(err),
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

        ManagedEnvironmentError::ReadManifest(e) => formatdoc! {"
            Could not read managed manifest.

            {err}
        ",err = display_chain(e) },
        ManagedEnvironmentError::CanonicalizePath(canonicalize_err) => formatdoc! {"
            Invalid path to environment: {canonicalize_err}

            Please ensure that the path exists and that you have read permissions.
        "},
        ManagedEnvironmentError::Build(core_environment_error) => {
            format_core_error(core_environment_error)
        },
        ManagedEnvironmentError::Registry(_) => display_chain(err),
    }
}

pub fn format_remote_error(err: &RemoteEnvironmentError) -> String {
    trace!("formatting remote_environment_error: {err:?}");

    match err {
        RemoteEnvironmentError::OpenManagedEnvironment(err) => formatdoc! {"
            Failed to open cloned remote environment: {err}

            This may be due to a corrupt or incompatible environment.
        ", err = display_chain(err)},

        RemoteEnvironmentError::CreateTempDotFlox(_) => formatdoc! {"
            Failed to initialize remote environment locally.

            Please ensure that you have write permissions to FLOX_CACHE_DIR/remote.
        "},

        RemoteEnvironmentError::ResetManagedEnvironment(err) => formatdoc! {"
            Failed to reset remote environment to latest upstream version:

            {err}
            ", err = format_managed_error(err)},

        RemoteEnvironmentError::GetLatestVersion(err) => formatdoc! {"
            Failed to get latest version of remote environment: {err}

            ", err = display_chain(err)},
        RemoteEnvironmentError::UpdateUpstream(ManagedEnvironmentError::Diverged) => formatdoc! {"
            The remote environment has diverged.

            This can happen if the environment is modified and pushed from another machine
            at the same time.

            Please try again after verifying the concurrent changes.
        "},
        RemoteEnvironmentError::UpdateUpstream(ManagedEnvironmentError::AccessDenied) => {
            formatdoc! {"
            Access denied to the remote environment.

            This can happen if the remote is not owned by you
            or the owner did not grant you access.

            Please check the spelling of the remote environment
            and make sure that you have access to it.
        "}
        },
        RemoteEnvironmentError::UpdateUpstream(_) => display_chain(err),
        RemoteEnvironmentError::InvalidTempPath(_) => display_chain(err),
        RemoteEnvironmentError::ReadInternalOutLink(_) => display_chain(err),
        RemoteEnvironmentError::DeleteOldOutLink(_) => display_chain(err),
        RemoteEnvironmentError::WriteNewOutlink(_) => display_chain(err),
    }
}

pub fn format_locked_manifest_error(err: &LockedManifestError) -> String {
    trace!("formatting locked_manifest_error: {err:?}");
    match err {
        LockedManifestError::CallContainerBuilder(_) => formatdoc! {"
            Failed to call container builder.

            Successfully created a container builder for you environment,
            but failed to call it.
        "},

        // todo: enrich with path
        LockedManifestError::WriteContainer(err) => formatdoc! {"
            Failed to write container: {err}

            Please ensure that you have write permissions to
            the destination file.
        "},

        // this is a BUG
        LockedManifestError::ParseBuildEnvOutput(_) => display_chain(err),
        // this is likely a BUG, since we ensure that the lockfile exists in all cases
        LockedManifestError::BadLockfilePath(canonicalize_error) => formatdoc! {"
            Bad lockfile path: {canonicalize_error}

            Please ensure that the path exists and that you have read permissions.
        "},

        LockedManifestError::BadManifestPath(canonicalize_error) => formatdoc! {"
            Corrupt environment: {canonicalize_error}

            Please ensure that the path exists and that you have read permissions.
        "},
        // region: errors returned by pkgdb
        // we do minimal formatting here as the error message is supposed to be
        // already user friendly.
        // some commands catch these errors and process them separately
        // e.g. `flox pull`, `flox push`

        // 105: invalid lockfile, just print the error message
        // https://github.com/flox/flox/issues/852
        LockedManifestError::LockManifest(CallPkgDbError::PkgDbError(PkgDbError {
            exit_code: error_codes::INVALID_MANIFEST_FILE,
            context_message: Some(ContextMsgError { message, .. }),
            ..
        })) => formatdoc! {"
            {message}
        "},

        // 116: toml parsing error, just print the error message
        // https://github.com/flox/flox/issues/852
        LockedManifestError::LockManifest(CallPkgDbError::PkgDbError(PkgDbError {
            exit_code: error_codes::TOML_TO_JSON,
            context_message:
                Some(ContextMsgError {
                    caught: Some(caught),
                    ..
                }),
            ..
        })) => {
            let message = &caught.message;
            let un_prefixed = message.strip_prefix("[error] ").unwrap_or(message);
            formatdoc! {"
                {un_prefixed}
            "}
        },
        // 127: bad package, forbidden by options
        // https://github.com/flox/flox/issues/492
        LockedManifestError::LockManifest(CallPkgDbError::PkgDbError(PkgDbError {
            exit_code: error_codes::BAD_PACKAGE_FAILURE,
            context_message: Some(ContextMsgError { message, .. }),
            ..
        })) => message.to_string(),

        LockedManifestError::LockManifest(pkgdb_error) => {
            format_pkgdb_error(pkgdb_error, err, "Failed to lock environment manifest.")
        },

        // catch package conflict error:
        // https://github.com/flox/flox/issues/857
        LockedManifestError::BuildEnv(CallPkgDbError::PkgDbError(PkgDbError {
            exit_code: error_codes::BUILDENV_CONFLICT,
            context_message: Some(ContextMsgError { message, .. }),
            ..
        })) => message.to_string(),
        LockedManifestError::BuildEnv(CallPkgDbError::PkgDbError(PkgDbError {
            exit_code: error_codes::PACKAGE_EVAL_INCOMPATIBLE_SYSTEM,
            context_message: Some(ContextMsgError { message, .. }),
            ..
        })) => message.into(),
        // We manually construct this error variant in cases where we want to add a link to the docs,
        // otherwise it's the same as the basic PACKAGE_EVAL_INCOMPATIBLE_SYSTEM error.
        LockedManifestError::UnsupportedPackageWithDocLink(CallPkgDbError::PkgDbError(
            PkgDbError {
                exit_code: error_codes::PACKAGE_EVAL_INCOMPATIBLE_SYSTEM,
                context_message,
                ..
            },
        )) => {
            if let Some(ctx_msg) = context_message {
                formatdoc! {"
                {}

                For more on managing system-specific packages, visit the documentation:
                https://flox.dev/docs/tutorials/multi-arch-environments/#handling-unsupported-packages
            ", ctx_msg}
            } else {
                // In this context it's an error to encounter an error (heh) where the context message is missing,
                // but a vague error message is preferable to panicking.
                formatdoc! {"
                    This package is not available for this system

                    For more on managing system-specific packages, visit the documentation:
                    https://flox.dev/docs/tutorials/multi-arch-environments/#handling-unsupported-packages
                "}
            }
        },
        // Since we manually construct the UnsupportedPackageWithDocLink variant we should *never* encounter
        // a situation in which it contains the wrong kind of error. That said, we need some kind of error
        // in case we've screwed up.
        LockedManifestError::UnsupportedPackageWithDocLink(_) => {
            // Could probably do with a better error message
            "encountered an internal error".into()
        },
        // catch package eval and build errors
        LockedManifestError::BuildEnv(CallPkgDbError::PkgDbError(PkgDbError {
            exit_code,
            context_message:
                Some(ContextMsgError {
                    message,
                    caught: Some(caught),
                }),
            ..
        })) if [
            error_codes::PACKAGE_EVAL_FAILURE,
            error_codes::PACKAGE_BUILD_FAILURE,
        ]
        .contains(exit_code) =>
        {
            format!("{message}: {caught}")
        },
        LockedManifestError::BuildEnv(pkgdb_error) => {
            format_pkgdb_error(pkgdb_error, err, "Failed to build environment.")
        },
        LockedManifestError::UpdateFailed(pkgdb_error) => {
            format_pkgdb_error(pkgdb_error, err, "Failed to update environment.")
        },
        LockedManifestError::CheckLockfile(pkgdb_error) => {
            format_pkgdb_error(pkgdb_error, err, "Failed to check environment.")
        },
        // endregion

        // this is a bug, but likely needs some formatting
        LockedManifestError::ReadLockfile(_) => display_chain(err),
        LockedManifestError::ParseLockfile(serde_error) => formatdoc! {"
            Failed to parse lockfile as JSON: {serde_error}

            This is likely due to a corrupt environment.
        "},

        LockedManifestError::ParseLockedManifest(serde_error) => formatdoc! {"
            Failed to parse lockfile structure: {serde_error}

            This is likely due to a corrupt environment.
        "},

        LockedManifestError::SerializeGlobalLockfile(_) => display_chain(err),

        // todo: add global-manifest.lock(1) manual entry and reference it here
        LockedManifestError::WriteGlobalLockfile(_) => formatdoc! {"
            Failed to write global lockfile: {err}

            Please ensure that you have write permissions to '~/.config/flox/global-manifest.lock'.
        "},

        LockedManifestError::ParseCheckWarnings(_) => display_chain(err),
        LockedManifestError::UnsupportedLockfileForUpdate => display_chain(err),
    }
}

fn format_pkgdb_error(
    err: &CallPkgDbError,
    parent: &dyn std::error::Error,
    context: &str,
) -> String {
    trace!("formatting pkgdb_error: {err:?}");

    match err {
        CallPkgDbError::PkgDbError(err) => formatdoc! {"
            {context}

            {err}
        ", err = display_chain(err)},
        _ => display_chain(parent),
    }
}

/// Displays and formats a chain of errors connected via their `source` attribute.
pub fn display_chain(mut err: &dyn std::error::Error) -> String {
    let mut fmt = err.to_string();
    while let Some(source) = err.source() {
        fmt = format!("{}: {}", fmt, source);
        err = source;
    }

    fmt
}
