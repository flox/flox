use std::collections::BTreeSet;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Result, bail};
use clap::Parser;
use flox_config::{Config, FloxConfig};
use flox_core::floxhub::{DEFAULT_FLOXHUB_URL, Floxhub};
use flox_core::util::message::format_error;
use floxhub_client::{
    AuthContext,
    AuthnMode,
    FloxhubClient,
    FloxhubClientConfig,
    FloxhubClientError,
    FloxhubMockMode,
};
use nef_lock_catalog::{
    CatalogRef,
    LockError,
    lock_references,
    render_lock,
    render_unresolvable,
    scan_package,
    write_lock,
};
use tracing::debug;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Environment variable holding the FloxHub auth token, referenced in error
/// hints. The value itself arrives via the layered config (`FLOX_*` env
/// variables override the config files).
const ENV_FLOXHUB_TOKEN: &str = "FLOX_FLOXHUB_TOKEN";

/// Lock the catalog inputs of one or more NEF package expressions and write the
/// resulting build lock. Invoked as a libexec helper by the package builder.
#[derive(Parser)]
struct Cli {
    /// Package-set root that `--rel-path` values are resolved against.
    #[arg(long)]
    base_dir: PathBuf,

    /// NEF package expression to lock, relative to `--base-dir`. Repeatable;
    /// multiple paths union their catalog references (manifest aggregation).
    #[arg(long = "rel-path", required = true)]
    rel_paths: Vec<PathBuf>,

    /// Path to write the resulting build lock to. When omitted, the lock is
    /// printed to stdout.
    #[arg(long)]
    out: Option<PathBuf>,

    /// Catalog stability channel.
    #[arg(long, default_value = "stable")]
    stability: String,

    /// Explain each step: files read, catalog references found (with source
    /// locations), the resolved catalog endpoint, and the full lookup request
    /// body. All diagnostics go to stderr; the lock is still written to `--out`.
    #[arg(short, long)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    // `--verbose` raises this crate's log level to `debug` on top of any
    // `RUST_LOG` setting; the file/reference/request diagnostics are emitted at
    // that level. Diagnostics go to stderr so `--out /dev/stdout` stays clean.
    let mut filter = tracing_subscriber::EnvFilter::from_default_env();
    if cli.verbose {
        filter = filter
            .add_directive("nef_lock_catalog=debug".parse().expect("valid directive"))
            .add_directive("lock=debug".parse().expect("valid directive"));
    }

    tracing_subscriber::registry()
        // NEW logs each span once at creation with its fields
        .with(tracing_subscriber::fmt::layer()
            .with_writer(std::io::stderr)
            .with_ansi(std::io::stderr().is_terminal())
            .with_span_events(FmtSpan::NEW))
        .with(filter)
        .init();

    match run(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        // Decorate the rendered message body with `✘ ERROR:` here — the single
        // place presentation is applied.
        Err(err) => {
            eprintln!("{}", format_error(err));
            ExitCode::FAILURE
        },
    }
}

#[tracing::instrument(
    skip_all,
    fields(
        base_dir = %cli.base_dir.display(),
        rel_paths = cli.rel_paths.len(),
        out = cli.out.as_deref().map(|out| out.display().to_string()).unwrap_or_else(|| "<stdout>".to_string()),
        stability = cli.stability,
    )
)]
async fn run(cli: Cli) -> Result<()> {
    // The layered config gives this helper the same view as the CLI:
    // defaults, /etc/flox.toml, the user flox.toml and FLOX_* env variables.
    let config = Config::parse()?;

    // Read the token once; whether it is present selects the auth-hint wording.
    // CLI parity: an empty token counts as unset (mk_data exports
    // FLOX_FLOXHUB_TOKEN="").
    let floxhub_token = config
        .flox
        .floxhub_token
        .clone()
        .filter(|token| !token.is_empty());
    let token_present = floxhub_token.is_some();

    let client = build_client(&config.flox, floxhub_token)?;

    // Union the catalog references discovered across every rel-path. Multiple
    // rel-paths model a manifest build aggregating its NEF dependencies.
    let references: BTreeSet<CatalogRef> = cli
        .rel_paths
        .iter()
        .flat_map(|rel| scan_package(&cli.base_dir, rel))
        .collect();

    // Render each failure to its message body at the boundary, while the
    // structured data is still in hand; `main` adds the `✘ ERROR:` decoration.
    let lock = match lock_references(&client, references, &cli.stability).await {
        Ok(lock) => lock,
        // REQ-013: surface the unresolvable dependency chains.
        Err(LockError::Unresolvable(entries)) => bail!(render_unresolvable(&entries)),
        // An auth failure can only surface as an `APIError` (the sole variant
        // carrying an HTTP status); only then is the token-state hint relevant.
        //
        // NOTE: `build_inputs_lookup` currently maps every failure to `Other`
        // (the lookup endpoint's `HttpValidationError` schema divergence), so no
        // `APIError` reaches here yet and the hint stays dormant until the
        // backend declares `ErrorResponse` on that endpoint.
        Err(LockError::Client(source @ FloxhubClientError::APIError(_))) => {
            bail!(render_client_error(&format!("{source:#}"), token_present))
        },
        // Any other lock failure needs no special rendering.
        Err(other) => return Err(other.into()),
    };

    // Write to the requested file, or print to stdout when `--out` is omitted
    // (convenient for firing the helper off by hand on the command line).
    match cli.out {
        Some(out) => write_lock(&lock, &out)?,
        None => println!("{}", render_lock(&lock)?),
    }
    Ok(())
}

