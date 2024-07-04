use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};

use once_cell::sync::Lazy;
#[cfg(test)]
use proptest::prelude::*;
use serde::{Deserialize, Serialize};

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
// write it out to either the env var location or a temp file
pub fn write_process_compose_config(
    config: &ProcessComposeConfig,
    path: impl AsRef<Path>,
) -> Result<(), ServiceError> {
    let contents = serde_yaml::to_string(&config).map_err(ServiceError::GenerateConfig)?;
    std::fs::write(path, contents).map_err(ServiceError::WriteConfig)?;
    Ok(())
}

/// A container for the path to the `process-compose` config file
///
/// This is necessary in the situation where the config file is written to a temporary location
/// and we need to keep the destructor from deleting the temp directory before we're done with it.
#[derive(Debug)]
pub struct ConfigPath {
    pub path: PathBuf,
    _tempdir: Option<tempfile::TempDir>,
}

/// Determines the location to write the `process-compose` config file
pub fn process_compose_config_write_location() -> Result<ConfigPath, ServiceError> {
    if let Ok(path) = env::var(SERVICES_TEMP_CONFIG_PATH_VAR) {
        return Ok(ConfigPath {
            path: PathBuf::from(path),
            _tempdir: None,
        });
    }
    let temp_dir = tempfile::tempdir().map_err(ServiceError::WriteConfig)?;
    let path = temp_dir.path().join("process-compose.yaml");
    Ok(ConfigPath {
        path,
        _tempdir: Some(temp_dir),
    })
}

pub fn maybe_make_service_config_file(
    flox: &Flox,
    lockfile: &LockedManifestCatalog,
) -> Result<Option<ConfigPath>, ServiceError> {
    let service_config_path = if flox.features.services {
        let config_path = process_compose_config_write_location()?;
        write_process_compose_config(
            &lockfile.manifest.services.clone().into(),
            &config_path.path,
        )?;
        tracing::debug!(
            path = traceable_path(&config_path.path),
            "wrote service config"
        );
        Some(config_path)
    } else {
        None
    };
    Ok(service_config_path)
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use indoc::indoc;
    use proptest::prelude::*;

    use super::*;
    use crate::flox::test_helpers::flox_instance_with_optional_floxhub_and_client;
    use crate::flox::{EnvironmentName, EnvironmentOwner};
    use crate::models::environment::path_environment::{InitCustomization, PathEnvironment};
    use crate::models::environment::{Environment, PathPointer};

    proptest! {
        #[test]
        fn test_process_compose_config_roundtrip(config: ProcessComposeConfig) {
            let path = process_compose_config_write_location().unwrap();
            write_process_compose_config(&config, &path.path).unwrap();
            let contents = std::fs::read_to_string(&path.path).unwrap();
            let deserialized: ProcessComposeConfig = serde_yaml::from_str(&contents).unwrap();
            prop_assert_eq!(config, deserialized);
        }
    }

    #[test]
    fn built_environments_generate_service_config() {
        let (mut flox, workspace_dir) = flox_instance_with_optional_floxhub_and_client(
            Some(&EnvironmentOwner::from_str("owner").unwrap()),
            true,
        );
        flox.features.services = true;
        let temp_dir = tempfile::tempdir().unwrap();
        let pointer = PathPointer::new(EnvironmentName::from_str("services_env").unwrap());
        let customization = InitCustomization::default();
        let mut env = PathEnvironment::init(
            pointer,
            workspace_dir.path(),
            temp_dir.path(),
            &flox.system,
            &customization,
            &flox,
        )
        .unwrap();

        // Write out a manifest with a services section
        let contents = indoc! {r#"
        version = 1

        [services.foo]
        command = "start foo"
        "#};
        std::fs::write(
            workspace_dir.path().join(".flox/env/manifest.toml"),
            contents,
        )
        .unwrap();

        // Build the environment and verify that the config file exists
        temp_env::with_var_unset(SERVICES_TEMP_CONFIG_PATH_VAR, || {
            env.build(&flox).unwrap();
        });
        let config_path = format!(
            "{}/.flox/run/{}.services_env/{SERVICE_CONFIG_FILENAME}",
            &workspace_dir.path().display(),
            &flox.system
        );
        assert!(PathBuf::from(config_path).exists());
    }
}
