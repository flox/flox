//! Kerberos authentication material and SPNEGO token generation.

use std::sync::Arc;

use url::Url;

use crate::auth_context::AuthHeaderError;

/// A function that generates a SPNEGO token for a given URL.
pub type TokenGenerator = Arc<dyn Fn(&Url) -> Result<String, AuthHeaderError> + Send + Sync>;

/// Material for Kerberos authentication.
#[derive(Clone)]
pub struct KerberosMaterial {
    /// The resolved principal name.
    pub principal: String,
    /// A function to generate SPNEGO tokens.
    pub generate_token: TokenGenerator,
}

mod credential;

pub(crate) use credential::kerberos_credential;
