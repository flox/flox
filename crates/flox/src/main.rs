#[macro_use]
extern crate anyhow;

use self::config::{Feature, Impl};
use anyhow::Result;
use commands::FloxArgs;
use flox_rust_sdk::environment::default_nix_subprocess_env;
use log::{debug, error, info, warn};
use std::env;
use std::fmt::{Debug, Display};
use std::process::{ExitCode, ExitStatus};

use tokio::process::Command;

mod build;
mod commands;
mod config;
mod utils;

use flox_rust_sdk::flox::FLOX_SH;

async fn run(args: FloxArgs) -> Result<()> {
    set_user()?;
    args.handle(config::Config::parse()?).await?;
    Ok(())
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = commands::flox_args().run();
    let debug = args.debug;

    match run(args).await {
        Ok(()) => ExitCode::from(0),
        Err(e) => {
            // Do not print any error if caused by wrapped flox (sh)
            if e.is::<FloxShellErrorCode>() {
                return e.downcast_ref::<FloxShellErrorCode>().unwrap().0;
            }
            if debug {
                error!("{:#?}", e);
            } else {
                error!("{}", e);
            }
            ExitCode::from(1)
        }
    }
}

pub fn should_flox_forward(f: Feature) -> Result<bool> {
    if f.implementation()? == Impl::Bash {
        let env_name = format!(
            "FLOX_PREVIEW_FEATURES_{}",
            serde_variant::to_variant_name(&f)?.to_uppercase()
        );
        debug!("`{env_name}` unset or not \"rust\", falling back to legacy flox");
        Ok(true)
    } else {
        Ok(false)
    }
}

#[derive(Debug)]
struct FloxShellErrorCode(ExitCode);
impl Display for FloxShellErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        <Self as Debug>::fmt(&self, f)
    }
}
impl std::error::Error for FloxShellErrorCode {}

pub async fn flox_forward() -> Result<()> {
    let result = run_in_flox(&env::args_os().collect::<Vec<_>>()[1..]).await?;
    if !result.success() {
        Err(FloxShellErrorCode(ExitCode::from(
            result.code().expect("Process terminated by signal") as u8,
        )))?;
    }
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
