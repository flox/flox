//! Resolution, creation, and FloxHub sync of the user's default environment.
//!
//! `--default` historically meant exactly "the remote environment
//! `<credential handle>/default`", which made the credential a hard
//! prerequisite for naming the environment at all (DEV-269). This module
//! resolves the default environment from local state when no credential is
//! available, and — behind the `auto_default` feature flag — implements
//! zero-setup defaults: create the default environment on first use, prefer a
//! `~/.flox` checkout as the single working copy, and keep it synced with
//! FloxHub around login and mutating commands.

use std::str::FromStr;

use anyhow::{Context, Result};
use flox_core::activate::mode::ActivateMode;
use flox_core::data::environment_ref::{
    DEFAULT_NAME,
    EnvironmentName,
    EnvironmentOwner,
    RemoteEnvironmentRef,
};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::floxmeta_branch::FloxmetaBranchError;
use flox_rust_sdk::models::environment::generations::GenerationId;
use flox_rust_sdk::models::environment::managed_environment::{
    GENERATION_LOCK_FILENAME,
    ManagedEnvironment,
    ManagedEnvironmentError,
    PushResult,
};
use flox_rust_sdk::models::environment::path_environment::{InitCustomization, PathEnvironment};
use flox_rust_sdk::models::environment::remote_environment::{
    RemoteEnvironment,
    RemoteEnvironmentError,
};
use flox_rust_sdk::models::environment::{
    ConcreteEnvironment,
    DOT_FLOX,
    DotFlox,
    ENVIRONMENT_POINTER_FILENAME,
    EnvironmentError,
    EnvironmentPointer,
    ManagedPointer,
    PathPointer,
    UninitializedEnvironment,
};
use flox_rust_sdk::models::user_state::{
    last_floxhub_handle,
    read_user_state_file,
    user_state_path,
};
use flox_rust_sdk::providers::git::GitRemoteCommandError;
use tracing::debug;

use crate::commands::install::prompt_to_modify_rc_file;
use crate::utils::dialog::Dialog;
use crate::utils::message;

/// Who the default environment belongs to, as far as we can tell without
/// forcing a login.
enum OwnerSource {
    /// The credential yielded a handle (an expired token still carries one).
    Authed(String),
    /// A credential is present but its identity could not be verified
    /// (e.g. a personal access token while FloxHub is unreachable).
    Unverified,
    /// No credential at all.
    LoggedOut,
}

async fn owner_source(flox: &Flox) -> OwnerSource {
    match flox.get_identity().await {
        Ok(Some(identity)) => OwnerSource::Authed(identity.handle),
        Ok(None) => OwnerSource::Unverified,
        Err(_) => OwnerSource::LoggedOut,
    }
}

/// Resolve the environment that `--default` refers to.
///
/// `allow_create` permits creating a missing default environment as a side
/// effect (on FloxHub when authenticated, at `~/.flox` otherwise). Pass it
/// only from interactive or mutating contexts so that read-only commands and
/// non-interactive shell-RC activations never create environments.
pub(crate) async fn resolve_default_environment(
    flox: &mut Flox,
    generation: Option<GenerationId>,
    allow_create: bool,
) -> Result<ConcreteEnvironment> {
    let source = owner_source(flox).await;
    if flox.features.auto_default {
        resolve_auto(flox, source, generation, allow_create).await
    } else {
        resolve_classic(flox, source, generation).await
    }
}

/// Open the default environment for `flox install` when no environment is
/// found, creating it if necessary. Creation is allowed even without a TTY:
/// installing a package is an explicit request for an environment to exist.
/// A user who previously declined the default-environment offer keeps that
/// choice: resolution still works, creation does not.
pub(crate) async fn open_or_create_default_environment(
    flox: &mut Flox,
) -> Result<ConcreteEnvironment> {
    let source = owner_source(flox).await;
    let allow_create = !declined_default_env(flox);
    resolve_auto(flox, source, None, allow_create).await
}

/// Whether the user answered "No" to the pre-flag default-environment offer.
fn declined_default_env(flox: &Flox) -> bool {
    read_user_state_file(user_state_path(flox))
        .ok()
        .flatten()
        .and_then(|state| state.confirmed_create_default_env)
        == Some(false)
}

