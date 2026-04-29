//! AuthContext construction from the configured auth method.

use crate::auth::{AuthContext, AuthnMode};
use crate::token::FloxhubToken;

// Conditionally include Kerberos
#[cfg(feature = "floxhub-authn-kerberos")]
mod kerberos;

impl AuthContext {
    /// Create an [`AuthContext`] for the given [`AuthnMode`].
    ///
    /// - Auth0: wraps the FloxHub token as a bearer credential.
    /// - Kerberos: resolves the principal and embeds a SPNEGO token generator;
    ///   returns `Kerberos(None)` (with a warning log) if the ticket cannot be
    ///   resolved.
    pub fn from_mode(mode: &AuthnMode, floxhub_token: Option<FloxhubToken>) -> Self {
        match mode {
            AuthnMode::Auth0 => AuthContext::Auth0(floxhub_token),
            #[cfg(feature = "floxhub-authn-kerberos")]
            AuthnMode::Kerberos => kerberos::kerberos_credential(),
        }
    }
}
