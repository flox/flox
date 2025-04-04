//! This module contains the generated OpenAPI client for the Catalog API.
//!
//! The client is generated from the OpenAPI spec in `openapi.json` using the `progenitor` crate.
//! The spec is managed by the Catalog API team and is updated when the upstream API changes.

mod client;
mod error;
pub use client::*;
#[cfg(test)]
mod mock;

pub mod types {
    pub use crate::client::types::*;
    pub use crate::error::MessageType;
}

#[cfg(test)]
mod tests {

    use crate::{mock::MockServerExt, types::Variant2};

    use super::*;
    use httpmock::prelude::*;

    #[tokio::test]
    async fn deserializes_user() {
        let server = MockServer::start();

        let get_user_mock = server.get_user_by_id(|when, then| {
            when.id(0);
            then.ok(&types::User::Variant2(Variant2 {
                variant: serde_json::Value::String("variant2".to_string()),
            }));
        });

        let client = Client::new(&server.base_url());

        let user = client.get_user_by_id(0).await.unwrap().into_inner();

        get_user_mock.assert();
        assert!(matches!(user, types::User::Variant2(_)), "Expected Variant2, got {:?}", user);
    }
}