/// The pre-`auto_default` behavior plus the DEV-269 fallback: when the
/// credential is missing (or unverifiable), a previously fetched checkout of
/// the default environment is used instead of forcing a login.
async fn resolve_classic(
    flox: &mut Flox,
    source: OwnerSource,
    generation: Option<GenerationId>,
) -> Result<ConcreteEnvironment> {
    match source {
        OwnerSource::Authed(handle) => open_remote_default(flox, &handle, generation),
        OwnerSource::Unverified => {
            if let Some(env) = open_cached_default(flox, generation, false)? {
                return Ok(env);
            }
            open_remote_default(flox, floxhub_client::UNKNOWN_HANDLE, generation)
        },
        OwnerSource::LoggedOut => {
            if let Some(env) = open_cached_default(flox, generation, true)? {
                return Ok(env);
            }
            let handle = super::ensure_auth(flox).await?;
            open_remote_default(flox, &handle, generation)
        },
    }
}

/// The `auto_default` resolution ladder. A default environment checked out at
/// `~/.flox` always wins — it is the single working copy both the
/// authenticated and logged-out paths agree on — followed by the FloxHub
/// environment (or its local cache when logged out), followed by creation.
async fn resolve_auto(
    flox: &mut Flox,
    source: OwnerSource,
    generation: Option<GenerationId>,
    allow_create: bool,
) -> Result<ConcreteEnvironment> {
    match home_default_env(flox) {
        Some(HomeDefault::Managed(dot_flox, pointer)) => {
            let matches_user = match &source {
                OwnerSource::Authed(handle) => pointer.owner.as_str() == handle.as_str(),
                // Without a verifiable identity, an identity recorded at
                // login/logout decides; absent that, assume the checkout is
                // the user's own.
                OwnerSource::Unverified | OwnerSource::LoggedOut => {
                    last_floxhub_handle(flox).is_none_or(|handle| pointer.owner.as_str() == handle)
                },
            };
            if matches_user {
                let env = UninitializedEnvironment::DotFlox(dot_flox)
                    .into_concrete_environment(flox, generation)?;
                return Ok(env);
            }
            if let OwnerSource::Authed(handle) = &source {
                message::info(format!(
                    "The default environment at '~/{DOT_FLOX}' belongs to '{owner}'. Using '{handle}/{DEFAULT_NAME}' from FloxHub.",
                    owner = pointer.owner,
                ));
            }
        },
        Some(HomeDefault::Path(dot_flox)) => {
            return resolve_home_path_default(flox, source, generation, allow_create, dot_flox)
                .await;
        },
        None => {},
    }

    // No usable ~/.flox checkout.
    match source {
        OwnerSource::Authed(handle) => match open_remote_default(flox, &handle, generation) {
            Ok(env) => Ok(env),
            // A specific generation can only come from FloxHub history, so a
            // missing environment stays an error rather than creating a
            // brand-new environment that cannot honor the request.
            Err(err)
                if allow_create && generation.is_none() && error_is_upstream_not_found(&err) =>
            {
                let env = create_default_on_floxhub(flox, &handle)?;
                maybe_offer_rc_setup();
                Ok(env)
            },
            Err(err) => Err(err),
        },
        OwnerSource::Unverified => {
            if let Some(env) = open_cached_default(flox, generation, false)? {
                return Ok(env);
            }
            open_remote_default(flox, floxhub_client::UNKNOWN_HANDLE, generation)
        },
        OwnerSource::LoggedOut => {
            if let Some(env) = open_cached_default(flox, generation, true)? {
                return Ok(env);
            }
            if allow_create && generation.is_none() {
                let env = create_default_at_home(flox, generation)?;
                maybe_offer_rc_setup();
                return Ok(env);
            }
            let handle = super::ensure_auth(flox).await?;
            open_remote_default(flox, &handle, generation)
        },
    }
}

