use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use floxhub_client::CatalogClientTrait;
use indoc::formatdoc;
use tracing::instrument;

use crate::commands::archive::{catalog_failure, split_package};
use crate::commands::ensure_auth;
use crate::utils::message;

#[derive(Bpaf, Clone)]
pub struct Unarchive {
    /// Package to unarchive.
    #[bpaf(positional::<String>("catalog>/<package"), parse(split_package))]
    package: (String, String),
}

impl Unarchive {
    #[instrument(name = "unarchive", skip_all)]
    pub async fn handle(self, mut flox: Flox) -> Result<()> {
        let (catalog, name) = self.package;
        ensure_auth(&mut flox).await?;
        unarchive_package(&flox.floxhub_client, &catalog, &name).await
    }
}

async fn unarchive_package(
    client: &impl CatalogClientTrait,
    catalog: &str,
    name: &str,
) -> Result<()> {
    let package = format!("{catalog}/{name}");
    let changed = client
        .unarchive_package(catalog, name)
        .await
        .map_err(|err| catalog_failure("unarchive", &package, err))?;

    if changed {
        message::updated(formatdoc! {"
            Unarchived '{package}'.
            It appears in search again and can be installed by name."
        });
    } else {
        message::plain(format!("'{package}' is not archived."));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use floxhub_client::FloxhubClient;
    use floxhub_client::client::test_helpers::client_config;
    use httpmock::MockServer;
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn unarchive_reports_server_responses() {
        let (catalog, name) = ("myorg", "mypkg");
        let package = format!("{catalog}/{name}");
        let detail = format!("You do not have write access to {package}.");
        for (status, body, expected) in [
            (
                403,
                json!({ "detail": detail }),
                Err(format!("Failed to unarchive '{package}'.\n{detail}")),
            ),
            (200, json!({ "changed": false }), Ok(())),
        ] {
            let server = MockServer::start_async().await;
            let mock = server.mock(|when, then| {
                when.method("DELETE").path(format!(
                    "/api/v1/catalog/catalogs/{catalog}/packages/{name}/archive"
                ));
                then.status(status).json_body(body.clone());
            });
            let client = FloxhubClient::new(client_config(server.base_url().as_str())).unwrap();
            let result = unarchive_package(&client, catalog, name)
                .await
                .map_err(|err| err.to_string());

            mock.assert_calls(1);
            assert_eq!(result, expected, "{status}");
        }
    }
}
