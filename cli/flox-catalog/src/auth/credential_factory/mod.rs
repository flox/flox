//! Credential factory — creates credentials from the configured auth method.

use super::Credential;
use crate::token::FloxhubToken;

// Always include Auth0
mod auth0;

// Conditionally include Kerberos
#[cfg(feature = "floxhub-authn-kerberos")]
mod kerberos;

/// Create a credential for the given [`AuthMethod`](super::AuthMethod).
///
/// - Auth0: wraps the FloxHub token as a bearer credential
/// - Kerberos: resolves the principal and embeds a SPNEGO token generator
pub fn credential_from_method(
    method: &super::AuthMethod,
    floxhub_token: Option<FloxhubToken>,
    #[cfg_attr(not(feature = "floxhub-authn-kerberos"), allow(unused_variables))]
    catalog_url: String,
) -> Credential {
    match method {
        super::AuthMethod::Auth0 => auth0::auth0_credential(floxhub_token),
        #[cfg(feature = "floxhub-authn-kerberos")]
        super::AuthMethod::Kerberos => kerberos::kerberos_credential(),
    }
}