/// Resolve `--default` when `~/.flox` holds a never-pushed path environment
/// named 'default'. Authenticated, the FloxHub state decides: an existing
/// `<handle>/default` wins (the local environment stays untouched), a missing
/// one is created from the local environment when creation is allowed.
async fn resolve_home_path_default(
    flox: &mut Flox,
    source: OwnerSource,
    generation: Option<GenerationId>,
    allow_create: bool,
    dot_flox: DotFlox,
) -> Result<ConcreteEnvironment> {
    let OwnerSource::Authed(handle) = source else {
        return UninitializedEnvironment::DotFlox(dot_flox)
            .into_concrete_environment(flox, generation)
            .map_err(anyhow::Error::new);
    };

    match open_remote_default(flox, &handle, generation) {
        Ok(env) => {
            message::info(format!(
                "Found a local default environment at '~/{DOT_FLOX}' and '{handle}/{DEFAULT_NAME}' on FloxHub. Using the FloxHub environment.\nTo use the local environment instead, activate it with 'flox activate -d ~'.",
            ));
            Ok(env)
        },
        Err(err) if error_is_upstream_not_found(&err) => {
            // `--generation` needs FloxHub history; a plain path environment
            // (and a conversion mid-resolution) cannot honor it.
            if generation.is_some() {
                return Err(err);
            }
            if allow_create {
                match push_home_default(flox, &handle, &dot_flox) {
                    Ok(env) => return Ok(env),
                    Err(push_err) => {
                        message::warning(format!(
                            "Could not sync your default environment to FloxHub: {push_err:#}\nUsing the local environment. Run 'flox push' from your home directory to retry.",
                        ));
                    },
                }
            }
            UninitializedEnvironment::DotFlox(dot_flox)
                .into_concrete_environment(flox, generation)
                .map_err(anyhow::Error::new)
        },
        Err(err) => {
            debug!(%err, "could not check FloxHub for a default environment");
            message::warning(
                "Could not reach FloxHub to check for a default environment. Using the local default environment.",
            );
            UninitializedEnvironment::DotFlox(dot_flox)
                .into_concrete_environment(flox, generation)
                .map_err(anyhow::Error::new)
        },
    }
}

/// How `~/.flox` participates in default-environment resolution.
enum HomeDefault {
    /// A checkout of `<owner>/default` on the configured FloxHub.
    Managed(DotFlox, Box<ManagedPointer>),
    /// A never-pushed local default environment.
    Path(DotFlox),
}

fn home_default_env(flox: &Flox) -> Option<HomeDefault> {
    let home = dirs::home_dir()?;
    let dot_flox = DotFlox::open_in(home).ok()?;
    if dot_flox.pointer.name().as_ref() != DEFAULT_NAME {
        return None;
    }
    match &dot_flox.pointer {
        EnvironmentPointer::Managed(pointer) => {
            if pointer.floxhub_base_url != *flox.floxhub.base_url() {
                debug!(
                    url = %pointer.floxhub_base_url,
                    "ignoring ~/.flox default environment from another FloxHub"
                );
                return None;
            }
            let pointer = Box::new(pointer.clone());
            Some(HomeDefault::Managed(dot_flox, pointer))
        },
        EnvironmentPointer::Path(_) => Some(HomeDefault::Path(dot_flox)),
    }
}

/// Open `<handle>/default` as a remote environment, the pre-existing
/// `--default` behavior.
fn open_remote_default(
    flox: &Flox,
    handle: &str,
    generation: Option<GenerationId>,
) -> Result<ConcreteEnvironment> {
    let env_ref = RemoteEnvironmentRef::new(handle, DEFAULT_NAME)
        .context("Failed to construct default environment reference")?;
    let pointer = ManagedPointer::new(
        env_ref.owner().clone(),
        env_ref.name().clone(),
        &flox.floxhub,
    );
    let env = RemoteEnvironment::new(flox, pointer, generation).map_err(anyhow::Error::new)?;
    Ok(ConcreteEnvironment::Remote(env))
}

/// Open the locally cached checkout of the default environment, when the
/// owner can be determined without a credential and the checkout exists.
///
/// Returns `Ok(None)` — letting the caller continue its resolution ladder —
/// whenever the cache cannot serve the request, including when a checkout
/// exists but fails to open: a broken cache must degrade to the ladder's next
/// step (login or creation), never dead-end the command.
fn open_cached_default(
    flox: &Flox,
    generation: Option<GenerationId>,
    suggest_login: bool,
) -> Result<Option<ConcreteEnvironment>> {
    let Some(owner) = offline_default_owner(flox) else {
        return Ok(None);
    };
    let pointer = ManagedPointer::new(owner.clone(), default_name(), &flox.floxhub);
    if !cached_default_checkout_is_valid(flox, &pointer) {
        return Ok(None);
    }
    match RemoteEnvironment::new(flox, pointer, generation) {
        Ok(env) => {
            let hint = if suggest_login {
                "Run 'flox auth login' to sync with FloxHub."
            } else {
                "Changes will sync when FloxHub is reachable."
            };
            message::info(format!(
                "Using the cached default environment '{owner}/{DEFAULT_NAME}'. {hint}",
            ));
            Ok(Some(ConcreteEnvironment::Remote(env)))
        },
        Err(err) => {
            debug!(%err, "could not open cached default environment");
            Ok(None)
        },
    }
}

