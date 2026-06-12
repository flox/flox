//! The sensitive-path set: credential and secret locations the sandbox treats
//! specially.
//!
//! Two consumers share this set, which is why it lives in the public `sandbox`
//! module rather than inside the broker:
//!
//! - The broker (`control`/`pending`) consults it so a sensitive path is never
//!   rolled into a directory-scope suggestion — a single "allow everything in
//!   `~/.aws/`" grant must never be offered for a credentials directory.
//! - The `flox sandbox` review CLI consults it so the interactive review omits
//!   the directory-scope option entirely when the pending path is sensitive,
//!   matching the mockup where `~/.aws/credentials` offers only file-scoped
//!   choices.
//!
//! The default set mirrors libsandbox's compiled-in sensitive globs so the two
//! layers agree on what counts as a secret. `FLOX_SANDBOX_SENSITIVE` overrides
//! it (space-separated globs) for testing and for deployments with a different
//! secret layout.
//!
//! Matching is fnmatch via `glob::Pattern`, the same matcher the grant set and
//! the engine's scope cache use, so "sensitive" means the same thing
//! everywhere. Patterns are matched against the resolved request path; entries
//! that begin with `~/` are expanded against `$HOME` before matching, and a
//! bare `**/...` entry matches at any depth.

use std::path::Path;

/// `FLOX_SANDBOX_SENSITIVE` — overrides the default sensitive set with a
/// space-separated list of globs. Empty or unset uses [`DEFAULT_SENSITIVE`].
pub const FLOX_SANDBOX_SENSITIVE_VAR: &str = "FLOX_SANDBOX_SENSITIVE";

/// The compiled-in sensitive globs, mirroring libsandbox's default set.
///
/// These are credential and secret locations: SSH keys, cloud credentials,
/// GPG and Kubernetes config, `.netrc`, GitHub CLI tokens, and any `.env`
/// file. `.flox/cache/sandbox/**` is included so the grants file and journal
/// themselves are never folded into a directory grant — that would let one
/// approval silence the tamper-evidence path.
///
/// `~/` entries are `$HOME`-relative; `**/` entries match at any depth.
pub const DEFAULT_SENSITIVE: &[&str] = &[
    "~/.ssh/**",
    "~/.aws/**",
    "~/.gnupg/**",
    "~/.kube/**",
    "~/.netrc",
    "~/.config/gh/**",
    "**/.env*",
    "**/.flox/cache/sandbox/**",
];

/// A compiled sensitive set: globs expanded against `$HOME` once, then matched
/// against request paths.
#[derive(Debug, Clone)]
pub struct SensitiveSet {
    /// fnmatch patterns, `$HOME`-expanded, ready to match against realpaths.
    patterns: Vec<String>,
}

impl SensitiveSet {
    /// Build the sensitive set, honoring `FLOX_SANDBOX_SENSITIVE` if set.
    ///
    /// `home` is the user's home directory used to expand `~/` entries; when
    /// `None`, `~/` entries are kept verbatim (they then only match a literal
    /// `~/` path, which is harmless — the realpaths the engine sends are
    /// always absolute, so an unexpanded `~/` simply never matches).
    pub fn from_env(home: Option<&Path>) -> Self {
        let raw = std::env::var(FLOX_SANDBOX_SENSITIVE_VAR).ok();
        let entries: Vec<String> = match raw.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(value) => value.split_whitespace().map(str::to_string).collect(),
            None => DEFAULT_SENSITIVE.iter().map(|s| s.to_string()).collect(),
        };
        Self::from_entries(entries, home)
    }

    /// Build the sensitive set from explicit entries, expanding `~/` against
    /// `home`. Exposed for tests and for callers that already hold the set.
    pub fn from_entries(entries: Vec<String>, home: Option<&Path>) -> Self {
        let patterns = entries
            .into_iter()
            .map(|entry| expand_home(&entry, home))
            .collect();
        Self { patterns }
    }

    /// True if `path` matches any sensitive glob.
    ///
    /// A pattern that fails to compile is skipped rather than treated as a
    /// match: a malformed override entry must not make every path look
    /// sensitive (which would block all directory suggestions), nor make a
    /// real secret look safe.
    pub fn is_sensitive(&self, path: &str) -> bool {
        self.patterns.iter().any(|pattern| {
            glob::Pattern::new(pattern)
                .ok()
                .is_some_and(|compiled| compiled.matches(path))
        })
    }

    /// The compiled patterns, for the `flox sandbox list` "Sensitive" readout.
    pub fn patterns(&self) -> &[String] {
        &self.patterns
    }
}

