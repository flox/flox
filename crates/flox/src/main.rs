#[macro_use]
extern crate anyhow;

use self::config::{Feature, Impl};
use anyhow::Result;
use flox_rust_sdk::environment::default_nix_subprocess_env;
use log::{debug, info, warn};
use std::env;
use std::fmt::Debug;
use std::process::ExitStatus;

use tokio::process::Command;

mod build;
mod commands;
mod config;
mod utils;

use flox_rust_sdk::flox::FLOX_SH;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    set_user()?;

    let args = commands::flox_args().run();

    args.handle(config::Config::parse()?).await?;

    Ok(())
}

pub fn should_flox_forward(f: Feature) -> Result<bool> {
    if f.implementation()? == Impl::Bash {
        let env_name = format!(
            "FLOX_PREVIEW_FEATURES_{}",
            serde_variant::to_variant_name(&f)?.to_uppercase()
        );
        info!("`{env_name}` unset or not \"rust\", falling back to legacy flox");
        Ok(true)
    } else {
        Ok(false)
    }
}

pub async fn flox_forward() -> Result<()> {
    run_in_flox(&env::args_os().collect::<Vec<_>>()[1..]).await?;
    Ok(())
}

pub async fn run_in_flox(args: &[impl AsRef<std::ffi::OsStr> + Debug]) -> Result<ExitStatus> {
    debug!("Running in flox with arguments: {:?}", args);
    let status = Command::new(FLOX_SH)
        .args(args)
        .envs(&default_nix_subprocess_env())
        .spawn()
        .expect("failed to spawn flox")
        .wait()
        .await?;

    Ok(status)
}

/// Resets the `$USER`/`HOME` variables to match `euid`
///
/// Files written by `sudo flox ...` / `su`,
/// may write into your user's home (instead of /root).
/// Resetting `$USER`/`$HOME` will solve that.
fn set_user() -> Result<()> {
    {
        if let Some(effective_user) = nix::unistd::User::from_uid(nix::unistd::geteuid())? {
            if env::var("USER")? != effective_user.name {
                env::set_var("USER", effective_user.name);
                env::set_var("HOME", effective_user.dir);
            }
        } else {
            // Corporate LDAP environments rely on finding nss_ldap in
            // ld.so.cache *or* by configuring nscd to perform the LDAP
            // lookups instead. The Nix version of glibc has been modified
            // to disable ld.so.cache, so if nscd isn't configured to do
            // this then ldap access to the passwd map will not work.
            // Bottom line - don't abort if we cannot find a passwd
            // entry for the euid, but do warn because it's very
            // likely to cause problems at some point.
            warn!(
                "cannot determine effective uid - continuing as user '{}'",
                env::var("USER")?
            );
        };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_flox_help() {
        // TODO check the output
        assert_eq!(run_in_flox(&["--help"]).await.unwrap().code().unwrap(), 0,)
    }
}
