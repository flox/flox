use anyhow::{Result, anyhow, bail};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use floxhub_client::{ApiError, CatalogClientTrait, FloxhubClientError};
use indoc::formatdoc;
use tracing::instrument;

use crate::commands::ensure_auth;
use crate::utils::dialog::{Confirm, Dialog};
use crate::utils::errors::display_chain;
use crate::utils::message;

#[derive(Bpaf, Clone)]
pub struct Archive {
    /// Reason for archiving the package, shown to its users.
    #[bpaf(long, argument("text"))]
    reason: Option<String>,

    /// Package that replaces this one, such as 'myorg/hello' or 'hello'.
    #[bpaf(long, argument("package"))]
    replacement: Option<String>,

    /// Archive a package without confirmation.
    #[bpaf(short, long)]
    force: bool,

    /// Package to archive.
    #[bpaf(positional::<String>("catalog>/<package"), parse(split_package))]
    package: (String, String),
}

impl Archive {
    #[instrument(name = "archive", skip_all)]
    pub async fn handle(self, mut flox: Flox) -> Result<()> {
        let (catalog, name) = self.package;
        if !self.force {
            confirm_archive(&format!("{catalog}/{name}"), Dialog::can_prompt()).await?;
        }
        ensure_auth(&mut flox).await?;
        archive_package(
            &flox.floxhub_client,
            &catalog,
            &name,
            self.reason,
            self.replacement,
        )
        .await
    }
}

/// Splits `<catalog>/<package>`, leaving the client to validate each name.
/// bpaf prefixes the error with the input, so the message does not repeat it.
pub(super) fn split_package(package: String) -> Result<(String, String)> {
    let Some((catalog, name)) = package.split_once('/') else {
        bail!("Name the package with its catalog (e.g. 'myorg/hello').");
    };
    Ok((catalog.to_string(), name.to_string()))
}

async fn archive_package(
    client: &impl CatalogClientTrait,
    catalog: &str,
    name: &str,
    reason: Option<String>,
    replacement: Option<String>,
) -> Result<()> {
    let package = format!("{catalog}/{name}");
    let changed = client
        .archive_package(catalog, name, reason, replacement)
        .await
        .map_err(|err| catalog_failure("archive", &package, err))?;

    if changed {
        message::updated(format!("Archived '{package}'."));
    } else {
        message::plain(format!("'{package}' is already archived."));
    }
    Ok(())
}

/// Archiving changes what every user of a package can install, so unlike
/// `flox delete` it never proceeds unconfirmed just because it cannot prompt.
async fn confirm_archive(package: &str, can_prompt: bool) -> Result<()> {
    if !can_prompt {
        bail!(formatdoc! {"
            Refusing to archive without confirmation.
            'flox archive' can only prompt in an interactive terminal.
            Pass '--force' to archive without confirmation."
        });
    }

    message::plain(formatdoc! {"
        You are about to archive '{package}'.
        It will be hidden from search and cannot be installed by name.
        Environments that already pin it keep working.
        You can unarchive it at any time."
    });
    let confirmed = Dialog {
        message: "Are you sure?",
        help_message: Some("Use '-f' to archive without confirmation"),
        typed: Confirm {
            default: Some(false),
        },
    }
    .prompt()
    .await
    .or_else(|err| match err {
        inquire::InquireError::OperationCanceled | inquire::InquireError::OperationInterrupted => {
            Ok(false)
        },
        err => Err(err),
    })?;
    if !confirmed {
        bail!("Archive canceled.");
    }
    Ok(())
}

/// The server's detail already distinguishes causes that share a status,
/// such as a 403 for a non-writer and for a principal that is not a user, and
/// says how to resolve a 409, so it is shown rather than replaced with a
/// message keyed on the status.
pub(super) fn catalog_failure(
    action: &str,
    package: &str,
    err: FloxhubClientError,
) -> anyhow::Error {
    let reason = match err {
        FloxhubClientError::APIError(ApiError::ErrorResponse(response)) => {
            response.into_inner().detail
        },
        err => display_chain(&err),
    };
    anyhow!("Failed to {action} '{package}'.\n{reason}")
}

#[cfg(test)]
mod tests {
    use floxhub_client::FloxhubClient;
    use floxhub_client::client::test_helpers::client_config;
    use httpmock::MockServer;
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn confirm_archive_refuses_without_terminal() {
        let result = confirm_archive("myorg/mypkg", false)
            .await
            .map_err(|err| err.to_string());

        assert_eq!(
            result,
            Err(formatdoc! {"
                Refusing to archive without confirmation.
                'flox archive' can only prompt in an interactive terminal.
                Pass '--force' to archive without confirmation."
            })
        );
    }

    #[tokio::test]
    async fn archive_reports_server_responses() {
        let (catalog, name) = ("myorg", "mypkg");
        let package = format!("{catalog}/{name}");
        let detail =
            format!("{package} is already archived with replacement unset and message 'old'.");
        let json = "application/json";
        for (status, content_type, body, expected) in [
            (
                409,
                json,
                json!({ "detail": detail }).to_string(),
                Err(format!("Failed to archive '{package}'.\n{detail}")),
            ),
            (
                502,
                "text/html",
                "<html>Bad Gateway</html>".to_string(),
                Err(format!("Failed to archive '{package}'.\n502 Bad Gateway")),
            ),
        ] {
            let server = MockServer::start_async().await;
            let mock = server.mock(|when, then| {
                when.method("PUT").path(format!(
                    "/api/v1/catalog/catalogs/{catalog}/packages/{name}/archive"
                ));
                then.status(status)
                    .header("content-type", content_type)
                    .body(&body);
            });
            let client = FloxhubClient::new(client_config(server.base_url().as_str())).unwrap();
            let result = archive_package(&client, catalog, name, None, None)
                .await
                .map_err(|err| err.to_string());

            mock.assert_calls(1);
            assert_eq!(result, expected, "{status}");
        }
    }
}
