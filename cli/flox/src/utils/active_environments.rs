use std::collections::VecDeque;
use std::env;
use std::fmt::{self, Display};
use std::str::FromStr;

use anyhow::Result;
use flox_core::activate::mode::ActivateMode;
use flox_core::activate::vars::FLOX_ACTIVE_ENVIRONMENTS_VAR;
use flox_rust_sdk::models::environment::UninitializedEnvironment;
use flox_rust_sdk::models::environment::generations::GenerationId;
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::utils::message;

/// An environment that has been activated with `flox activate`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActiveEnvironment {
    /// Metadata to (re)open the activated environment.
    pub environment: UninitializedEnvironment,

    /// Specific generation that was activated, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation: Option<GenerationId>,

    /// --mode the environment was activated with
    pub mode: ActivateMode,
}

/// A list of environments that are currently active
/// (i.e. have been activated with `flox activate`)
///
/// Environments which are activated while in a `flox activate` shell, are prepended
/// -> the most recently activated environment is the _first_ in the list of environments.
///
/// Internally this is implemented through a [VecDeque] which is serialized to JSON and stored
/// in `$FLOX_ACTIVE_ENVIRONMENTS` by `flox activate`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ActiveEnvironments(VecDeque<ActiveEnvironment>);

impl FromStr for ActiveEnvironments {
    type Err = serde_json::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Ok(Self(VecDeque::new()));
        }

        serde_json::from_str(s).map(Self).or_else(|_| {
            // Fallback from the old flat UninitializedEnvironment format.
            serde_json::from_str::<VecDeque<UninitializedEnvironment>>(s).map(|envs| {
                Self(
                    envs.into_iter()
                        .map(|environment| ActiveEnvironment {
                            environment,
                            generation: None,
                            // Dev mode was the default for restarting services
                            // before we recorded the mode
                            mode: ActivateMode::Dev,
                        })
                        .collect(),
                )
            })
        })
    }
}

impl ActiveEnvironments {
    /// Read the last active environment.
    pub fn last_active(&self) -> Option<UninitializedEnvironment> {
        self.0.front().map(|active| &active.environment).cloned()
    }

    /// Read the last active environment along with its activation metadata
    /// (mode, generation). Needed when callers must know how the env was
    /// activated, e.g. to pick the matching rendered-env link for deactivation.
    pub fn last_active_full(&self) -> Option<ActiveEnvironment> {
        self.0.front().cloned()
    }

    /// Set the last active environment.
    pub fn set_last_active(
        &mut self,
        environment: UninitializedEnvironment,
        generation: Option<GenerationId>,
        mode: ActivateMode,
    ) {
        self.0.push_front(ActiveEnvironment {
            environment,
            generation,
            mode,
        });
    }

    /// Check if the given environment is active
    pub fn is_active(&self, env: &UninitializedEnvironment) -> bool {
        self.0.iter().any(|active| &active.environment == env)
    }

    /// Return the corresponding ActiveEnvironment if the given
    /// UninitializedEnvironment is active
    pub fn get_if_active(&self, env: &UninitializedEnvironment) -> Option<&ActiveEnvironment> {
        self.0.iter().find(|active| &active.environment == env)
    }

    /// Check if the given environment is active with a generation.
    pub fn is_active_with_generation(
        &self,
        env: &UninitializedEnvironment,
    ) -> Option<GenerationId> {
        self.0
            .iter()
            .find(|active| &active.environment == env)
            .and_then(|active| active.generation)
    }

    /// Iterate over the active environments
    pub fn iter(&self) -> impl Iterator<Item = &UninitializedEnvironment> {
        self.0.iter().map(|active| &active.environment)
    }

    /// Iterate over the active environments with their activation metadata
    /// (mode, generation), most recently activated first. Needed when callers
    /// must know how an env was activated, e.g. to pick the matching
    /// rendered-env link when deactivating layers below the front of the
    /// stack.
    pub fn iter_full(&self) -> impl Iterator<Item = &ActiveEnvironment> {
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
    type IntoIter = std::iter::Map<
        std::collections::vec_deque::IntoIter<ActiveEnvironment>,
        fn(ActiveEnvironment) -> UninitializedEnvironment,
    >;
    type Item = UninitializedEnvironment;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter().map(|active| active.environment)
    }
}