/// Whether the cache holds a genuinely usable checkout for `pointer`.
///
/// `RemoteEnvironment::new` creates the checkout directory before any
/// validation, so a failed open leaves an empty `.flox` behind; a bare
/// existence check would mistake that phantom for a cached environment. A
/// real checkout has a pointer file matching `pointer` and a generation lock
/// (both written only after a successful open).
fn cached_default_checkout_is_valid(flox: &Flox, pointer: &ManagedPointer) -> bool {
    let dot_flox = RemoteEnvironment::checkout_path(flox, pointer).join(DOT_FLOX);
    let pointer_matches = std::fs::read_to_string(dot_flox.join(ENVIRONMENT_POINTER_FILENAME))
        .ok()
        .and_then(|contents| serde_json::from_str::<ManagedPointer>(&contents).ok())
        .is_some_and(|on_disk| {
            on_disk.owner == pointer.owner
                && on_disk.name == pointer.name
                && on_disk.floxhub_base_url == pointer.floxhub_base_url
        });
    pointer_matches && dot_flox.join(GENERATION_LOCK_FILENAME).exists()
}

/// Determine the default environment's owner without a credential.
///
/// An identity recorded at login/logout is authoritative and never falls
/// through to enumeration — enumeration could name a different user. Without
/// a recorded identity, a single unambiguous cached checkout decides.
fn offline_default_owner(flox: &Flox) -> Option<EnvironmentOwner> {
    if let Some(handle) = last_floxhub_handle(flox) {
        return EnvironmentOwner::from_str(&handle).ok();
    }
    single_cached_default_owner(flox)
}

/// Enumerate `<cache_dir>/remote/<owner>/default` checkouts belonging to the
/// configured FloxHub and return the owner iff exactly one matches.
fn single_cached_default_owner(flox: &Flox) -> Option<EnvironmentOwner> {
    let entries = std::fs::read_dir(RemoteEnvironment::cache_base_dir(flox)).ok()?;
    let mut found: Option<EnvironmentOwner> = None;
    for entry in entries.flatten() {
        let pointer_path = entry
            .path()
            .join(DEFAULT_NAME)
            .join(DOT_FLOX)
            .join(ENVIRONMENT_POINTER_FILENAME);
        let Ok(contents) = std::fs::read_to_string(&pointer_path) else {
            continue;
        };
        let Ok(pointer) = serde_json::from_str::<ManagedPointer>(&contents) else {
            continue;
        };
        if pointer.floxhub_base_url != *flox.floxhub.base_url()
            || pointer.name.as_ref() != DEFAULT_NAME
        {
            continue;
        }
        match &found {
            None => found = Some(pointer.owner),
            Some(owner) if *owner == pointer.owner => {},
            Some(_) => {
                debug!("multiple cached default environments found, not picking one");
                return None;
            },
        }
    }
    found
}

/// Create `<handle>/default` on FloxHub (with an empty manifest) and open it.
fn create_default_on_floxhub(flox: &Flox, handle: &str) -> Result<ConcreteEnvironment> {
    let env_ref = RemoteEnvironmentRef::new(handle, DEFAULT_NAME)
        .context("Failed to construct default environment reference")?;
    let env = RemoteEnvironment::init_floxhub_environment(flox, env_ref, false)
        .context("Failed to create the default environment on FloxHub")?;
    message::created(format!(
        "Created your default environment '{handle}/{DEFAULT_NAME}' on FloxHub.",
    ));
    Ok(ConcreteEnvironment::Remote(env))
}

