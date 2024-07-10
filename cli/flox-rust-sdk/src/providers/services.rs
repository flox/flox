use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};

use once_cell::sync::Lazy;
#[cfg(test)]
use proptest::prelude::*;
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

use crate::flox::Flox;
use crate::models::lockfile::LockedManifestCatalog;
use crate::models::manifest::ManifestServices;
use crate::utils::traceable_path;

pub const SERVICES_ENV_VAR: &str = "FLOX_FEATURES_SERVICES";
pub const SERVICES_TEMP_CONFIG_PATH_VAR: &str = "_FLOX_SERVICES_CONFIG_PATH";
pub const SERVICE_CONFIG_FILENAME: &str = "service-config.yaml";
pub static PROCESS_COMPOSE_BIN: Lazy<String> = Lazy::new(|| {
    env::var("PROCESS_COMPOSE_BIN").unwrap_or(env!("PROCESS_COMPOSE_BIN").to_string())
});

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("failed to generate service config")]
    GenerateConfig(#[source] serde_yaml::Error),
    #[error("failed to write service config")]
    WriteConfig(#[source] std::io::Error),
    #[error("services are disabled by the feature flag")]
    FeatureFlagDisabled,
    #[error("services have not been started in this activation")]
    NotInActivation,
}

/// The deserialized representation of a `process-compose` config file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub struct ProcessComposeConfig {
    #[cfg_attr(
        test,
        proptest(
            strategy = "proptest::collection::btree_map(any::<String>(), any::<ProcessConfig>(), 0..=3)"
        )
    )]
    pub processes: BTreeMap<String, ProcessConfig>,
}

/// The config for a single service
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub struct ProcessConfig {
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, proptest(strategy = "arbitrary_process_config_environment()"))]
    pub vars: Option<BTreeMap<String, String>>,
}

#[cfg(test)]
fn arbitrary_process_config_environment(
) -> impl proptest::strategy::Strategy<Value = Option<BTreeMap<String, String>>> {
    proptest::option::of(proptest::collection::btree_map(
        any::<String>(),
        any::<String>(),
        0..=3,
    ))
}

impl From<ManifestServices> for ProcessComposeConfig {
    fn from(services: ManifestServices) -> Self {
        let processes = services
            .0
            .into_iter()
            .map(|(name, service)| {
                let command = service.command;
                let environment = service.vars.map(|vars| vars.0);
                (name, ProcessConfig {
                    command,
                    vars: environment,
                })
            })
            .collect();
        ProcessComposeConfig { processes }
    }
}

// generate the config string
// write it out to the path
pub fn write_process_compose_config(
    config: &ProcessComposeConfig,
    path: impl AsRef<Path>,
) -> Result<(), ServiceError> {
    let contents = serde_yaml::to_string(&config).map_err(ServiceError::GenerateConfig)?;
    std::fs::write(path, contents).map_err(ServiceError::WriteConfig)?;
    Ok(())
}

/// Determines the location to write the service config file
pub fn service_config_write_location(temp_dir: impl AsRef<Path>) -> Result<PathBuf, ServiceError> {
    if let Ok(path) = env::var(SERVICES_TEMP_CONFIG_PATH_VAR) {
        return Ok(PathBuf::from(path));
    }

    let file = NamedTempFile::new_in(temp_dir).map_err(ServiceError::WriteConfig)?;
    let (_, path) = file
        .keep()
        .map_err(|e| ServiceError::WriteConfig(e.error))?;

    Ok(path)
}

pub fn maybe_make_service_config_file(
    flox: &Flox,
    lockfile: &LockedManifestCatalog,
) -> Result<Option<PathBuf>, ServiceError> {
    let service_config_path = if flox.features.services {
        let config_path = service_config_write_location(&flox.temp_dir)?;
        write_process_compose_config(&lockfile.manifest.services.clone().into(), &config_path)?;
        tracing::debug!(path = traceable_path(&config_path), "wrote service config");
        Some(config_path)
    } else {
        None
    };
    Ok(service_config_path)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use tempfile::TempDir;

    use super::*;

    proptest! {
        #[test]
        fn test_process_compose_config_roundtrip(config: ProcessComposeConfig) {
            let temp_dir = TempDir::new().unwrap();
            let path = service_config_write_location(&temp_dir).unwrap();
            write_process_compose_config(&config, &path).unwrap();
            let contents = std::fs::read_to_string(&path).unwrap();
            let deserialized: ProcessComposeConfig = serde_yaml::from_str(&contents).unwrap();
            prop_assert_eq!(config, deserialized);
        }
    }
}
