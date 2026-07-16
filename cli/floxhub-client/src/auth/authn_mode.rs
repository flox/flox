//! [`AuthnMode`] — the authentication method configured for FloxHub.

use serde::{Deserialize, Serialize};

/// Available authentication methods
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthnMode {
    /// Auth0 authentication
    #[default]
    Auth0,
    /// Kerberos authentication
    Kerberos,
}
