//! FloxHub token types, one per capability class: what a credential's
//! claims answer locally decides its type, and [`AuthContext::new_from_token`]
//! routes between them.
//!
//! - [`FloxhubToken`]: an Auth0-shaped JWT — handle and expiry claims
//!   present, identity answered locally.
//! - [`BareToken`]: a decodable JWT without the handle claim — `exp` and
//!   `sub` read opportunistically, identity resolved from `/me`.
//! - [`AccessToken`]: not decodable at all (`flox_`-prefixed tokens, or
//!   any opaque string an issuer mints) — everything resolved from `/me`.
//!
//! [`AuthContext::new_from_token`]: crate::auth::AuthContext::new_from_token

mod access_token;
mod bare_token;
mod floxhub_token;

pub(crate) use access_token::ACCESS_TOKEN_PREFIX;
pub use access_token::AccessToken;
pub use bare_token::BareToken;
pub use floxhub_token::FloxhubToken;
use thiserror::Error;

/// The string does not decode as a JWT of the attempted shape. Not
/// necessarily a bad credential — routing tries the next capability
/// class, and the server is the only authority on validity.
#[derive(Debug, Error)]
#[error("invalid token")]
pub struct InvalidTokenError(#[source] pub(crate) jsonwebtoken::errors::Error);

/// Test fixtures, re-exported from each type's own module.
#[cfg(any(test, feature = "tests"))]
pub mod test_helpers {
    pub use super::bare_token::test_helpers::*;
    pub use super::floxhub_token::test_helpers::*;
}
