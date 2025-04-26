use std::sync::Arc;

use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use rmcp::ServiceExt as _;
use server::Server;

mod server;

#[derive(Bpaf, Clone)]
pub struct Mcp {}

impl Mcp {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        tracing::info!("Starting MCP server");

        // Create an instance of our counter router
        let service = Server::new(Arc::new(flox))
            .serve(rmcp::transport::stdio())
            .await
            .inspect_err(|e| {
                tracing::error!("serving error: {:?}", e);
            })?;

        service.waiting().await?;
        Ok(())
    }
}
