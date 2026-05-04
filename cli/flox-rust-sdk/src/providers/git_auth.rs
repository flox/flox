use flox_catalog::AuthContext;
use url::Url;

use super::git::GitCommandOptions;
use crate::models::floxmeta::FLOXHUB_TOKEN_ENV_VAR;

/// Extension trait for applying authentication to git command options.
pub trait GitCommandOptionsExt {
    /// Apply authentication based on the [`AuthContext`].
    ///
    /// Matches on the variant because git genuinely needs different behavior
    /// per auth type:
    /// - Auth0 (bearer): inline credential helper with the token
    /// - Kerberos: no-op (kerberized git uses the ccache directly)
    /// - No material: empty credential helper to prevent pinentry fallback
    fn authenticate(&mut self, auth_context: &AuthContext, git_url: &Url);
}

impl GitCommandOptionsExt for GitCommandOptions {
    fn authenticate(&mut self, auth_context: &AuthContext, git_url: &Url) {
        let token = match auth_context {
            AuthContext::Auth0(Some(token)) => {
                if token.is_expired() {
                    tracing::debug!("FloxHub token is expired, sending for identification");
                } else {
                    tracing::debug!("using valid FloxHub token");
                }
                token.secret()
            },
            AuthContext::Kerberos(_) => {
                tracing::debug!("Kerberos mode — git auth handled natively via ccache");
                return;
            },
            AuthContext::Auth0(None) => {
                tracing::debug!("no credential available for git auth");
                ""
            },
        };

        self.add_env_var(FLOXHUB_TOKEN_ENV_VAR, token);
        self.add_config_flag(
            &format!("credential.{git_url}.helper"),
            format!(
                r#"!f(){{ echo "username=oauth"; echo "password=${FLOXHUB_TOKEN_ENV_VAR}"; }}; f"#
            ),
        );
    }
}