/// Create a local default environment at `~/.flox`.
fn create_default_at_home(
    flox: &Flox,
    generation: Option<GenerationId>,
) -> Result<ConcreteEnvironment> {
    if generation.is_some() {
        anyhow::bail!("The '--generation' option requires a default environment on FloxHub.");
    }
    let home = dirs::home_dir().context("failed to locate home directory")?;
    // `~/.flox` may be occupied by an environment this resolution could not
    // use (another owner's checkout, another FloxHub, a differently named
    // environment); creating over it is never right.
    if home.join(DOT_FLOX).exists() {
        anyhow::bail!(
            "An environment already exists at '~/{DOT_FLOX}' but it is not your default environment.\nActivate it directly with 'flox activate -d ~', or log in with 'flox auth login'."
        );
    }
    let customization = InitCustomization {
        activate_mode: Some(ActivateMode::Run),
        ..Default::default()
    };
    let env = PathEnvironment::init(PathPointer::new(default_name()), home, &customization, flox)?;
    message::created(format!(
        "Created your default environment at '~/{DOT_FLOX}'. Log in with 'flox auth login' to sync it with FloxHub.",
    ));
    Ok(ConcreteEnvironment::Path(env))
}

/// Convert the `~/.flox` path environment into `<handle>/default` on FloxHub.
fn push_home_default(flox: &Flox, handle: &str, dot_flox: &DotFlox) -> Result<ConcreteEnvironment> {
    let owner = EnvironmentOwner::from_str(handle).context("invalid FloxHub handle")?;
    let ConcreteEnvironment::Path(path_env) = UninitializedEnvironment::DotFlox(dot_flox.clone())
        .into_concrete_environment(flox, None)?
    else {
        unreachable!("caller checked that ~/.flox holds a path environment");
    };
    let managed = ManagedEnvironment::push_new(flox, path_env, owner, false, false)?;
    message::updated(format!(
        "Synced your default environment to FloxHub as '{handle}/{DEFAULT_NAME}'. It will now sync across your machines.",
    ));
    Ok(ConcreteEnvironment::Managed(managed))
}

/// Whether the (anyhow-wrapped) error means `<owner>/default` does not exist
/// on FloxHub.
fn error_is_upstream_not_found(err: &anyhow::Error) -> bool {
    let Some(remote_err) = err.downcast_ref::<RemoteEnvironmentError>() else {
        return false;
    };
    matches!(
        remote_err,
        RemoteEnvironmentError::GetLatestVersion(FloxmetaBranchError::UpstreamNotFound { .. })
            | RemoteEnvironmentError::OpenManagedEnvironment(
                ManagedEnvironmentError::UpstreamNotFound { .. }
            )
            | RemoteEnvironmentError::OpenManagedEnvironment(
                ManagedEnvironmentError::FloxmetaBranch(
                    FloxmetaBranchError::UpstreamNotFound { .. }
                )
            )
    )
}

/// After creating a default environment, offer to wire it into the user's
/// shell RC files. Failures and non-interactive contexts are silently
/// ignored; creation itself already succeeded.
fn maybe_offer_rc_setup() {
    if !Dialog::can_prompt() {
        return;
    }
    if let Err(err) = prompt_to_modify_rc_file() {
        debug!(%err, "could not offer RC file setup");
    }
}

/// After a mutating command on the user's own default environment, push the
/// change to FloxHub so other machines and the web UI reflect it without a
/// manual 'flox push'. Failures never fail the primary command.
pub(crate) async fn sync_default_env_to_floxhub(flox: &Flox, env: &mut ConcreteEnvironment) {
    if !flox.features.auto_default {
        return;
    }
    // Cheap structural checks first; identity resolution can hit the network
    // for personal access tokens.
    let pointer = match env {
        ConcreteEnvironment::Managed(managed) => managed.pointer().clone(),
        ConcreteEnvironment::Remote(remote) => remote.pointer().clone(),
        ConcreteEnvironment::Path(_) => return,
    };
    if pointer.name.as_ref() != DEFAULT_NAME || pointer.floxhub_base_url != *flox.floxhub.base_url()
    {
        return;
    }
    let Ok(Some(identity)) = flox.get_identity().await else {
        return;
    };
    if identity.handle == floxhub_client::UNKNOWN_HANDLE
        || pointer.owner.as_str() != identity.handle
    {
        return;
    }
    // An expired token cannot push; skip with one actionable hint rather
    // than warning with a failed-push error on every mutation.
    if identity.is_expired() {
        message::info(
            "Not syncing your default environment because your FloxHub token has expired. Run 'flox auth login' to resume syncing.",
        );
        return;
    }

    let result = match env {
        ConcreteEnvironment::Managed(managed) => managed.push(flox, false),
        ConcreteEnvironment::Remote(remote) => remote.push(flox, false),
        ConcreteEnvironment::Path(_) => unreachable!("filtered above"),
    };
    match result {
        Ok(PushResult::Updated) => {
            message::updated("Synced your default environment to FloxHub.");
        },
        Ok(PushResult::UpToDate) => {},
        Err(err) if error_is_diverged(&err) => {
            message::warning(
                "Your default environment has diverged from FloxHub.\nRun 'flox pull' to reconcile, then 'flox push'.",
            );
        },
        Err(err) => {
            message::warning(format!(
                "Could not sync your default environment to FloxHub: {err:#}\nRun 'flox push' to retry.",
            ));
        },
    }
}

