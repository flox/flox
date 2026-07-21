use std::backtrace::BacktraceStatus;
use std::env;
use std::fmt::{Debug, Display};
use std::process::ExitCode;
use std::time::Instant;

use anyhow::Result;
use bpaf::{Args, Parser};
use commands::{
    EnvironmentSelectError,
    FloxArgs,
    FloxCli,
    Interrupted,
    NoEnvironmentError,
    Prefix,
    Version,
};
use flox_config::Config;
use flox_core::sentry::init_sentry;
use flox_core::vars::{FLOX_VERSION_STRING, FLOX_VERSION_VAR};
use flox_events::{EventsHub, LifecycleFields};
use flox_rust_sdk::flox::FLOX_VERSION;
use flox_rust_sdk::models::environment::EnvironmentError;
use flox_rust_sdk::models::environment::managed_environment::ManagedEnvironmentError;
use flox_rust_sdk::models::environment::remote_environment::RemoteEnvironmentError;
use flox_rust_sdk::providers::services::process_compose::ServiceError;
use tracing::{debug, warn};
use utils::errors::format_service_error;
use utils::init::init_logger;
use utils::{message, populate_default_nix_env_vars};

use crate::utils::errors::{
    format_environment_select_error,
    format_error,
    format_managed_error,
    format_remote_error,
};
use crate::utils::init::init_telemetry_uuid;
use crate::utils::metrics::{Hub, read_metrics_uuid};

mod commands;
mod utils;

async fn run(args: FloxArgs) -> Result<()> {
    populate_default_nix_env_vars();
    let config = Config::parse()?;
    args.handle(config).await?;
    Ok(())
}

fn main() -> ExitCode {
    // Avoid SIGPIPE from killing the process
    reset_sigpipe();

    // Eagerly evaluate version and prevent it from propagating to sub-processes.
    let _ = *FLOX_VERSION_STRING;
    // SAFETY: Writing to the environment is safe here since we can guarantee
    // not to look up the env concurrently,
    // because at this point the program is still single threaded.
    unsafe {
        env::remove_var(FLOX_VERSION_VAR);
    }

    // Override bpaf's bash completion script to fix unsafe eval.
    // bpaf's generated script interpolates COMP_WORDS into a string
    // and passes it to `eval`, so unclosed quotes in user input
    // (e.g. `flox activate -c "bas<TAB>`) cause parse errors.
    // Our version uses array-based argument passing instead.
    // Upstream issue: https://github.com/pacak/bpaf/issues/440
    // This must run before any bpaf parser call (Prefix::check, etc.)
    // because bpaf's ArgScanner intercepts this flag and calls
    // process::exit(0) directly.
    if env::args_os().any(|a| a == "--bpaf-complete-style-bash") {
        print!("{}", BASH_COMPLETION_SCRIPT);
        return ExitCode::from(0);
    }

    // Quit early if `--prefix` is present
    if Prefix::check() {
        println!(env!("out"));
        return ExitCode::from(0);
    }

    // Quit early if `--version` is present
    if Version::check() {
        println!("{}", *FLOX_VERSION);
        return ExitCode::from(0);
    }

    // Parse verbosity flags to affect help message/parse errors
    let verbosity = {
        let verbosity_parser = commands::verbosity();
        let other_parser = bpaf::any("_", Some::<String>).many();

        bpaf::construct!(verbosity_parser, other_parser)
            .map(|(v, _)| v)
            .to_options()
            .run_inner(Args::current_args())
            .unwrap_or_default()
    };

    init_logger(Some(verbosity));
    debug!("FLOX_VERSION={}", *FLOX_VERSION);

    if let Err(err) = set_user() {
        message::error(err.to_string());
        return ExitCode::from(1);
    }

    let config = Config::parse().unwrap_or_default();
    let metrics_uuid = if !config.flox.disable_metrics {
        init_telemetry_uuid(&config.flox.data_dir, &config.flox.cache_dir)
            .and_then(|_| read_metrics_uuid(&config))
            .inspect_err(|e| warn!("Failed to initialize metrics UUID: {e}"))
            .ok()
    } else {
        None
    };

    // Sentry client must be initialized before starting an async runtime or spawning threads
    // https://docs.sentry.io/platforms/rust/#async-main-function
    let _sentry_guard = metrics_uuid.map(|uuid| init_sentry("flox-cli", uuid));
    let _metrics_guard = Hub::global().try_guard().ok();
    let _v2_events_guard = EventsHub::global().try_guard().ok();

    // Pass down the verbosity level to all sub-processes
    unsafe {
        std::env::set_var(
            "_FLOX_SUBSYSTEM_VERBOSITY",
            format!("{}", verbosity.to_i32()),
        );
    }
    debug!("set _FLOX_SUBSYSTEM_VERBOSITY={}", verbosity.to_i32());

    // Run the argument parser
    //
    // Pass through Completion "failure"; In completion mode this needs to be printed as is
    // to work with the shell completion frontends
    //
    // Pass through Stdout failure; This represents `--help`
    // todo: just `run()` the parser? Unless we still need to control which std{err/out} to use
    let args = commands::flox_cli().run_inner(Args::current_args());

    if let Some(parse_err) = args.as_ref().err() {
        match parse_err {
            bpaf::ParseFailure::Stdout(m, _) => {
                print!("{m:80}");
                return ExitCode::from(0);
            },
            bpaf::ParseFailure::Stderr(m) => {
                message::error(format!("{m:80}"));
                return ExitCode::from(1);
            },
            bpaf::ParseFailure::Completion(c) => {
                print!("{c}");
                return ExitCode::from(0);
            },
        }
    }

    // Errors handled above
    let FloxCli(args) = args.unwrap();

    let v2_subcommand = args.subcommand_name();

    // Runtime creates our SIGINT/Ctrl-C handler, so care must be taken to drop it last
    let runtime = tokio::runtime::Runtime::new().unwrap();

    let dispatch_start = Instant::now();
    let result = runtime.block_on(run(args));

    // Print errors; derive the exit code and the telemetry `error_kind`.
    let (code, error_kind): (u8, Option<&'static str>) = match &result {
        Ok(()) => (0, None),

        // Do not print any error for a controlled exit
        Err(e) if e.is::<Exit>() => (e.downcast_ref::<Exit>().unwrap().0, Some("controlled_exit")),

        Err(e) => {
            let kind = if let Some((message, kind)) = display_and_kind(e) {
                message::error(message);
                kind
            } else {
                // unknown errors are printed with an error trace
                let err_str = e
                    .chain()
                    .skip(1)
                    .fold(e.to_string(), |acc, cause| format!("{acc}: {cause}"));
                message::error(err_str);
                "uncategorized"
            };

            if matches!(e.backtrace().status(), BacktraceStatus::Captured) {
                eprintln!("{}", e.backtrace());
            }

            (1, Some(kind))
        },
    };

    // Emit the v2 `cli.command_completed`. The hub no-ops when no client was
    // installed (e.g. a bare `flox` invocation) or when `activate.rs`
    // recorded the pre-exec completion before `exec`.
    if let Err(err) = flox_events::EventsHub::global().record_command_completed(
        v2_subcommand.to_string(),
        LifecycleFields {
            exit_code: i32::from(code),
            duration_ms: Some(duration_to_ms(dispatch_start.elapsed())),
            error_kind: error_kind.map(String::from),
        },
    ) {
        debug!(error = %err, "Failed to record v2 cli.command_completed event");
    }

    drop(_v2_events_guard);
    drop(_metrics_guard);
    drop(_sentry_guard);

    ExitCode::from(code)

    // drop(runtime) should implicitly be last
}

