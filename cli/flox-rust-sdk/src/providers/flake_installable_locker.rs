use std::collections::{BTreeMap, VecDeque};
use std::fmt::Debug;
use std::sync::{Arc, Mutex};

use enum_dispatch::enum_dispatch;
use serde::{Deserialize, Deserializer, Serialize};
use serde_with::skip_serializing_none;
use thiserror::Error;
use tracing::{debug, instrument};

use super::nix::nix_base_command;
use crate::models::manifest::typed::{DEFAULT_PRIORITY, PackageDescriptorFlake};
use crate::models::nix_plugins::NIX_PLUGINS;
use crate::utils::CommandExt;

#[derive(Debug, Error)]
pub enum FlakeInstallableError {
    // todo: do we need to break this into more specific errors?
    #[error("Failed to lock flake installable: {0}")]
    LockInstallable(String),
    #[error("Failed to deserialize locked installable")]
    DeserializeLockedInstallable(#[from] serde_json::Error),
    #[error("Caught Nix error while locking flake:\n{0}")]
    NixError(String),
}

/// Rust representation of the output of `buitins.lockFlakeInstallable`
/// This is a direct translation of the definition in
/// `<flox>/nix-plugins/include/flox/lock-flake-installable.hh`
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
// [sic] this is inconsistent with the naming of all other structs in the lockfile
// and a relict of different naming conventions in the pkgdb/C++ code.
#[serde(rename_all = "kebab-case")]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub struct LockedInstallable {
    /// locked url of the flakeref component of the installable
    pub locked_url: String,
    pub flake_description: Option<String>,
    pub locked_flake_attr_path: String,
    pub derivation: String,
    /// Map of output names to their paths
    /// The values are expected to be nix store paths
    pub outputs: BTreeMap<String, String>,
    /// List of output names in the original order
    pub output_names: Vec<String>,
    /// List of output names to install as defined by the package
    pub outputs_to_install: Option<Vec<String>>,
    /// List of output names to install as requested by the user
    pub requested_outputs_to_install: Option<Vec<String>>,
    /// System as defined by the package
    pub package_system: String,
    /// System as specified by the manifest and used to set default attribute
    /// paths when locking the installable
    pub system: String,
    pub name: String,
    pub pname: Option<String>,
    pub version: Option<String>,
    pub description: Option<String>,
    pub licenses: Option<Vec<String>>,
    pub broken: Option<bool>,
    pub unfree: Option<bool>,
    // In the lockfile, the priority should always be known.
    // Usage of the output type of `buitins.lockFlakeInstallable`,
    // however requires guarding against a missing priority.
    // Since the default priority is not known statically,
    // we assign it as a default value during deserialization.
    #[serde(
        deserialize_with = "locked_installable_default_priority_on_null",
        default = "locked_installable_default_priority_on_undefined"
    )]
    pub priority: u64,
}

/// Deserialize the priority field of a locked installable.
/// `buitins.lockFlakeInstallable` will yield a `null` priority
/// if the priority is not set, which requires a custom deserializer
/// to set the default priority.
fn locked_installable_default_priority_on_null<'de, D>(d: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    Deserialize::deserialize(d).map(|x: Option<_>| x.unwrap_or(DEFAULT_PRIORITY))
}

/// Default priority for a locked installable if the priority is not set,
/// as we may remove null attributes during serialization.
fn locked_installable_default_priority_on_undefined() -> u64 {
    DEFAULT_PRIORITY
}

/// Required functionality to lock a flake installable
///
/// Implemented as a trait to allow mocking as evaluation is
/// time-consuming unless cached.
///
/// The trait is implemented by the [Nix] struct which is the canonical implementation
/// using the `buitins.lockFlakeInstallable` primop.
///
/// The trait is also implemented by the [`InstallableLockerMock`] struct which is used for testing.
#[enum_dispatch]
pub trait InstallableLocker {
    fn lock_flake_installable(
        &self,
        system: impl AsRef<str>,
        descriptor: &PackageDescriptorFlake,
    ) -> Result<LockedInstallable, FlakeInstallableError>;
}

#[derive(Debug)]
#[enum_dispatch(InstallableLocker)]
pub enum InstallableLockerImpl {
    Mock(InstallableLockerMock),
    Nix(Nix),
}

impl Default for InstallableLockerImpl {
    fn default() -> Self {
        InstallableLockerImpl::Nix(Nix)
    }
}

/// Sets the priority for the locked installable.
///
/// The priority order of...the priority is:
/// - Priority set in the descriptor
/// - `meta.priority` of the derivation
/// - Default priority
fn set_priority(locked: &mut LockedInstallable, descriptor: &PackageDescriptorFlake) {
    if let Some(priority) = descriptor.priority {
        locked.priority = priority;
    }
}