/// Divergence surfaces in two shapes depending on whether it is detected
/// before the push (metadata comparison) or by the server rejecting the ref
/// update mid-push.
fn error_is_diverged(err: &EnvironmentError) -> bool {
    matches!(
        err,
        EnvironmentError::ManagedEnvironment(ManagedEnvironmentError::Diverged(_))
            | EnvironmentError::ManagedEnvironment(ManagedEnvironmentError::Push(
                GitRemoteCommandError::Diverged
            ))
    )
}

/// On explicit `flox auth login`, reconcile the local and FloxHub state of
/// the default environment: push a local-only default, pre-fetch a
/// remote-only one, and explain the situation when both exist. Every failure
/// is a warning — authentication already succeeded.
pub(crate) async fn sync_default_env_after_login(flox: &mut Flox, handle: &str) {
    if !flox.features.auto_default {
        return;
    }
    if handle == floxhub_client::UNKNOWN_HANDLE {
        return;
    }

    match home_default_env(flox) {
        Some(HomeDefault::Managed(_, pointer)) => {
            if pointer.owner.as_str() != handle {
                message::info(format!(
                    "The default environment at '~/{DOT_FLOX}' belongs to '{owner}'; 'flox activate --default' will use '{handle}/{DEFAULT_NAME}'.",
                    owner = pointer.owner,
                ));
            }
            // The user's own checkout needs no login-time action: '--default'
            // resolves to it, and mutations sync as they happen.
        },
        Some(HomeDefault::Path(dot_flox)) => {
            let declined = read_user_state_file(user_state_path(flox))
                .ok()
                .flatten()
                .and_then(|state| state.confirmed_create_default_env)
                == Some(false);
            if declined {
                message::info(format!(
                    "Found a local default environment at '~/{DOT_FLOX}'. Not syncing it to FloxHub because you previously declined; run 'flox push' from your home directory to sync it.",
                ));
                return;
            }
            match open_remote_default(flox, handle, None) {
                Ok(_) => {
                    message::info(format!(
                        "Found a local default environment at '~/{DOT_FLOX}' and '{handle}/{DEFAULT_NAME}' on FloxHub. 'flox activate --default' will use the FloxHub environment.",
                    ));
                },
                Err(err) if error_is_upstream_not_found(&err) => {
                    if let Err(push_err) = push_home_default(flox, handle, &dot_flox) {
                        message::warning(format!(
                            "Could not sync your default environment to FloxHub: {push_err:#}\nRun 'flox push' from your home directory to retry.",
                        ));
                    }
                },
                Err(err) => {
                    debug!(%err, "could not check FloxHub for a default environment");
                },
            }
        },
        None => {
            // Pre-fetch a FloxHub default so '--default' works immediately
            // (and offline later). A warm cache means nothing was actually
            // fetched, so no message in that case.
            let owner = match EnvironmentOwner::from_str(handle) {
                Ok(owner) => owner,
                Err(err) => {
                    debug!(%err, "login handle is not a valid environment owner");
                    return;
                },
            };
            let pointer = ManagedPointer::new(owner, default_name(), &flox.floxhub);
            let was_cached = cached_default_checkout_is_valid(flox, &pointer);
            match open_remote_default(flox, handle, None) {
                Ok(_) if !was_cached => {
                    message::info(format!(
                        "Fetched your default environment '{handle}/{DEFAULT_NAME}' from FloxHub. Activate it with 'flox activate --default'.",
                    ));
                },
                Ok(_) => {},
                Err(err) if error_is_upstream_not_found(&err) => {},
                Err(err) => {
                    debug!(%err, "could not check FloxHub for a default environment");
                },
            }
        },
    }
}

fn default_name() -> EnvironmentName {
    DEFAULT_NAME
        .parse()
        .expect("'default' is a valid environment name")
}