/// Determine the most recently activated environment.
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

    use flox_core::data::environment_ref::EnvironmentName;
    use flox_rust_sdk::models::environment::{DotFlox, EnvironmentPointer, PathPointer};

    use super::*;

    fn new_uninitialized_environment(name: &str) -> UninitializedEnvironment {
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
        let env1 = new_uninitialized_environment("env1");
        let env2 = new_uninitialized_environment("env2");

        let mut active = ActiveEnvironments::default();
        active.set_last_active(env1.clone(), None, ActivateMode::Dev);

        assert!(active.is_active(&env1));
        assert!(!active.is_active(&env2));
    }

    #[test]
    fn test_is_active_with_generation() {
        let env1 = new_uninitialized_environment("env1");
        let env2 = new_uninitialized_environment("env2");

        let generation = Some(GenerationId::from_str("42").unwrap());
        let mut active = ActiveEnvironments::default();
        active.set_last_active(env1.clone(), generation, ActivateMode::Dev);
        assert_eq!(active.is_active_with_generation(&env1), generation);

        active.set_last_active(env2.clone(), None, ActivateMode::Dev);
        assert_eq!(active.is_active_with_generation(&env2), None);
    }

    /// Simulate setting an active environment in one flox invocation and then
    /// checking if it's active in a second.
    #[test]
    fn test_is_active_round_trip_from_env() {
        let uninitialized = new_uninitialized_environment("test");
        let mut first_active = temp_env::with_var(
            FLOX_ACTIVE_ENVIRONMENTS_VAR,
            None::<&str>,
            activated_environments,
        );

        first_active.set_last_active(uninitialized.clone(), None, ActivateMode::Dev);

        let second_active = temp_env::with_var(
            FLOX_ACTIVE_ENVIRONMENTS_VAR,
            Some(first_active.to_string()),
            activated_environments,
        );

        assert!(second_active.is_active(&uninitialized));
    }

    #[test]
    fn test_last_activated() {
        let env1 = new_uninitialized_environment("env1");
        let env2 = new_uninitialized_environment("env2");

        let mut active = ActiveEnvironments::default();
        active.set_last_active(env1, None, ActivateMode::Dev);
        active.set_last_active(env2.clone(), None, ActivateMode::Dev);

        let last_active = temp_env::with_var(
            FLOX_ACTIVE_ENVIRONMENTS_VAR,
            Some(active.to_string()),
            last_activated_environment,
        );
        assert_eq!(last_active.unwrap(), env2)
    }

    #[test]
    fn test_active_environments_forwards_compat_without_generation() {
        let env1 = new_uninitialized_environment("env1");
        let env2 = new_uninitialized_environment("env2");

        let old_format = VecDeque::from(vec![env1.clone(), env2.clone()]);
        let old_format_str = serde_json::to_string(&old_format).unwrap();
        let active = temp_env::with_var(
            FLOX_ACTIVE_ENVIRONMENTS_VAR,
            Some(old_format_str),
            activated_environments,
        );
        assert_eq!(
            active,
            ActiveEnvironments(VecDeque::from(vec![
                ActiveEnvironment {
                    environment: env1,
                    generation: None,
                    mode: ActivateMode::Dev,
                },
                ActiveEnvironment {
                    environment: env2,
                    generation: None,
                    mode: ActivateMode::Dev,
                },
            ]))
        );
    }

    /// The link-name prefix computed by
    /// `flox_activations::container_active_env::guest_env_link_prefix` must
    /// match the prefix that
    /// `flox_rust_sdk::models::environment::RenderedEnvironmentLinks::new_in_base_dir_with_name_and_system`
    /// would derive for the same env name and the current host system.
    ///
    /// `flox-activations` cannot depend on `flox-rust-sdk` (circular), so it
    /// re-derives the system string and naming formula independently.
    /// This cross-crate guard breaks CI instead of the demo if those
    /// formulas ever drift apart.
    #[test]
    fn guest_env_link_prefix_matches_rendered_env_links() {
        use flox_activations::container_active_env::guest_env_link_prefix;
        use flox_core::data::CanonicalPath;
        use flox_rust_sdk::models::environment::RenderedEnvironmentLinks;
        use tempfile::TempDir;

        let env_name = "sandbox-demo";

        // Derive the prefix via the flox-activations helper.
        // The helper formats "{ARCH}-{OS}.{name}", where ARCH and OS come
        // from std::env::consts. Container guests are always Linux, so the
        // formula yields e.g. "aarch64-linux" or "x86_64-linux", matching
        // what RenderedEnvironmentLinks would use for those NIX_TARGET_SYSTEM
        // values. The guard uses the same formula-derived system for both
        // sides to stay host-neutral: the formula must produce the same string
        // as the SDK naming convention when given the same system string, and
        // this test verifies that invariant without depending on the host OS.
        let activations_prefix = guest_env_link_prefix(env_name);
        let activations_system = format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS);
        assert_eq!(
            activations_prefix,
            format!("{activations_system}.{env_name}"),
            "guest_env_link_prefix formula is wrong"
        );

        // Feed the same system string into the SDK so both sides use the same
        // input. The SDK simply emits `{system}.{name}` as a path prefix, so
        // the structural invariant holds for any system string.
        let tmp = TempDir::new().unwrap();
        let base_dir = CanonicalPath::new(tmp.path()).unwrap();
        let sdk_links = RenderedEnvironmentLinks::new_in_base_dir_with_name_and_system(
            &base_dir,
            env_name,
            &activations_system,
        );

        // The dev link from the SDK must equal base_dir/{activations_prefix}-dev.
        let activations_dev = base_dir.join(format!("{activations_prefix}-dev"));
        let activations_run = base_dir.join(format!("{activations_prefix}-run"));

        assert_eq!(
            *sdk_links.dev, activations_dev,
            "dev link path mismatch between flox-activations and flox-rust-sdk"
        );
        assert_eq!(
            *sdk_links.run, activations_run,
            "run link path mismatch between flox-activations and flox-rust-sdk"
        );
    }

    /// The JSON that flox-activations hand-builds for a container guest
    /// (`container_active_env::container_active_environments_json`) must
    /// deserialize into a one-entry `ActiveEnvironments` whose entry is a
    /// `DotFlox`/`Path` environment that `flox deactivate` can open. This
    /// guards the hand-built shape against serde drift, since
    /// flox-activations cannot depend on flox-rust-sdk to reuse the types.
    ///
    /// Keep this literal in sync with
    /// `container_active_env::container_active_environments_json`.
    #[test]
    fn container_active_env_json_deserializes_to_openable_path_entry() {
        let dot_flox_path = "/home/user/sandbox-demo/.flox";
        let name = "sandbox-demo";
        let container_json = format!(
            "[{{\"environment\":{{\"type\":\"dot-flox\",\"path\":\"{dot_flox_path}\",\
             \"pointer\":{{\"name\":\"{name}\",\"version\":1}}}},\"mode\":\"dev\"}}]"
        );

        let active = ActiveEnvironments::from_str(&container_json)
            .expect("container active-env JSON must deserialize");

        let entry = active
            .last_active_full()
            .expect("container active-env JSON must yield one entry");

        assert_eq!(entry.mode, ActivateMode::Dev);
        assert_eq!(entry.generation, None);
        let expected_env = UninitializedEnvironment::DotFlox(DotFlox {
            path: PathBuf::from(dot_flox_path),
            pointer: EnvironmentPointer::Path(PathPointer::new(
                EnvironmentName::from_str(name).unwrap(),
            )),
        });
        assert_eq!(entry.environment, expected_env);
    }
}
