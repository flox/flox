use std::collections::HashSet;
use std::fmt;
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use fslock::LockFile;
use nix::errno::Errno;
use nix::sys::signal::kill;
use nix::unistd::Pid as NixPid;
use serde::{Deserialize, Serialize};
use tracing::debug;

use super::environment::{path_hash, EnvironmentPointer};
use crate::data::{CanonicalPath, Version};
use crate::flox::Flox;
use crate::utils::traceable_path;

pub const ENV_REGISTRY_FILENAME: &str = "env-registry.json";

/// Errors encountered while interacting with the environment registry.
#[derive(Debug, thiserror::Error)]
pub enum EnvRegistryError {
    #[error("couldn't acquire environment registry file lock")]
    AcquireLock(#[source] fslock::Error),
    #[error("couldn't open environment registry file")]
    OpenRegistry(#[source] std::io::Error),
    #[error("couldn't parse environment registry")]
    ParseRegistry(#[source] serde_json::Error),
    #[error("failed to open temporary file for registry")]
    OpenTmpRegistry(#[source] std::io::Error),
    #[error("failed to write temporary environment registry file")]
    WriteTmpRegistry(#[source] serde_json::Error),
    #[error("failed to rename temporary registry file")]
    RenameRegistry(#[source] tempfile::PersistError),
    #[error("registry file stored in an invalid location: {0}")]
    InvalidRegistryLocation(PathBuf),
    #[error("no environments registered with key: {0}")]
    UnknownKey(String),
    #[error("did not find environment in registry")]
    EnvNotRegistered,
}

/// A local registry of environments on the system.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary, PartialEq))]
pub struct EnvRegistry {
    /// The schema version of the local environment registry file.
    pub version: Version<1>,
    /// The list of locations at which environments can be found and the metadata about
    /// the environments that have existed there.
    // Note: We use this ugly macro to generate fewer `RegistryEntry`s than `proptest`s default,
    //       and this makes a _huge_ difference in test execution speed.
    #[cfg_attr(
        test,
        proptest(
            strategy = "proptest::collection::vec(proptest::arbitrary::any::<RegistryEntry>(), 0..=3)"
        )
    )]
    pub entries: Vec<RegistryEntry>,
}

impl EnvRegistry {
    /// Returns the [RegistryEntry] that corresponds to the provided path hash, if it exists.
    pub fn entry_for_hash_mut(&mut self, hash: &str) -> Option<&mut RegistryEntry> {
        self.entries
            .iter_mut()
            .find(|entry| entry.path_hash == hash)
    }

    /// Returns the [RegistryEntry] that corresponds to the provided path hash, if it exists.
    pub fn entry_for_hash(&self, hash: &str) -> Option<&RegistryEntry> {
        self.entries.iter().find(|entry| entry.path_hash == hash)
    }

    /// Returns the path associated with a particular hash
    pub fn path_for_hash(&self, hash: &str) -> Result<PathBuf, EnvRegistryError> {
        let entry = self
            .entry_for_hash(hash)
            .ok_or(EnvRegistryError::EnvNotRegistered)?;
        Ok(entry.path.clone())
    }

    /// Registers the environment, creating a new [RegistryEntry] if necessary and returning the
    /// [RegisteredEnv] that was created. If the environment was already it returns `Ok(None)`.
    fn register_env(
        &mut self,
        dot_flox_path: &impl AsRef<Path>,
        hash: &str,
        env_pointer: &EnvironmentPointer,
    ) -> Result<Option<RegisteredEnv>, EnvRegistryError> {
        let entry = match self.entry_for_hash_mut(hash) {
            Some(entry) => entry,
            None => {
                self.entries.push(RegistryEntry {
                    path_hash: hash.to_string(),
                    path: dot_flox_path.as_ref().to_path_buf(),
                    activations: HashSet::new(),
                    envs: vec![],
                });
                self.entries
                    .last_mut()
                    .expect("didn't find registry entry that was just pushed")
            },
        };
        entry.register_env(env_pointer)
    }

    /// Deregisters and returns the latest entry if it is the same type of environment and has
    /// the same pointer.
    fn deregister_env(
        &mut self,
        hash: &str,
        env_pointer: &EnvironmentPointer,
    ) -> Result<RegisteredEnv, EnvRegistryError> {
        let entry = self
            .entry_for_hash_mut(hash)
            .ok_or(EnvRegistryError::UnknownKey(hash.to_string()))?;
        let res = entry
            .deregister_env(env_pointer)
            .ok_or(EnvRegistryError::EnvNotRegistered);
        // Remove the entry if it's empty. We use [Vec::retain] because the entry doesn't
        // track its own index.
        if entry.envs.is_empty() {
            self.entries.retain(|e| e.path_hash != hash);
        }
        res
    }
}

/// Metadata about the location at which one or more environments were registered over time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RegistryEntry {
    /// The truncated hash of the path to the environment.
    #[serde(rename = "hash")]
    pub path_hash: String,
    /// The path to the environment's `.flox` directory
    pub path: PathBuf,
    /// The list of environments that have existed at this path
    /// since the last time environments were garbage collected.
    pub envs: Vec<RegisteredEnv>,
    /// The PIDs of current activations
    #[serde(default)]
    pub activations: HashSet<Pid>,
}

impl RegistryEntry {
    /// Returns the latest environment registered at this location.
    pub fn latest_env(&self) -> Option<&RegisteredEnv> {
        self.envs.iter().last()
    }

    /// Adds the environment to the list of registered environments. This is a no-op if the latest
    /// registered environment has the same environment pointer, which indicates that it's the
    /// currently registered environment.
    fn register_env(
        &mut self,
        env_pointer: &EnvironmentPointer,
    ) -> Result<Option<RegisteredEnv>, EnvRegistryError> {
        // Bail early if the environment is the same as the latest registered one
        if self.is_same_as_latest_env(env_pointer) {
            return Ok(None);
        }

        // [SystemTime] isn't guaranteed to be monotonic, so we account for that by setting the
        // `created_at` time to be `max(now, latest_created_at)`.
        let now = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("'now' was earlier than UNIX epoch")
            .as_secs();
        let created_at = if let Some(RegisteredEnv { created_at, .. }) = self.latest_env() {
            now.max(*created_at)
        } else {
            now
        };

        let env = RegisteredEnv {
            created_at,
            pointer: env_pointer.clone(),
        };
        self.envs.push(env.clone());
        Ok(Some(env))
    }

    /// Returns true if there is a latest registered environment that is a managed environment with
    /// the same
    pub fn is_same_as_latest_env(&self, ptr: &EnvironmentPointer) -> bool {
        if let Some(RegisteredEnv { pointer, .. }) = self.latest_env() {
            return pointer == ptr;
        }
        false
    }

    /// Deregisters and returns the latest entry if it is the same type of environment and has
    /// the same pointer.
    fn deregister_env(&mut self, ptr: &EnvironmentPointer) -> Option<RegisteredEnv> {
        if self.is_same_as_latest_env(ptr) {
            return Some(self.envs.pop().expect("envs was assumed to be non-empty"));
        }
        None
    }

    /// Register an activation for an existing enviroment.
    fn register_activation(&mut self, pid: Pid) {
        tracing::debug!("registering activation: {}", &pid);
        self.activations.insert(pid);
    }

    /// Deregister an activation for an existing enviroment.
    fn deregister_activation(&mut self, pid: Pid) {
        tracing::debug!("deregistering activation: {}", &pid);
        self.activations.remove(&pid);
    }

    /// Remove any activation PIDs that are no longer running and weren't explicitly deregistered.
    fn remove_stale_activations(&mut self) {
        let stale_pids: Vec<Pid> = self
            .activations
            .iter()
            .filter(|pid| !pid.is_running())
            .cloned()
            .collect();

        for pid in stale_pids {
            tracing::debug!("removing stale activation: {}", &pid);
            self.activations.remove(&pid);
        }
    }
}

/// Metadata about an environment that has been registered.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub struct RegisteredEnv {
    /// The time at which this environment was registered in seconds since the Unix Epoch.
    pub created_at: u64,
    /// The metadata about the owner and name of the environment if this environment is a
    /// managed environment.
    pub pointer: EnvironmentPointer,
}

/// PID of an environment's activation.
#[derive(Debug, Clone, Serialize, Deserialize, Hash, Eq, PartialEq)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub struct Pid(u32);

impl Pid {
    /// Construct a Pid from the current running process.
    pub fn from_self() -> Self {
        Pid(nix::unistd::getpid().as_raw() as u32)
    }

    /// Check whether an activation is still running.
    fn is_running(&self) -> bool {
        // TODO: Compare name or check for watchdog child to see if it's a real activation?
        let pid = NixPid::from_raw(self.0 as i32);
        match kill(pid, None) {
            Ok(_) => true,              // known running
            Err(Errno::EPERM) => true,  // no perms but running
            Err(Errno::ESRCH) => false, // known not running
            Err(_) => false,            // assumed not running
        }
    }
}

impl fmt::Display for Pid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Returns the path to the user's environment registry file.
pub fn env_registry_path(flox: &Flox) -> PathBuf {
    flox.data_dir.join(ENV_REGISTRY_FILENAME)
}

/// Returns the path to the user's environment registry lock file.
pub(crate) fn env_registry_lock_path(reg_path: impl AsRef<Path>) -> PathBuf {
    reg_path.as_ref().with_extension("lock")
}

/// Returns the parsed environment registry file or `None` if it doesn't yet exist.
pub fn read_environment_registry(
    path: impl AsRef<Path>,
) -> Result<Option<EnvRegistry>, EnvRegistryError> {
    let path = path.as_ref();
    if !path.exists() {
        debug!(
            path = traceable_path(&path),
            "environment registry not found"
        );
        return Ok(None);
    }
    let f = File::open(path).map_err(EnvRegistryError::OpenRegistry)?;
    let reader = BufReader::new(f);
    let parsed: EnvRegistry =
        serde_json::from_reader(reader).map_err(EnvRegistryError::ParseRegistry)?;
    Ok(Some(parsed))
}

/// Writes the environment registry to disk.
///
/// First the registry is written to a temporary file and then it is renamed so the write appears
/// atomic. This also takes a [LockFile] argument to ensure that the write can only be performed
/// when the lock is acquired. It is a bug if you pass a [LockFile] that doesn't correspond to the
/// environment registry, as that is essentially bypassing the lock.
pub fn write_environment_registry(
    reg: &EnvRegistry,
    reg_path: &impl AsRef<Path>,
    _lock: LockFile,
) -> Result<(), EnvRegistryError> {
    // We use a temporary directory here instead of a temporary file because it allows us to get the
    // path, which we can't easily get from a temporary file.
    let parent = reg_path.as_ref().parent().ok_or(
        // This error is thrown in the unlikely scenario that `reg_path` is:
        // - An empty string
        // - `/`
        // - `.`
        EnvRegistryError::InvalidRegistryLocation(reg_path.as_ref().to_path_buf()),
    )?;
    let temp_file =
        tempfile::NamedTempFile::new_in(parent).map_err(EnvRegistryError::OpenTmpRegistry)?;

    let writer = BufWriter::new(&temp_file);
    serde_json::to_writer_pretty(writer, reg).map_err(EnvRegistryError::WriteTmpRegistry)?;
    temp_file
        .persist(reg_path.as_ref())
        .map_err(EnvRegistryError::RenameRegistry)?;
    Ok(())
}

/// Acquires the filesystem-based lock on the user's environment registry file
pub fn acquire_env_registry_lock(reg_path: impl AsRef<Path>) -> Result<LockFile, EnvRegistryError> {
    let lock_path = env_registry_lock_path(reg_path);
    LockFile::open(lock_path.as_os_str()).map_err(EnvRegistryError::AcquireLock)
}

/// Ensures that the environment is registered. This is a no-op if it is already registered.
pub fn ensure_registered(
    flox: &Flox,
    dot_flox_path: &CanonicalPath,
    env_pointer: &EnvironmentPointer,
) -> Result<(), EnvRegistryError> {
    // Acquire the lock before reading the registry so that we know there are no modifications while
    // we're editing it.
    let reg_path = env_registry_path(flox);
    let lock = acquire_env_registry_lock(&reg_path)?;
    let mut reg = read_environment_registry(&reg_path)?.unwrap_or_default();
    let dot_flox_hash = path_hash(&dot_flox_path);
    // Skip writing the registry if the environment was already registered
    if reg
        .register_env(dot_flox_path, &dot_flox_hash, env_pointer)?
        .is_some()
    {
        write_environment_registry(&reg, &reg_path, lock)?;
    }
    Ok(())
}

/// Deletes the environment from the registry.
///
/// The deleted environment must be of the same type as the requested environment as indicated by
/// the presence of a [ManagedPointer].
pub fn deregister(
    flox: &Flox,
    dot_flox_path: &CanonicalPath,
    env_pointer: &EnvironmentPointer,
) -> Result<(), EnvRegistryError> {
    // Acquire the lock before reading the registry so that we know there are no modifications while
    // we're editing it.
    let reg_path = env_registry_path(flox);
    let lock = acquire_env_registry_lock(&reg_path)?;
    let mut reg = read_environment_registry(&reg_path)?.unwrap_or_default();
    let dot_flox_hash = path_hash(&dot_flox_path);
    reg.deregister_env(&dot_flox_hash, env_pointer)?;
    write_environment_registry(&reg, &reg_path, lock)?;
    Ok(())
}

/// Register an activation for an existing enviroment.
pub fn register_activation(
    reg_path: impl AsRef<Path>,
    path_hash: &str,
    pid: Pid,
) -> Result<(), EnvRegistryError> {
    // Acquire the lock before reading the registry so that we know there are no modifications while
    // we're editing it.
    let lock = acquire_env_registry_lock(&reg_path)?;
    let mut reg = read_environment_registry(&reg_path)?.unwrap_or_default();
    let entry = reg
        .entry_for_hash_mut(path_hash)
        .ok_or(EnvRegistryError::UnknownKey(path_hash.to_string()))?;

    entry.remove_stale_activations();
    entry.register_activation(pid);

    write_environment_registry(&reg, &reg_path, lock)?;
    Ok(())
}

/// Deregister an activation for an existing enviroment.
pub fn deregister_activation(
    reg_path: impl AsRef<Path>,
    path_hash: &str,
    pid: Pid,
) -> Result<(), EnvRegistryError> {
    // Acquire the lock before reading the registry so that we know there are no modifications while
    // we're editing it.
    let lock = acquire_env_registry_lock(&reg_path)?;
    let mut reg = read_environment_registry(&reg_path)?.unwrap_or_default();
    let entry = reg
        .entry_for_hash_mut(path_hash)
        .ok_or(EnvRegistryError::UnknownKey(path_hash.to_string()))?;

    entry.deregister_activation(pid);
    entry.remove_stale_activations();

    write_environment_registry(&reg, &reg_path, lock)?;
    Ok(())
}

#[cfg(test)]
mod test {
    use std::fs::OpenOptions;
    use std::process::{Child, Command};

    use proptest::arbitrary::{any, Arbitrary};
    use proptest::collection::{hash_set, vec};
    use proptest::path::PathParams;
    use proptest::strategy::{BoxedStrategy, Just, Strategy};
    use proptest::{prop_assert, prop_assert_eq, prop_assume, proptest};
    use tempfile::tempdir;

    use super::*;
    use crate::flox::test_helpers::flox_instance;

    impl Arbitrary for RegistryEntry {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_: Self::Parameters) -> Self::Strategy {
            // Creates a RegistryEntry with the following guarantees:
            // - The hash is the actual hash of the path, though the path may not exist.
            // - The registered envs are sorted in ascending order by `created_at`, as they would
            //   be in reality.
            (
                PathBuf::arbitrary_with(PathParams::default().with_components(1..3)),
                hash_set((1u32..=65535).prop_map(Pid), 0..20),
                vec(any::<RegisteredEnv>(), 0..=3),
            )
                .prop_flat_map(|(path, activation_pids, mut registered_envs)| {
                    registered_envs.sort_by_cached_key(|e| e.created_at);
                    (
                        Just(path.clone()),
                        Just(path_hash(&path)),
                        Just(activation_pids),
                        Just(registered_envs),
                    )
                })
                .prop_map(|(path, hash, activation_pids, envs)| RegistryEntry {
                    path_hash: hash.to_string(),
                    path,
                    activations: activation_pids,
                    envs,
                })
                .boxed()
        }
    }

    proptest! {
        #[test]
        fn can_roundtrip(reg: EnvRegistry) {
            let serialized = serde_json::to_string(&reg).unwrap();
            let deserialized = serde_json::from_str::<EnvRegistry>(&serialized).unwrap();
            prop_assert_eq!(reg, deserialized);
        }

        #[test]
        fn reads_registry(reg: EnvRegistry) {
            let tmp_dir = tempdir().unwrap();
            let reg_path = tmp_dir.path().join(ENV_REGISTRY_FILENAME);
            let file = OpenOptions::new().write(true).create_new(true).open(&reg_path).unwrap();
            let writer = BufWriter::new(file);
            serde_json::to_writer(writer, &reg).unwrap();
            let reg_read = read_environment_registry(&reg_path).unwrap().unwrap();
            prop_assert_eq!(reg, reg_read);
        }

        #[test]
        fn writes_registry(reg: EnvRegistry) {
            let (flox, _temp_dir_handle) = flox_instance();
            let reg_path = env_registry_path(&flox);
            let lock_path = env_registry_lock_path(&reg_path);
            let lock = LockFile::open(&lock_path).unwrap();
            prop_assert!(!reg_path.exists());
            write_environment_registry(&reg, &reg_path, lock).unwrap();
            prop_assert!(reg_path.exists());
        }

        #[test]
        fn new_env_added_to_reg_entry(mut entry: RegistryEntry, ptr: EnvironmentPointer) {
            // Skip cases where they're the same since that's a no-op
            prop_assume!(!entry.is_same_as_latest_env(&ptr));
            let previous_envs = entry.envs.clone();
            entry.register_env(&ptr).unwrap();
            let new_envs = entry.envs;
            prop_assert!(new_envs.len() == previous_envs.len() + 1);
            let latest_ptr = new_envs.into_iter().last().unwrap().pointer;
            prop_assert_eq!(latest_ptr, ptr);
        }

        #[test]
        fn noop_on_existing_env(mut entry: RegistryEntry, ptr: EnvironmentPointer) {
            if entry.is_same_as_latest_env(&ptr) {
                let previous_envs = entry.envs.clone();
                entry.register_env(&ptr).unwrap();
                let new_envs = entry.envs.clone();
                prop_assert_eq!(previous_envs, new_envs);
            } else {
                entry.register_env(&ptr).unwrap();
                let previous_envs = entry.envs.clone();
                entry.register_env(&ptr).unwrap();
                let new_envs = entry.envs.clone();
                prop_assert_eq!(previous_envs, new_envs);
            }
        }

        #[test]
        fn none_on_nonexistent_registry_file(path: PathBuf) {
            prop_assume!(path != PathBuf::from(""));
            prop_assume!(!path.exists() || path.is_file());
            prop_assert!(read_environment_registry(path).unwrap().is_none())
        }

        #[test]
        fn ensures_new_registration(existing_reg: EnvRegistry, ptr: EnvironmentPointer) {
            // Make sure all the directories exist
            let (flox, tmp_dir) = flox_instance();
            let dot_flox_path = tmp_dir.path().join(".flox");
            std::fs::create_dir_all(&dot_flox_path).unwrap();
            let canonical_dot_flox_path = CanonicalPath::new(&dot_flox_path).unwrap();
            // Seed the existing registry
            let reg_contents = serde_json::to_string(&existing_reg).unwrap();
            let reg_path = env_registry_path(&flox);
            std::fs::write(&reg_path, reg_contents).unwrap();
            // Do the registration
            ensure_registered(&flox, &canonical_dot_flox_path, &ptr).unwrap();
            // Check the registration
            let new_reg = read_environment_registry(&reg_path).unwrap().unwrap();
            let expected_hash = path_hash(&canonical_dot_flox_path);
            let entry = new_reg.entry_for_hash(&expected_hash).unwrap();
            prop_assert_eq!(&entry.latest_env().as_ref().unwrap().pointer, &ptr);
        }

        #[test]
        fn registered_envs_remain_sorted(mut entry: RegistryEntry, new_envs in vec(any::<EnvironmentPointer>(), 0..=3)) {
            let mut envs_before = entry.envs.clone();
            envs_before.sort_by_cached_key(|e| e.created_at);
            prop_assert_eq!(&envs_before, &entry.envs);
            for env in new_envs {
                entry.register_env(&env).unwrap();
                let mut sorted_envs = entry.envs.clone();
                sorted_envs.sort_by_cached_key(|e| e.created_at);
                prop_assert_eq!(&entry.envs, &sorted_envs);
            }
        }

        #[test]
        fn entries_deregister_envs(mut entry: RegistryEntry) {
            prop_assume!(!entry.envs.is_empty());
            let latest_env = entry.envs.iter().last().unwrap().clone();
            let n_envs_before = entry.envs.len();
            let removed = entry.deregister_env(&latest_env.pointer).unwrap();
            let n_envs_after = entry.envs.len();
            prop_assert_eq!(n_envs_after + 1, n_envs_before);
            prop_assert_eq!(latest_env, removed);
        }

        #[test]
        fn registry_deregisters_envs(mut reg: EnvRegistry) {
            prop_assume!(!reg.entries.is_empty());
            prop_assume!(!reg.entries[0].envs.is_empty());
            let hash = reg.entries[0].path_hash.clone();
            let envs_to_deregister = reg.entries[0].envs.iter().cloned().rev().collect::<Vec<_>>();
            for env in envs_to_deregister.iter() {
                let deregistered = reg.deregister_env(&hash, &env.pointer).unwrap();
                prop_assert_eq!(&deregistered, env);
            }
            // Empty entries should be removed
            prop_assert!(reg.entry_for_hash(&hash).is_none());
        }

        #[test]
        fn entries_register_activation(mut entry: RegistryEntry, activation: Pid) {
            entry.register_activation(activation.clone());
            prop_assert!(entry.activations.contains(&activation));
        }

        #[test]
        fn entries_deregister_activation(mut entry: RegistryEntry) {
            prop_assume!(!entry.activations.is_empty());
            let activations = entry.activations.clone();
            let activation = activations.iter().next().unwrap();
            entry.deregister_activation(activation.clone());
            prop_assert!(!entry.activations.contains(&activation));
        }
    }

    /// Start a shortlived process that we can check the PID is running.
    fn start_process() -> Child {
        Command::new("sleep")
            .arg("2")
            .spawn()
            .expect("failed to start")
    }

    /// Stop a shortlived process that we can check the PID is not running. It's
    /// unlikely, but not impossible, that the kernel will have not re-used the
    /// PID by the time we check it.
    fn stop_process(mut child: Child) {
        child.kill().expect("failed to kill");
        child.wait().expect("failed to wait");
    }

    #[test]
    fn test_pid_is_running_lifecycle() {
        let child = start_process();

        let pid = Pid(child.id());
        assert!(pid.is_running());

        stop_process(child);
        assert!(!pid.is_running());
    }

    #[test]
    fn test_pid_is_running_pid1() {
        // PID 1 is always running on Linux and MacOS but we don't have perms to send signals.
        assert!(Pid(1).is_running());
    }

    #[test]
    fn test_remove_stale_activations() {
        let child1 = start_process();
        let child2 = start_process();
        let activations_before = HashSet::from([Pid(1), Pid(child1.id()), Pid(child2.id())]);
        let mut entry = RegistryEntry {
            path: PathBuf::from("foo"),
            path_hash: String::from("foo"),
            envs: vec![],
            activations: activations_before.clone(),
        };
        entry.remove_stale_activations();
        assert_eq!(entry.activations, activations_before);

        stop_process(child1);
        stop_process(child2);
        entry.remove_stale_activations();
        assert_eq!(entry.activations, HashSet::from([Pid(1)]));
    }
}
