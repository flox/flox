//! Auth0 credential creation

use crate::auth::Credential;
use crate::token::FloxhubToken;

/// Create a credential from an Auth0 token.
pub fn auth0_credential(token: Option<FloxhubToken>) -> Credential {
    match token {
        Some(t) => Credential::Bearer(t),
        None => Credential::None,
    }
}