/// Display message and telemetry `error_kind` for a typed dispatch error;
/// `None` when the error has no typed match. The kinds are the strum-derived
/// namespaced variant slugs, so classification cannot drift from this ladder.
fn display_and_kind(e: &anyhow::Error) -> Option<(String, &'static str)> {
    e.downcast_ref::<NoEnvironmentError>()
        .map(|err| (err.to_string(), kind_of(err)))
        .or_else(|| {
            e.downcast_ref::<EnvironmentError>()
                .map(|err| (format_error(err), kind_of(err)))
        })
        .or_else(|| {
            e.downcast_ref::<ManagedEnvironmentError>()
                .map(|err| (format_managed_error(err), kind_of(err)))
        })
        .or_else(|| {
            e.downcast_ref::<RemoteEnvironmentError>()
                .map(|err| (format_remote_error(err), kind_of(err)))
        })
        .or_else(|| {
            e.downcast_ref::<EnvironmentSelectError>()
                .map(|err| (format_environment_select_error(err), kind_of(err)))
        })
        .or_else(|| {
            e.downcast_ref::<ServiceError>()
                .map(|err| (format_service_error(err), kind_of(err)))
        })
        .or_else(|| {
            // `Interrupted` is a struct, so it has no strum variant slug.
            e.downcast_ref::<Interrupted>()
                .map(|err| (err.to_string(), "interrupted"))
        })
}

/// The strum-derived namespaced variant slug for `e`.
fn kind_of<T>(e: &T) -> &'static str
where
    for<'a> &'a T: Into<&'static str>,
{
    e.into()
}

/// Saturate a duration into whole ms (u64::MAX ms is ~584M years — a
/// ceiling only).
fn duration_to_ms(elapsed: std::time::Duration) -> u64 {
    u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
}