/// Mock implementation of [`InstallableLocker`] for testing.
#[derive(Debug, Default)]
pub struct InstallableLockerMock {
    lock_flake_installable: Arc<Mutex<VecDeque<Result<LockedInstallable, FlakeInstallableError>>>>,
}

impl InstallableLockerMock {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_lock_result(&self, result: Result<LockedInstallable, FlakeInstallableError>) {
        self.lock_flake_installable
            .lock()
            .unwrap()
            .push_back(result);
    }

    #[allow(unused)]
    fn set_lock_results(
        &self,
        results: impl IntoIterator<Item = Result<LockedInstallable, FlakeInstallableError>>,
    ) {
        let mut queue = self.lock_flake_installable.lock().unwrap();
        queue.clear();
        queue.extend(results);
    }
}

impl InstallableLocker for InstallableLockerMock {
    fn lock_flake_installable(
        &self,
        system: impl AsRef<str>,
        descriptor: &PackageDescriptorFlake,
    ) -> Result<LockedInstallable, FlakeInstallableError> {
        let mut mocked_result = self
            .lock_flake_installable
            .lock()
            .unwrap()
            .pop_front()
            .expect("no more mock results");

        debug!(
            system=system.as_ref(),
            installable=&descriptor.flake,
            mocked_result=?mocked_result,
            "responding with mocked result"
        );

        // Same logic as the real locker
        if let Ok(ref mut lock) = mocked_result {
            set_priority(lock, descriptor);
        }

        mocked_result
    }
}