/// Build the catalog client from the layered config and the already-read token.
///
/// The catalog API base URL is derived with the *same* implementation as the
/// CLI: [`Floxhub`] turns the configured base (`floxhub_url`, else the
/// compiled-in [`DEFAULT_FLOXHUB_URL`]) into the API base (`api_url_str`),
/// with `catalog_url` as the API override. The generated client then joins
/// `/api/v1/catalog/...` onto it. Mock mode is environment-driven. The token
/// is read once by the caller and passed in.
fn build_client(config: &FloxConfig, floxhub_token: Option<String>) -> Result<FloxhubClient> {
    let floxhub = Floxhub::new(
        config
            .floxhub_url
            .clone()
            .unwrap_or_else(|| DEFAULT_FLOXHUB_URL.clone()),
        config.catalog_url.clone(),
        None,
    )?;
    let catalog_url = floxhub.api_url_str();
    let mock_mode = FloxhubMockMode::default_from_env();

    // Connection details for `--verbose`: the resolved base, the path the
    // generated client appends, whether a token is attached, and whether a mock
    // is intercepting the request.
    debug!(
        catalog_base_url = %catalog_url,
        lookup_path = "/api/v1/catalog/build-inputs/lookup",
        has_token = floxhub_token.is_some(),
        mock = ?mock_mode,
        "configured catalog client",
    );

    let floxhub_token = floxhub_token.map(|token| token.parse()).transpose()?;
    let auth_context = AuthContext::from_mode(&effective_authn_mode(config)?, floxhub_token);

    let config = FloxhubClientConfig {
        base_url: catalog_url,
        extra_headers: Default::default(),
        mock_mode,
        auth_context,
        user_agent: None,
    };

    Ok(FloxhubClient::new(config)?)
}

/// Resolve the configured authn mode to the client's, applying the
/// compiled-in default when unset.
///
/// The config enum always parses both modes; the client enum only carries the
/// modes compiled into this build.
/// The config enum always parses both modes; the client enum only carries the
/// modes compiled into this build.
fn effective_authn_mode(config: &Config) -> Result<AuthnMode> {
    match config.flox.floxhub_authn_mode {
        None => Ok(AuthnMode::default()),
        Some(flox_config::AuthnMode::Auth0) => Ok(AuthnMode::Auth0),
        #[cfg(feature = "floxhub-authn-kerberos")]
        Some(flox_config::AuthnMode::Kerberos) => Ok(AuthnMode::Kerberos),
        #[cfg(not(feature = "floxhub-authn-kerberos"))]
        Some(flox_config::AuthnMode::Kerberos) => Err(anyhow!(
            "Kerberos authentication is not supported by this build."
        )),
    }
}

/// Render an authentication-related catalog failure with a token-aware hint.
/// Only called for an `APIError` (see [run]), so the hint is always warranted;
/// `token_present` only selects its wording. This helper runs non-interactively
/// and cannot log in, so it can only tell the developer how to fix
/// authentication themselves. Returns the message body; `main` adds the
/// `✘ ERROR:` decoration.
fn render_client_error(message: &str, token_present: bool) -> String {
    let mut body = format!("catalog request failed: {message}");
    if token_present {
        body.push_str(
            "\n\n  If this is an authentication failure, your token may be expired or \
             lack access;\n  refresh it with `flox auth login` and retry.",
        );
    } else {
        body.push_str(&format!(
            "\n\n  {ENV_FLOXHUB_TOKEN} is not set. If the catalog requires authentication, set it \
             to a\n  valid token (this helper cannot log in for you) and retry.",
        ));
    }
    body
}
