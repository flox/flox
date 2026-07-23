//! FloxHub authentication.
//!
//! [`AuthContext`] is the central credential type threaded through the CLI:
//! it captures both the *kind* of authentication in use (Auth0 / PAT /
//! Kerberos) and the material available for that kind. It is built via
//! [`AuthContext::new_from_token`] (routing by the token's form) or
//! [`AuthContext::new_kerberos`].
//!
//! This module carries no transport: the identity behind an opaque token is
//! resolved at the point of use through `FloxhubClient::resolve_identity`
//! and surfaced uniformly via `Flox::get_identity`.
//!
//! One file per type:
//! - [`auth_context`]: [`AuthContext`] and its failure types
//! - [`identity`]: [`UserIdentity`] and its resolution errors
//! - [`token`]: [`FloxhubToken`] (decoded Auth0 JWT) and
//!   [`AccessToken`] (opaque `flox_`-prefixed token with lazy identity
//!   resolution)
//! - [`kerberos`]: [`KerberosMaterial`] and SPNEGO token generation

mod auth_context;
pub(crate) mod identity;
mod kerberos;
mod token;

pub use auth_context::{AuthContext, AuthFailure, AuthHeaderError};
pub use identity::{IdentityError, UNKNOWN_HANDLE, UserIdentity};
pub use kerberos::{KerberosMaterial, TokenGenerator};
pub use token::{AccessToken, FloxhubToken, FloxhubTokenError};

/// Test fixtures, re-exported from each type's own module.
#[cfg(any(test, feature = "tests"))]
pub mod test_helpers {
    pub use crate::auth::identity::test_helpers::*;
    pub use crate::auth::token::test_helpers::*;
}