#[derive(Debug)]
pub struct Nix;
impl InstallableLocker for Nix {
    #[instrument(skip_all, fields(
        system = system.as_ref(),
        descriptor = descriptor.flake,
        progress = format!(
            "Locking flake installable '{}' for '{}'",
            descriptor.flake, system.as_ref())
    ))]
    fn lock_flake_installable(
        &self,
        system: impl AsRef<str>,
        descriptor: &PackageDescriptorFlake,
    ) -> Result<LockedInstallable, FlakeInstallableError> {
        let mut command = nix_base_command();
        command.args(["--option", "extra-plugin-files", &*NIX_PLUGINS]);

        command.args(["--option", "pure-eval", "false"]);
        command.arg("eval");
        command.arg("--no-update-lock-file");
        command.arg("--no-write-lock-file");
        command.arg("--json");
        command.args(["--system", system.as_ref()]);
        command.args([
            "--expr",
            &format!(r#"builtins.lockFlakeInstallable "{}""#, descriptor.flake),
        ]);

        debug!(cmd=%command.display(), "running nix evaluation");

        let output = command
            .output()
            .map_err(|e| FlakeInstallableError::NixError(e.to_string()))?;

        if !output.status.success() {
            return Err(FlakeInstallableError::LockInstallable(
                String::from_utf8_lossy(&output.stderr).to_string(),
            ));
        }

        let mut lock = serde_json::from_slice(&output.stdout)
            .map_err(FlakeInstallableError::DeserializeLockedInstallable)?;

        set_priority(&mut lock, descriptor);

        Ok(lock)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use indoc::formatdoc;
    use url::Url;

    use super::*;
    use crate::flox::test_helpers::flox_instance;
    use crate::models::environment::Environment;
    use crate::models::environment::path_environment::test_helpers::new_path_environment;
    use crate::models::manifest::raw::{FlakePackage, PackageToInstall};

    /// Returns the path to a bundled flake that contains a number of test packages
    /// for sped up evaluation
    fn local_test_flake() -> String {
        let manifest_root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let local_test_flake_path = manifest_root
            .join("../../nix-plugins/tests/data/lock-flake-installable")
            .canonicalize()
            .unwrap();
        local_test_flake_path.to_str().unwrap().to_string()
    }

    /// Test that the output of `buitins.lockFlakeInstallable` can be deserialized
    /// into a [LockedFlakeInstallble] struct.
    #[test]
    fn test_output_format() {
        // `$system` is set by the nix devshell
        let system = env!("system");
        let installable = format!("{flake}#hello", flake = local_test_flake());

        // make sure the deserialization is not accidentally optimized away
        Nix.lock_flake_installable(system, &PackageDescriptorFlake {
            flake: installable,
            priority: None,
            systems: None,
        })
        .expect("locking local test flake should succeed");
    }

    #[test]
    fn test_catches_absent_flake() {
        let system = env!("system");
        let installable = "github:flox/trust-this-wont-be-added#hello";

        let result = Nix.lock_flake_installable(system, &PackageDescriptorFlake {
            flake: installable.to_string(),
            priority: None,
            systems: None,
        });
        assert!(
            matches!(result, Err(FlakeInstallableError::LockInstallable(_))),
            "{result:#?}"
        );
    }

    #[test]
    fn test_catches_absent_flake_output() {
        let system = env!("system");
        let installable = format!("{flake}#nonexistent", flake = local_test_flake());

        let result = Nix.lock_flake_installable(system, &PackageDescriptorFlake {
            flake: installable,
            priority: None,
            systems: None,
        });

        assert!(
            matches!(result, Err(FlakeInstallableError::LockInstallable(_))),
            "{result:#?}"
        );
    }

    #[test]
    fn catches_nix_eval_errors() {
        let (mut flox, _temp_dir) = flox_instance();
        flox.installable_locker = InstallableLockerImpl::Nix(Nix);
        let manifest = formatdoc! {r#"
        version =  1
        "#};
        let mut env = new_path_environment(&flox, &manifest);
        let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let flake_dir = crate_root
            .join("../tests/flakes/teeny-tiny-failure")
            .canonicalize()
            .unwrap();
        let pkgs = [PackageToInstall::Flake(FlakePackage {
            id: "gonna_fail".to_string(),
            url: Url::parse(&format!("path:{}", flake_dir.display())).unwrap(),
        })];
        let res = temp_env::with_var("_PKGDB_ALLOW_LOCAL_FLAKE", Some("1"), || {
            env.install(&pkgs, &flox)
        });
        if let Err(e) = res {
            eprintln!("Error: {:?}", e);
            let err_string = e.to_string();
            let has_nix_error = err_string.contains("I'm broken inside")
                || err_string.contains("cached failure of attribute");
            assert!(has_nix_error);
        } else {
            panic!("expected an error");
        }
    }

    #[test]
    fn fills_in_priority() {
        let locked_hello = r#"
        {
            "broken": false,
            "derivation": "/nix/store/4w0wsrlfad3ilqjxk34fnkmdckiq0k0m-hello-2.12.1.drv",
            "description": "Program that produces a familiar, friendly greeting",
            "flake-description": "A collection of packages for the Nix package manager",
            "licenses": [
                "GPL-3.0-or-later"
            ],
            "locked-flake-attr-path": "legacyPackages.aarch64-darwin.hello",
            "locked-url": "github:NixOS/nixpkgs/56bf14fe1c5ba088fff3f337bc0cdf28c8227f81",
            "name": "hello-2.12.1",
            "output-names": [
                "out"
            ],
            "outputs": {
                "out": "/nix/store/ia1pdwpvhswwnbamqkzbz69ja02bjfqx-hello-2.12.1"
            },
            "outputs-to-install": [
                "out"
            ],
            "package-system": "aarch64-darwin",
            "pname": "hello",
            "priority": null,
            "requested-outputs-to-install": null,
            "system": "aarch64-darwin",
            "unfree": false,
            "version": "2.12.1"
        }
        "#;
        let mut locked: LockedInstallable = serde_json::from_str(locked_hello).unwrap();
        let descriptor = PackageDescriptorFlake {
            flake: "github:NixOS/nipxkgs#hello".to_string(),
            priority: Some(10),
            systems: None,
        };
        set_priority(&mut locked, &descriptor);
        assert_eq!(locked.priority, 10);
    }

    #[test]
    fn falls_back_to_default_priority() {
        let locked_hello = r#"
        {
            "broken": false,
            "derivation": "/nix/store/4w0wsrlfad3ilqjxk34fnkmdckiq0k0m-hello-2.12.1.drv",
            "description": "Program that produces a familiar, friendly greeting",
            "flake-description": "A collection of packages for the Nix package manager",
            "licenses": [
                "GPL-3.0-or-later"
            ],
            "locked-flake-attr-path": "legacyPackages.aarch64-darwin.hello",
            "locked-url": "github:NixOS/nixpkgs/56bf14fe1c5ba088fff3f337bc0cdf28c8227f81",
            "name": "hello-2.12.1",
            "output-names": [
                "out"
            ],
            "outputs": {
                "out": "/nix/store/ia1pdwpvhswwnbamqkzbz69ja02bjfqx-hello-2.12.1"
            },
            "outputs-to-install": [
                "out"
            ],
            "package-system": "aarch64-darwin",
            "pname": "hello",
            "requested-outputs-to-install": null,
            "system": "aarch64-darwin",
            "unfree": false,
            "version": "2.12.1"
        }
        "#;
        let mut locked: LockedInstallable = serde_json::from_str(locked_hello).unwrap();
        let descriptor = PackageDescriptorFlake {
            flake: "github:NixOS/nipxkgs#hello".to_string(),
            priority: None,
            systems: None,
        };
        set_priority(&mut locked, &descriptor);
        assert_eq!(locked.priority, DEFAULT_PRIORITY);
    }
}
