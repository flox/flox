use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::Environment;
use flox_rust_sdk::providers::services::ServiceError;
use tracing::instrument;

use super::{ConcreteEnvironment, EnvironmentSelect};

mod logs;
mod status;
mod stop;

/// Services Commands.
#[derive(Debug, Clone, Bpaf)]
pub enum ServicesCommands {
    /// Status of a service or services
    #[bpaf(command)]
    Status(#[bpaf(external(status::status))] status::Status),

    /// Stop a service or services
    #[bpaf(command)]
    Stop(#[bpaf(external(stop::stop))] stop::Stop),

    /// Print logs of services
    #[bpaf(command)]
    Logs(#[bpaf(external(logs::logs))] logs::Logs),
}

impl ServicesCommands {
    #[instrument(name = "services", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        if !flox.features.services {
            return Err(ServiceError::FeatureFlagDisabled.into());
        }

        match self {
            ServicesCommands::Status(args) => args.handle(flox).await?,
            ServicesCommands::Stop(args) => args.handle(flox).await?,
            ServicesCommands::Logs(args) => args.handle(flox).await?,
        }

        Ok(())
    }
}

/// Return an Environment for variants that support services.
pub fn supported_environment(
    flox: &Flox,
    environment: EnvironmentSelect,
) -> Result<Box<dyn Environment>> {
    let concrete_environment = environment.detect_concrete_environment(flox, "Services in")?;
    if let ConcreteEnvironment::Remote(_) = concrete_environment {
        return Err(ServiceError::RemoteEnvsNotSupported.into());
    }
    let dyn_environment = concrete_environment.into_dyn_environment();
    Ok(dyn_environment)
}