/// Fixed bash completion script that replaces bpaf's generated version.
///
/// Workaround for <https://github.com/pacak/bpaf/issues/440>.
///
/// bpaf's script does:
///   line="$1 --bpaf-complete-rev=8 ${COMP_WORDS[@]:1}"
///   source <( eval ${line})
///
/// The unquoted ${COMP_WORDS[@]:1} interpolation means special characters
/// in user input (unclosed quotes, backticks, etc.) are interpreted by eval.
///
/// Our version passes each COMP_WORD as a separate array element, avoiding
/// eval entirely. This correctly preserves word boundaries and handles all
/// special characters.
const BASH_COMPLETION_SCRIPT: &str = r#"_bpaf_dynamic_completion()
{
    local -a _args=("$1" "--bpaf-complete-rev=8" "${COMP_WORDS[@]:1}")
    source <( "${_args[@]}" )
}
complete -o nosort -F _bpaf_dynamic_completion flox
"#;

/// Error to exit without printing an error message. Carries the process exit
/// code as a `u8` (rather than an opaque [`ExitCode`]) so telemetry can
/// report the code the process actually exits with.
#[derive(Debug)]
struct Exit(u8);
impl Display for Exit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        <Self as Debug>::fmt(self, f)
    }
}
impl std::error::Error for Exit {}

/// Resets the `$USER`/`HOME` variables to match `euid`
///
/// Files written by `sudo flox ...` / `su`,
/// may write into your user's home (instead of /root).
/// Resetting `$USER`/`$HOME` will solve that.
fn set_user() -> Result<()> {
    {
        let effective_uid = nix::unistd::geteuid();
        let user_var = env::var("USER").unwrap_or_default();

        if let Some(effective_user) = nix::unistd::User::from_uid(nix::unistd::geteuid())? {
            if user_var != effective_user.name {
                debug!(user_old = %user_var, user = %effective_user.name, home = ?effective_user.dir, "Resetting USER and HOME environment variables");
                unsafe {
                    env::set_var("USER", effective_user.name);
                    env::set_var("HOME", effective_user.dir);
                }
            }
        } else {
            // Corporate LDAP environments rely on finding nss_ldap in
            // ld.so.cache *or* by configuring nscd to perform the LDAP
            // lookups instead. The Nix version of glibc has been modified
            // to disable ld.so.cache, so if nscd isn't configured to do
            // this then ldap access to the passwd map will not work.
            // Bottom line - don't abort or warn if we cannot find a passwd
            // entry for the euid, but do log it in debug output so that we can
            // diagnose whether it has contributed to user reported issues.
            debug!(euid = %effective_uid, user = %user_var, "Unable to get passwd entry for USER and HOME check");
        };
        Ok(())
    }
}

/// Avoid SIGPIPE from killing the process
///
/// SECURITY: This is safe because we are setting the signal handler to the default
fn reset_sigpipe() {
    unsafe {
        nix::libc::signal(nix::libc::SIGPIPE, nix::libc::SIG_DFL);
    }
}

#[cfg(test)]
mod error_kind_tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn kind_is_namespaced_variant_slug() {
        let err = anyhow::Error::from(EnvironmentError::ManifestNotFound);
        let (_, kind) = display_and_kind(&err).expect("typed error classifies");
        assert_eq!(kind, "environment.manifest_not_found");
    }

    #[test]
    fn no_environment_error_kind() {
        let err = anyhow::Error::from(NoEnvironmentError::CurrentDirectory);
        let (_, kind) = display_and_kind(&err).expect("typed error classifies");
        assert_eq!(kind, "no_environment.current_directory");
    }

    /// The derived kind is the outer variant: a wrapped `EnvironmentError`
    /// classifies as `env_select.environment_error`, not its inner variant.
    #[test]
    fn nested_environment_error_reports_outer_variant() {
        let inner = EnvironmentError::DotFloxNotFound(PathBuf::from("/tmp/project/.flox"));
        let err = anyhow::Error::from(EnvironmentSelectError::EnvironmentError(inner));
        let (_, kind) = display_and_kind(&err).expect("typed error classifies");
        assert_eq!(kind, "env_select.environment_error");
    }

    #[test]
    fn interrupted_classifies_with_original_message() {
        let err = anyhow::Error::from(Interrupted);
        let (message, kind) = display_and_kind(&err).expect("typed error classifies");
        assert_eq!(kind, "interrupted");
        assert_eq!(message, "user interrupted process");
    }

    #[test]
    fn untyped_error_has_no_kind_and_no_pii() {
        let err = anyhow::anyhow!("could not open /Users/alice/secret.toml");
        assert_eq!(display_and_kind(&err), None);
    }
}
