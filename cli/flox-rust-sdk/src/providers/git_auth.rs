use flox_catalog::AuthContext;
use url::Url;

use super::git::GitCommandOptions;
use crate::models::floxmeta::FLOXHUB_TOKEN_ENV_VAR;

/// Apply authentication to git command options based on a [Credential].
///
/// Matches on the credential variant because git genuinely needs different
/// behavior per auth type:
/// - Bearer: inline credential helper with the token
/// - Kerberos: no-op (kerberized git uses the ccache directly)
/// - None: empty credential helper to prevent pinentry fallback
pub fn apply_git_auth(credential: &AuthContext, git_url: &Url, options: &mut GitCommandOptions) {
    let token = match credential {
        AuthContext::Auth0(Some(token)) => {
            if token.is_expired() {
                tracing::debug!("FloxHub token is expired, sending for identification");
            } else {
                tracing::debug!("using valid FloxHub token");
            }
            token.secret()
        },
        AuthContext::Kerberos(_) => {
            // Kerberized git handles SPNEGO auth natively via ccache — no-op
            return;
        },
        AuthContext::Auth0(None) => {
            tracing::debug!("no credential available for git auth");
            ""
        },
    };

    options.add_env_var(FLOXHUB_TOKEN_ENV_VAR, token);
    options.add_config_flag(
        &format!("credential.{git_url}.helper"),
        format!(r#"!f(){{ echo "username=oauth"; echo "password=${FLOXHUB_TOKEN_ENV_VAR}"; }}; f"#),
    );
}
