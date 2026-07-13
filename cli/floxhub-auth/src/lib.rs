//! FloxHub authentication.
//!
//! [`AuthContext`] is the central credential type threaded through the CLI:
//! it captures both the *kind* of authentication in use (Auth0 / PAT /
//! Kerberos) and the material available for that kind. It is built from the
//! configured [`AuthnMode`] and the stored token via
//! [`AuthContext::from_mode`].
//!
//! One file per type:
//! - [`auth_context`]: [`AuthContext`] and its failure types
//! - [`authn_mode`]: the configured [`AuthnMode`]
//! - [`token`]: [`FloxhubToken`] (decoded Auth0 JWT) and
//!   [`PersonalAccessToken`] (opaque `flox_pat_` token with lazy `/me`
//!   identity resolution)
//! - [`kerberos`]: [`KerberosMaterial`] and SPNEGO token generation
//! - [`accounts`]: the hand-written `GET /api/v1/accounts/me` request

mod accounts;
mod auth_context;
mod authn_mode;
mod kerberos;
mod token;

pub use accounts::{MeError, UserIdentity, fetch_me};
pub use auth_context::{AuthContext, AuthFailure, AuthHeaderError, UNKNOWN_HANDLE};
pub use authn_mode::AuthnMode;
pub use kerberos::{KerberosMaterial, TokenGenerator};
pub use token::{
    FloxhubToken,
    FloxhubTokenError,
    PAT_PREFIX,
    PersonalAccessToken,
    test_helpers as token_test_helpers,
};
