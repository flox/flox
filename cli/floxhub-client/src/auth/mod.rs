//! FloxHub authentication.
//!
//! [`AuthContext`] is the central credential type threaded through the CLI:
//! it captures both the *kind* of authentication in use (Auth0 / PAT /
//! Kerberos) and the material available for that kind. It is built via
//! [`AuthContext::new_from_token`] (routing by the token's form) or
//! [`AuthContext::new_kerberos`].
//!
//! Credential loading and cached identity resolution are handled by
//! [`AuthContext`]. Callers request a handle or identity directly.
//! Storage adapters use the separate [storage] integration API.

mod auth_context;
mod credential;
mod discovery;
pub(crate) mod identity;
mod kerberos;
pub mod storage;
mod token;

pub use auth_context::AuthContext;
pub use credential::{AuthFailure, AuthHeaderError, Credential, CredentialKind};
pub use discovery::{DiscoveredLoginConfig, LoginDiscoveryError, discover_login_config};
pub use identity::{UNKNOWN_HANDLE, UserIdentity};
pub use kerberos::{KerberosMaterial, TokenGenerator};
pub use token::{AccessToken, BareToken, FloxhubToken, InvalidTokenError};

/// Test fixtures, re-exported from each type's own module.
#[cfg(any(test, feature = "tests"))]
pub mod test_helpers {
    pub use crate::auth::identity::test_helpers::*;
    pub use crate::auth::token::test_helpers::*;
}
