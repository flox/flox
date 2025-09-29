use std::collections::VecDeque;
use std::env;
use std::fmt::{self, Display};
use std::str::FromStr;

use anyhow::Result;
use flox_rust_sdk::models::environment::{FLOX_ACTIVE_ENVIRONMENTS_VAR, UninitializedEnvironment};
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::utils::message;

/// A list of environments that are currently active
/// (i.e. have been activated with `flox activate`)
///
/// When inside a `flox activate` shell,
/// flox stores [UninitializedEnvironment] metadata to (re)open the activated environment
/// in `$FLOX_ACTIVE_ENVIRONMENTS`.
///
/// Environments which are activated while in a `flox activate` shell, are prepended
/// -> the most recently activated environment is the _first_ in the list of environments.
///
/// Internally this is implemented through a [VecDeque] which is serialized to JSON.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ActiveEnvironments(VecDeque<UninitializedEnvironment>);

impl FromStr for ActiveEnvironments {
    type Err = serde_json::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Ok(Self(VecDeque::new()));
        }
        serde_json::from_str(s).map(Self)
    }
}

impl ActiveEnvironments {
    /// Read the last active environment
    pub fn last_active(&self) -> Option<UninitializedEnvironment> {
        self.0.front().cloned()
    }

    /// Set the last active environment
    pub fn set_last_active(&mut self, env: UninitializedEnvironment) {
        self.0.push_front(env);
    }

    /// Check if the given environment is active
    pub fn is_active(&self, env: &UninitializedEnvironment) -> bool {
        self.0.contains(env)
    }

    /// Iterate over the active environments
    pub fn iter(&self) -> impl Iterator<Item = &UninitializedEnvironment> {
        self.0.iter()
    }
}

impl Display for ActiveEnvironments {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let result = if f.alternate() {
            serde_json::to_string_pretty(&self)
        } else {
            serde_json::to_string(&self)
        };
        let data = match result {
            Ok(data) => data,
            Err(e) => {
                debug!("Could not serialize active environments: {e}");
                return Err(fmt::Error);
            },
        };

        f.write_str(&data)
    }
}

impl IntoIterator for ActiveEnvironments {
    type IntoIter = std::collections::vec_deque::IntoIter<Self::Item>;
    type Item = UninitializedEnvironment;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

/// Determine the most recently activated environment [ActiveEnvironment].
pub(crate) fn last_activated_environment() -> Option<UninitializedEnvironment> {
    let env = activated_environments().last_active();
    debug!(
        env = env
            .as_ref()
            .map(|e| e.name().to_string())
            .unwrap_or("null".into()),
        "most recent activation"
    );
    env
}

/// Read [ActiveEnvironments] from the process environment [FLOX_ACTIVE_ENVIRONMENTS_VAR]
pub(crate) fn activated_environments() -> ActiveEnvironments {
    let flox_active_environments_var: String =
        env::var(FLOX_ACTIVE_ENVIRONMENTS_VAR).unwrap_or_default();
    debug!("read active environments variable");

    match ActiveEnvironments::from_str(&flox_active_environments_var) {
        Ok(active_environments) => active_environments,
        Err(e) => {
            message::error(format!(
                "Could not parse _FLOX_ACTIVE_ENVIRONMENTS -- using defaults: {e}"
            ));
            ActiveEnvironments::default()
        },
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use flox_rust_sdk::flox::EnvironmentName;
    use flox_rust_sdk::models::environment::{DotFlox, EnvironmentPointer, PathPointer};

    use super::*;

    fn path_env_fixture(name: &str) -> UninitializedEnvironment {
        UninitializedEnvironment::DotFlox(DotFlox {
            path: PathBuf::new(),
            pointer: EnvironmentPointer::Path(PathPointer::new(
                EnvironmentName::from_str(name).unwrap(),
            )),
        })
    }

    /// is_active() behaves as expected when using set_last_active()
    #[test]
    fn test_is_active() {
        let env1 = path_env_fixture("env1");
        let env2 = path_env_fixture("env2");

        let mut active = ActiveEnvironments::default();
        active.set_last_active(env1.clone());

        assert!(active.is_active(&env1));
        assert!(!active.is_active(&env2));
    }

    /// Simulate setting an active environment in one flox invocation and then
    /// checking if it's active in a second.
    #[test]
    fn test_is_active_round_trip_from_env() {
        let uninitialized = path_env_fixture("test");
        let mut first_active = temp_env::with_var(
            FLOX_ACTIVE_ENVIRONMENTS_VAR,
            None::<&str>,
            activated_environments,
        );

        first_active.set_last_active(uninitialized.clone());

        let second_active = temp_env::with_var(
            FLOX_ACTIVE_ENVIRONMENTS_VAR,
            Some(first_active.to_string()),
            activated_environments,
        );

        assert!(second_active.is_active(&uninitialized));
    }

    #[test]
    fn test_last_activated() {
        let env1 = path_env_fixture("env1");
        let env2 = path_env_fixture("env2");

        let mut active = ActiveEnvironments::default();
        active.set_last_active(env1);
        active.set_last_active(env2.clone());

        let last_active = temp_env::with_var(
            FLOX_ACTIVE_ENVIRONMENTS_VAR,
            Some(active.to_string()),
            last_activated_environment,
        );
        assert_eq!(last_active.unwrap(), env2)
    }
}