/// Expand a leading `~/` against `home`. Non-`~/` entries (including bare
/// `**/...` globs) pass through unchanged.
fn expand_home(entry: &str, home: Option<&Path>) -> String {
    match (entry.strip_prefix("~/"), home) {
        (Some(rest), Some(home)) => home.join(rest).to_string_lossy().into_owned(),
        _ => entry.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn home() -> PathBuf {
        PathBuf::from("/home/dev")
    }

    #[test]
    fn default_set_flags_credential_paths() {
        let set = SensitiveSet::from_entries(
            DEFAULT_SENSITIVE.iter().map(|s| s.to_string()).collect(),
            Some(&home()),
        );

        assert!(set.is_sensitive("/home/dev/.ssh/id_rsa"));
        assert!(set.is_sensitive("/home/dev/.aws/credentials"));
        assert!(set.is_sensitive("/home/dev/.gnupg/secring.gpg"));
        assert!(set.is_sensitive("/home/dev/.kube/config"));
        assert!(set.is_sensitive("/home/dev/.netrc"));
        assert!(set.is_sensitive("/home/dev/.config/gh/hosts.yml"));
    }

    #[test]
    fn env_files_are_sensitive_at_any_depth() {
        let set = SensitiveSet::from_entries(
            DEFAULT_SENSITIVE.iter().map(|s| s.to_string()).collect(),
            Some(&home()),
        );

        assert!(set.is_sensitive("/home/dev/project/.env"));
        assert!(set.is_sensitive("/srv/app/.env.production"));
    }

    #[test]
    fn the_grants_file_is_sensitive() {
        let set = SensitiveSet::from_entries(
            DEFAULT_SENSITIVE.iter().map(|s| s.to_string()).collect(),
            Some(&home()),
        );
        // The grants/journal path must never be foldable into a directory
        // grant — that would silence the tamper-evidence path.
        assert!(set.is_sensitive("/home/dev/project/.flox/cache/sandbox/grants.toml"));
    }

    #[test]
    fn routine_paths_are_not_sensitive() {
        let set = SensitiveSet::from_entries(
            DEFAULT_SENSITIVE.iter().map(|s| s.to_string()).collect(),
            Some(&home()),
        );

        assert!(!set.is_sensitive("/home/dev/.cargo/registry/index.crates.io/foo"));
        assert!(!set.is_sensitive("/home/dev/.config/gh-not-real/file"));
        assert!(!set.is_sensitive("/home/dev/project/src/main.rs"));
    }

    #[test]
    fn env_override_replaces_the_default_set() {
        // The override is parsed from a space-separated string; default
        // entries are not merged in, so a custom layout can shrink the set.
        let set = SensitiveSet::from_entries(vec!["~/.vault/**".to_string()], Some(&home()));
        assert!(set.is_sensitive("/home/dev/.vault/token"));
        // A default-set path is no longer sensitive under a replacing override.
        assert!(!set.is_sensitive("/home/dev/.aws/credentials"));
    }

    #[test]
    fn a_malformed_pattern_is_skipped_not_treated_as_a_match() {
        // An override with an invalid glob must not make every path sensitive.
        let set = SensitiveSet::from_entries(
            vec!["[unterminated".to_string(), "~/.aws/**".to_string()],
            Some(&home()),
        );
        assert!(!set.is_sensitive("/home/dev/project/src/main.rs"));
        assert!(set.is_sensitive("/home/dev/.aws/credentials"));
    }
}
