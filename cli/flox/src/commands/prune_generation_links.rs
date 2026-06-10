//! Pruning of managed-environment generation GC-root links (flox#4332).
//!
//! Shared by two callers:
//! - `flox gc` (interactive), which prunes synchronously before running the
//!   nix store GC — see [`prune_registered_environments`].
//! - the hidden `prune-generation-links` worker ([`PruneGenerationLinks`]),
//!   which the activation executive fires periodically in the background and
//!   which does **not** run the nix store GC.

use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::env_registry::{env_registry_path, read_environment_registry};
use flox_rust_sdk::models::environment::managed_environment::ManagedEnvironment;
use flox_rust_sdk::models::environment::{
    EnvironmentPointer,
    PrunePolicy,
    live_activation_store_paths,
};
use fslock::LockFile;
use tracing::{debug, instrument};

use crate::subcommand_metric;

/// Hidden worker: prune aged generation GC-root links across all registered
/// managed environments, **without** running the nix store GC.
///
/// The activation executive fires this periodically (see the executive's
/// background prune). The interactive `flox gc` instead calls
/// [`prune_registered_environments`] directly and then runs the store GC.
#[derive(Bpaf, Clone, Debug)]
pub struct PruneGenerationLinks {}

impl PruneGenerationLinks {
    #[instrument(name = "prune-generation-links", skip_all)]
    pub fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("prune-generation-links");

        // Multiple executives may fire this concurrently. The prune is
        // idempotent, but take a non-blocking lock so they don't race to remove
        // the same links; if another prune already holds it, this run would be
        // redundant, so just exit.
        let lock_path = flox.runtime_dir.join("generation-prune.lock");
        let mut lock = LockFile::open(&lock_path)?;
        if !lock.try_lock()? {
            debug!("another generation-link prune is in progress; skipping");
            return Ok(());
        }

        prune_registered_environments(&flox, PrunePolicy::default_aged_out());
        // The lock is released when `lock` is dropped.
        Ok(())
    }
}

/// Prune managed-environment generation GC-root links across all registered
/// environments according to `policy`, protecting links that live activations
/// depend on ([`live_activation_store_paths`]).
///
/// Best-effort throughout: failing to read the registry, open an environment,
/// or remove a link is logged and skipped so it never aborts the caller.
pub fn prune_registered_environments(flox: &Flox, policy: PrunePolicy) {
    let protected = live_activation_store_paths(&flox.runtime_dir);

    let registry = match read_environment_registry(env_registry_path(flox)) {
        Ok(Some(registry)) => registry,
        Ok(None) => return,
        Err(err) => {
            debug!(%err, "failed to read environment registry for generation-link pruning");
            return;
        },
    };

    for entry in &registry.entries {
        let Some(registered) = entry.latest_env() else {
            continue;
        };
        // Only managed environments have generations / generation links.
        let EnvironmentPointer::Managed(pointer) = &registered.pointer else {
            continue;
        };

        let env = match ManagedEnvironment::open(flox, pointer.clone(), &entry.path, None) {
            Ok(env) => env,
            Err(err) => {
                debug!(path = %entry.path.display(), %err, "skipping environment for generation-link pruning");
                continue;
            },
        };

        match env.prune_generation_links(policy, &protected) {
            Ok(removed) if !removed.is_empty() => {
                debug!(count = removed.len(), path = %entry.path.display(), "pruned generation links");
            },
            Ok(_) => {},
            Err(err) => {
                debug!(path = %entry.path.display(), %err, "failed to prune generation links");
            },
        }
    }
}
