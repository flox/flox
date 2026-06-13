use flox_catalog::AuthContext;
use url::Url;

use crate::models::floxmeta::FLOXHUB_TOKEN_ENV_VAR;
use crate::providers::git::GitCommandOptions;

/// Extension trait for applying authentication to git command options.
pub trait GitCommandOptionsExt {
    /// Apply authentication based on the [`AuthContext`].
    ///
    /// Matches on the variant because git genuinely needs different behavior
    /// per auth type:
    /// - Auth0 (bearer): inline credential helper with the token
    /// - Kerberos: set `http.emptyAuth=true` so libcurl performs the SPNEGO
    ///   Negotiate handshake against the remote; credentials themselves come
    ///   from the ccache
    /// - No material: empty credential helper to prevent pinentry fallback
    fn authenticate(&mut self, auth_context: &AuthContext, git_url: &Url);
}

impl GitCommandOptionsExt for GitCommandOptions {
    fn authenticate(&mut self, auth_context: &AuthContext, git_url: &Url) {
        match auth_context {
            AuthContext::Auth0(maybe_token) => {
                let token = match maybe_token {
                    Some(token) => {
                        if token.is_expired() {
                            tracing::debug!("FloxHub token is expired, sending for identification");
                        } else {
                            tracing::debug!("using valid FloxHub token");
                        }
                        token.secret()
                    },
                    None => {
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
            },
            AuthContext::Kerberos(material) => {
                match material {
                    Some(_) => {
                        tracing::debug!("Kerberos mode — git auth handled natively via ccache");
                    },
                    None => {
                        tracing::warn!(
                            "Kerberos mode but no ticket available — git operations will likely fail; run 'kinit'"
                        );
                    },
                }
                self.add_config_flag("http.emptyAuth", "true");
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flox::test_helpers::create_test_token;

    fn test_url() -> Url {
        Url::parse("https://git.floxhub.com").unwrap()
    }

    #[test]
    fn auth0_with_token_sets_credential_helper() {
        let token = create_test_token("testuser");
        let auth = AuthContext::Auth0(Some(token.clone()));
        let mut options = GitCommandOptions::default();

        let mut expected = GitCommandOptions::default();
        expected.add_env_var(FLOXHUB_TOKEN_ENV_VAR, token.secret());
        expected.add_config_flag(
            &format!("credential.{}.helper", test_url()),
            format!(
                r#"!f(){{ echo "username=oauth"; echo "password=${FLOXHUB_TOKEN_ENV_VAR}"; }}; f"#
            ),
        );

        options.authenticate(&auth, &test_url());
        assert_eq!(options, expected);
    }

    #[test]
    fn auth0_without_token_sets_empty_credential_helper() {
        let auth = AuthContext::Auth0(None);
        let mut options = GitCommandOptions::default();

        let mut expected = GitCommandOptions::default();
        expected.add_env_var(FLOXHUB_TOKEN_ENV_VAR, "");
        expected.add_config_flag(
            &format!("credential.{}.helper", test_url()),
            format!(
                r#"!f(){{ echo "username=oauth"; echo "password=${FLOXHUB_TOKEN_ENV_VAR}"; }}; f"#
            ),
        );

        options.authenticate(&auth, &test_url());
        assert_eq!(options, expected);
    }

    #[test]
    fn kerberos_with_material_sets_empty_auth() {
        let auth = AuthContext::Kerberos(Some(flox_catalog::KerberosMaterial {
            principal: "user@REALM".to_string(),
            generate_token: std::sync::Arc::new(|_| Ok("token".to_string())),
        }));
        let mut options = GitCommandOptions::default();

        let mut expected = GitCommandOptions::default();
        expected.add_config_flag("http.emptyAuth", "true");

        options.authenticate(&auth, &test_url());
        assert_eq!(options, expected);
    }

    #[test]
    fn kerberos_without_material_sets_empty_auth() {
        let auth = AuthContext::Kerberos(None);
        let mut options = GitCommandOptions::default();

        let mut expected = GitCommandOptions::default();
        expected.add_config_flag("http.emptyAuth", "true");

        options.authenticate(&auth, &test_url());
        assert_eq!(options, expected);
    }
}
