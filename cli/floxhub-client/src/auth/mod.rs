//! FloxHub authentication.
//!
//! [`AuthContext`] is the central credential type threaded through the CLI:
//! it captures both the *kind* of authentication in use (Auth0 / PAT /
//! Kerberos) and the material available for that kind. It is built from the
//! configured [`AuthnMode`] and the stored token via
//! [`AuthContext::from_mode`].
//!
//! This module carries no transport: the identity behind an opaque token is
//! resolved at the point of use through `FloxhubClient::resolve_identity`
//! and surfaced uniformly via `Flox::get_identity`.
//!
//! One file per type:
//! - [`auth_context`]: [`AuthContext`] and its failure types
//! - [`authn_mode`]: the configured [`AuthnMode`]
//! - [`identity`]: [`UserIdentity`] and its resolution errors
//! - [`token`]: [`FloxhubToken`] (decoded Auth0 JWT) and
//!   [`PersonalAccessToken`] (opaque `flox_pat_` token with lazy identity
//!   resolution)
//! - [`kerberos`]: [`KerberosMaterial`] and SPNEGO token generation

mod auth_context;
mod authn_mode;
pub(crate) mod identity;
mod kerberos;
mod token;

pub use auth_context::{AuthContext, AuthFailure, AuthHeaderError, UNKNOWN_HANDLE};
pub use authn_mode::AuthnMode;
pub use identity::{IdentityError, UserIdentity};
pub use kerberos::{KerberosMaterial, TokenGenerator};
pub use token::{FloxhubToken, FloxhubTokenError, PAT_PREFIX, PersonalAccessToken};

/// Test fixtures, re-exported from each type's own module.
///
/// Intentionally not behind `#[cfg(test)]` so that other crates' (also
/// non-gated) test helpers can use them without enabling a feature.
pub mod test_helpers {
    pub use crate::auth::identity::test_helpers::*;
    pub use crate::auth::token::test_helpers::*;
}
