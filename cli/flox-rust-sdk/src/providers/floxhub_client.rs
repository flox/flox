use reqwest::Client;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

#[derive(Serialize)]
struct RenameRequest {
    name: String,
}

#[derive(Deserialize)]
pub struct RenameResponse {
    pub owner: String,
    pub name: String,
}

pub struct FloxhubClient {
    client: Client,
    base_url: Url,
    token: String,
}

impl FloxhubClient {
    pub fn new(base_url: &Url, token: &str) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.clone(),
            token: token.to_string(),
        }
    }

    pub async fn rename_environment(
        &self,
        owner: &str,
        current_name: &str,
        new_name: &str,
    ) -> Result<RenameResponse, FloxhubClientError> {
        let url = self
            .base_url
            .join(&format!("environment/{}/{}/rename", owner, current_name))
            .map_err(FloxhubClientError::InvalidUrl)?;

        let response = self
            .client
            .post(url)
            .header("X-Flox-Github-Token", &self.token)
            .json(&RenameRequest {
                name: new_name.to_string(),
            })
            .send()
            .await
            .map_err(FloxhubClientError::Request)?;

        match response.status() {
            status if status.is_success() => Ok(response
                .json()
                .await
                .map_err(FloxhubClientError::Response)?),
            status if status == 403 => Err(FloxhubClientError::AccessDenied),
            status if status == 409 => Err(FloxhubClientError::Conflict),
            status if status == 400 => Err(FloxhubClientError::InvalidName),
            _ => Err(FloxhubClientError::Other(response.status())),
        }
    }
}

#[derive(Error, Debug)]
pub enum FloxhubClientError {
    #[error("Access denied: not permitted to rename environment")]
    AccessDenied,
    #[error("An environment with that name already exists")]
    Conflict,
    #[error("Invalid environment name")]
    InvalidName,
    #[error("Invalid URL")]
    InvalidUrl(#[source] url::ParseError),
    #[error("Request failed")]
    Request(#[source] reqwest::Error),
    #[error("Failed to parse response")]
    Response(#[source] reqwest::Error),
    #[error("HTTP {0}")]
    Other(reqwest::StatusCode),
}
