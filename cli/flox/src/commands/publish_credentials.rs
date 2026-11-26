use std::path::PathBuf;
use std::process::ExitCode;

use bpaf::Bpaf;
use chrono::Utc;
use flox_rust_sdk::providers::nix_auth::CatalogAuth;
use flox_rust_sdk::providers::publish::upload_credentials::{
    RenewalCredential,
    RenewalRequest,
    serve,
};
use floxhub_client::{AuthContext, CatalogClientTrait};
use uuid::Uuid;

use crate::utils::init::init_floxhub_client;

/// Serve S3 upload credentials to `nix copy` during 'flox publish'.
///
/// Runs as the `credential_process` of the AWS profile 'flox publish' writes,
/// once per S3 request, so it runs before the usual startup (telemetry,
/// keyring, update checks) and answers from the cache when it can.
#[derive(Debug, Clone, Bpaf)]
pub struct PublishCredentials {
    /// The upload credential directory 'flox publish' prepared
    #[bpaf(long("dir"), argument("DIR"))]
    dir: PathBuf,
}

impl PublishCredentials {
    /// Print one credential set as `credential_process` JSON.
    ///
    /// Always exits successfully; a failure is recorded in the directory for
    /// 'flox publish' to report. See `upload_credentials`.
    pub fn run(&self) -> ExitCode {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(err) => {
                eprintln!("could not start the credential helper: {err}");
                return ExitCode::FAILURE;
            },
        };
        let credentials = runtime.block_on(serve(&self.dir, Utc::now(), renew));
        match serde_json::to_string(&credentials) {
            Ok(json) => {
                println!("{json}");
                ExitCode::SUCCESS
            },
            Err(err) => {
                eprintln!("could not serialize upload credentials: {err}");
                ExitCode::FAILURE
            },
        }
    }
}

/// Ask FloxHub for a fresh upload session for the same catalog and package,
/// authenticated the way 'flox publish' was.
async fn renew(request: &RenewalRequest) -> Result<CatalogAuth, String> {
    let auth_context = match &request.credential {
        RenewalCredential::Token(token) => AuthContext::new_from_token(Some(token)),
        RenewalCredential::Kerberos => AuthContext::new_kerberos(),
        RenewalCredential::None => AuthContext::new_from_token(None),
    };
    let client = init_floxhub_client(
        request.floxhub_api_url.clone(),
        auth_context,
        None,
        Uuid::new_v4(),
    )
    .map_err(|err| format!("{err:#}"))?;

    // The client retries transient failures itself.
    let response = client
        .publish_info(&request.catalog, &request.package)
        .await
        .map_err(|err| err.to_string())?;
    response
        .ingress_auth
        .ok_or_else(|| "FloxHub returned no upload credentials".to_string())
}
