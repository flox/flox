use floxhub_client::auth::Credential;
use floxhub_client::{AuthContext, CredentialKind};
use url::Url;

use crate::models::floxmeta::FLOXHUB_TOKEN_ENV_VAR;
use crate::providers::git::GitCommandOptions;

/// Extension trait for applying authentication to git command options.
pub trait GitCommandOptionsExt {
    /// Apply authentication based on the [`AuthContext`].
    ///
    /// Bearer credentials use an inline helper. Kerberos delegates to git's
    /// native ccache support via `http.emptyAuth`.
    fn authenticate(&mut self, auth_context: &AuthContext, git_url: &Url);
}

impl GitCommandOptionsExt for GitCommandOptions {
    fn authenticate(&mut self, auth_context: &AuthContext, git_url: &Url) {
        if let Credential::Kerberos(material) = auth_context.credential() {
            self.add_config_flag("http.emptyAuth", "true");
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
            return;
        }
        let token = auth_context.token_secret().unwrap_or("");
        // The credential is loaded, so diagnostics use its actual facts.
        // For these JWT kinds, a present token is locally unauthenticated only
        // when it has expired.
        match auth_context.kind() {
            CredentialKind::Auth0 if auth_context.is_unauthenticated() => {
                tracing::debug!("FloxHub token is expired, sending for identification");
            },
            CredentialKind::Auth0 => {
                tracing::debug!("using valid FloxHub token");
            },
            CredentialKind::Bare if auth_context.is_unauthenticated() => {
                tracing::debug!("bare FloxHub token is expired, sending for identification");
            },
            CredentialKind::Bare => {
                tracing::debug!("using bare FloxHub token");
            },
            CredentialKind::PersonalAccessToken
            | CredentialKind::ServiceAccountToken
            | CredentialKind::OpaqueToken => {
                tracing::debug!(kind = ?auth_context.kind(), "using FloxHub access token");
            },
            CredentialKind::NotLoggedIn => {
                tracing::debug!("no credential available for git auth");
            },
            _ => {},
        }
        self.add_env_var(FLOXHUB_TOKEN_ENV_VAR, token);
        self.add_config_flag(
            &format!("credential.{git_url}.helper"),
            format!(
                r#"!f(){{ echo "username=oauth"; echo "password=${FLOXHUB_TOKEN_ENV_VAR}"; }}; f"#
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use floxhub_client::KerberosMaterial;
    use floxhub_client::auth::storage::{AuthContextStorageExt, CachedFacts};

    use super::*;
    use crate::flox::test_helpers::create_test_token;

    fn test_url() -> Url {
        Url::parse("https://git.floxhub.com").unwrap()
    }

    fn capture_logs(f: impl FnOnce()) -> String {
        let log = tempfile::NamedTempFile::new().unwrap();
        let writer = log.reopen().unwrap();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_target(false)
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(move || writer.try_clone().unwrap())
            .finish();
        tracing::subscriber::with_default(subscriber, f);
        std::fs::read_to_string(log.path()).unwrap()
    }

    #[test]
    fn jwt_git_diagnostics_use_loaded_expiry_and_still_send_expired_tokens() {
        for (kind, expiry, expected_log) in [
            (
                CredentialKind::Auth0,
                Some(1),
                "DEBUG FloxHub token is expired, sending for identification\n",
            ),
            (
                CredentialKind::Auth0,
                Some(9999999999_i64),
                "DEBUG using valid FloxHub token\n",
            ),
            (
                CredentialKind::Bare,
                Some(1),
                "DEBUG bare FloxHub token is expired, sending for identification\n",
            ),
            (
                CredentialKind::Bare,
                Some(9999999999_i64),
                "DEBUG using bare FloxHub token\n",
            ),
            (
                CredentialKind::Bare,
                None,
                "DEBUG using bare FloxHub token\n",
            ),
        ] {
            let mut claims = serde_json::json!({});
            if let Some(expiry) = expiry {
                claims["exp"] = serde_json::json!(expiry);
            }
            if kind == CredentialKind::Auth0 {
                claims["https://flox.dev/handle"] = serde_json::json!("testuser");
            }
            let secret = jsonwebtoken::encode(
                &jsonwebtoken::Header::default(),
                &claims,
                &jsonwebtoken::EncodingKey::from_secret(b"secret"),
            )
            .unwrap();
            let resolved = AuthContext::new_from_token(Some(&secret));
            let mut recorded = resolved.cached_facts();
            // Both the kind and expiry can change before a deferred read.
            recorded.kind = if kind == CredentialKind::Auth0 {
                CredentialKind::Bare
            } else {
                CredentialKind::Auth0
            };
            recorded.expires_at =
                chrono::DateTime::from_timestamp(if expiry == Some(1) { 9999999999 } else { 1 }, 0);
            let calls = Arc::new(AtomicUsize::new(0));
            let counter = Arc::clone(&calls);
            let credential = resolved.clone();
            let deferred = AuthContext::deferred(recorded, move || {
                counter.fetch_add(1, Ordering::SeqCst);
                credential.clone()
            });
            assert_eq!(calls.load(Ordering::SeqCst), 0);

            for context in [resolved, deferred.clone(), deferred] {
                let mut options = GitCommandOptions::default();
                let logs = capture_logs(|| options.authenticate(&context, &test_url()));
                assert_eq!(logs, expected_log);

                let mut expected = GitCommandOptions::default();
                expected.add_env_var(FLOXHUB_TOKEN_ENV_VAR, &secret);
                expected.add_config_flag(
                    &format!("credential.{}.helper", test_url()),
                    format!(
                        r#"!f(){{ echo "username=oauth"; echo "password=${FLOXHUB_TOKEN_ENV_VAR}"; }}; f"#
                    ),
                );
                assert_eq!(options, expected);
            }
            assert_eq!(calls.load(Ordering::SeqCst), 1);
        }
    }

    #[test]
    fn kerberos_git_diagnostics_check_deferred_credentials_once_across_clones() {
        for principal in [Some("user@REALM"), None] {
            let calls = Arc::new(AtomicUsize::new(0));
            let counter = Arc::clone(&calls);
            let auth = AuthContext::deferred(
                CachedFacts {
                    kind: CredentialKind::Kerberos,
                    // Advisory facts must not decide whether a ticket exists.
                    handle: principal.is_none().then(|| "stale@REALM".to_string()),
                    ..Default::default()
                },
                move || {
                    counter.fetch_add(1, Ordering::SeqCst);
                    AuthContext::from_kerberos(principal.map(|principal| KerberosMaterial {
                        principal: principal.to_string(),
                        generate_token: Arc::new(|_| {
                            panic!("git must use native ccache authentication")
                        }),
                    }))
                },
            );
            assert_eq!(calls.load(Ordering::SeqCst), 0);

            for context in [&auth, &auth.clone()] {
                let mut options = GitCommandOptions::default();
                let logs = capture_logs(|| options.authenticate(context, &test_url()));
                let expected_log = match principal {
                    Some(_) => "DEBUG Kerberos mode — git auth handled natively via ccache\n",
                    None => {
                        " WARN Kerberos mode but no ticket available — git operations will likely fail; run 'kinit'\n"
                    },
                };
                assert_eq!(logs, expected_log);
                assert_eq!(calls.load(Ordering::SeqCst), 1);

                let mut expected = GitCommandOptions::default();
                expected.add_config_flag("http.emptyAuth", "true");
                assert_eq!(options, expected);
            }
        }
    }

    #[test]
    fn auth0_with_token_sets_credential_helper() {
        let token = create_test_token("testuser");
        let auth = AuthContext::from_auth0_token(Some(token.clone()));
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
        let auth = AuthContext::from_auth0_token(None);
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
    fn pat_sets_credential_helper_with_secret() {
        let token = floxhub_client::AccessToken::new("flox_pat_secret".to_string());
        let auth = AuthContext::from_access_token(token.clone());
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
    fn kerberos_with_material_sets_empty_auth() {
        let auth = AuthContext::from_kerberos(Some(floxhub_client::KerberosMaterial {
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
        let auth = AuthContext::from_kerberos(None);
        let mut options = GitCommandOptions::default();

        let mut expected = GitCommandOptions::default();
        expected.add_config_flag("http.emptyAuth", "true");

        options.authenticate(&auth, &test_url());
        assert_eq!(options, expected);
    }
}
