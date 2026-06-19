use std::collections::BTreeSet;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Result, bail};
use clap::Parser;
use flox_core::util::message::format_error;
use floxhub_client::{
    AuthContext,
    AuthnMode,
    DEFAULT_CATALOG_URL,
    FloxhubClient,
    FloxhubClientConfig,
    FloxhubClientError,
    FloxhubMockMode,
};
use nef_lock_catalog::{
    CatalogRef,
    LockError,
    lock_references,
    render_unresolvable,
    scan_package,
    write_lock,
};
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Environment variable holding the FloxHub auth token.
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

    /// Path to write the resulting build lock to.
    #[arg(long)]
    out: PathBuf,

    /// Catalog stability channel.
    #[arg(long, default_value = "stable")]
    stability: String,
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::registry()
        // NEW logs each span once at creation with its fields
        .with(tracing_subscriber::fmt::layer()
            .with_ansi(std::io::stderr().is_terminal())
            .with_span_events(FmtSpan::NEW))
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

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
        out = %cli.out.display(),
        stability = cli.stability,
    )
)]
async fn run(cli: Cli) -> Result<()> {
    // Read the token once; whether it is present selects the auth-hint wording
    // and needs no second environment read.
    let floxhub_token = std::env::var(ENV_FLOXHUB_TOKEN).ok();
    let token_present = floxhub_token.is_some();

    let client = build_client(floxhub_token)?;

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

    write_lock(&lock, &cli.out)?;
    Ok(())
}

/// Build the catalog client from the environment and the already-read token.
///
/// The catalog URL is resolved once (`FLOX_CATALOG_URL`, else the default) and
/// used for the request base; mock mode is environment-driven. The token is
/// read once by the caller (see [ENV_FLOXHUB_TOKEN]) and passed in.
fn build_client(floxhub_token: Option<String>) -> Result<FloxhubClient> {
    let catalog_url =
        std::env::var("FLOX_CATALOG_URL").unwrap_or_else(|_| DEFAULT_CATALOG_URL.to_string());

    let floxhub_token = floxhub_token.map(|token| token.parse()).transpose()?;
    let auth_context = AuthContext::from_mode(&AuthnMode::Auth0, floxhub_token);

    let config = FloxhubClientConfig {
        base_url: catalog_url,
        extra_headers: Default::default(),
        mock_mode: FloxhubMockMode::default_from_env(),
        auth_context,
        user_agent: None,
    };

    Ok(FloxhubClient::new(config)?)
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
        body.push_str(
            "\n\n  FLOXHUB_TOKEN is not set. If the catalog requires authentication, set it \
             to a\n  valid token (this helper cannot log in for you) and retry.",
        );
    }
    body
}
