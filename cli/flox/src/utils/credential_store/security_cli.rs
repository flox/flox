//! The macOS Keychain backend, via the Apple-signed `security(1)` tool.
//!
//! The legacy Keychain records an "Always Allow" as a *trusted application*
//! on the item's ACL, keyed by the caller's code-signing designated
//! requirement. A nix-built `flox` is ad-hoc (linker) signed, so its
//! designated requirement is the `cdhash` of that exact build: every release
//! upgrade — and every developer rebuild — is a new, untrusted application,
//! and the prompt comes back (DEV-290). Going through `/usr/bin/security`
//! sidesteps this the same way `gh` does: the item is created *by*
//! `security`, so `security` is trusted to read it, and `security` is
//! Apple-signed, so that trust survives every flox upgrade.
//!
//! The secret never appears on a command line: writes go through
//! `security -i`, which reads its command from stdin, and reads return it on
//! stdout. Secrets are stored base64-encoded under [ENCODED_PREFIX] because
//! `security … -w` hex-encodes any value that is not plain printable text;
//! an item written by an earlier, native flox holds the raw token and is
//! read as-is.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use thiserror::Error;

/// Default location of the `security` tool on macOS.
const SECURITY_PROGRAM: &str = "/usr/bin/security";

/// Prefix marking a secret stored base64-encoded by this backend.
const ENCODED_PREFIX: &str = "flox-base64:";

/// `security -i` reads a command line into a 4096-byte buffer, so a line
/// (including its newline) may be at most 4095 bytes.
const INTERACTIVE_LINE_LIMIT: usize = 4095;

/// The tool's stderr when the addressed item does not exist
/// (`errSecItemNotFound`), for both find and delete.
const NOT_FOUND_MARKER: &str = "could not be found";

/// Errors from the `security` tool backend. No variant carries the secret:
/// `security` reports OSStatus messages on stderr, never item contents.
#[derive(Debug, Error)]
pub enum SecurityCliError {
    /// The `security` tool could not be started. The only variant callers
    /// branch on: a missing tool is the macOS "no backend" condition.
    #[error("could not run '{}'", program.display())]
    Spawn {
        program: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Any other failure — a non-"not found" tool error, an undecodable
    /// stored value, an unsafe identifier, or an oversized write. Nothing
    /// branches on the cause, so it is carried as a preformatted message.
    #[error("{0}")]
    Other(String),
}

/// A generic-password item addressed by service and account, manipulated
/// through the `security` tool.
#[derive(Debug, Clone)]
pub struct SecurityCli {
    program: PathBuf,
    service: String,
    account: String,
    /// When absent, use security's default keychain for writes and search
    /// list for reads and deletes. When present, scope all operations to it.
    keychain: Option<PathBuf>,
}

impl SecurityCli {
    /// Address the item `service`/`account` through `/usr/bin/security`.
    /// An explicit keychain scopes all operations to that file.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn new(
        service: impl Into<String>,
        account: impl Into<String>,
        keychain: Option<PathBuf>,
    ) -> Self {
        Self {
            keychain,
            ..Self::with_program(SECURITY_PROGRAM, service, account)
        }
    }

    /// Address the item through a specific `security` executable (tests
    /// substitute a recording fake).
    pub fn with_program(
        program: impl Into<PathBuf>,
        service: impl Into<String>,
        account: impl Into<String>,
    ) -> Self {
        Self {
            program: program.into(),
            service: service.into(),
            account: account.into(),
            keychain: None,
        }
    }

    /// Read the secret, `None` when no such item exists.
    pub fn get(&self) -> Result<Option<String>, SecurityCliError> {
        let output = self.run(
            "find-generic-password",
            &[
                "find-generic-password",
                "-s",
                &self.service,
                "-a",
                &self.account,
                "-w",
            ],
            None,
            true,
        )?;
        let Some(output) = output else {
            return Ok(None);
        };
        let stored = String::from_utf8(output.stdout).map_err(|e| {
            SecurityCliError::Other(format!("the stored credential is not valid UTF-8: {e}"))
        })?;
        let stored = stored.trim_end_matches(['\r', '\n']);
        match stored.strip_prefix(ENCODED_PREFIX) {
            Some(encoded) => {
                let bytes = BASE64.decode(encoded).map_err(|e| {
                    SecurityCliError::Other(format!(
                        "the stored credential is not valid base64: {e}"
                    ))
                })?;
                String::from_utf8(bytes).map(Some).map_err(|e| {
                    SecurityCliError::Other(format!(
                        "the stored credential is not valid UTF-8: {e}"
                    ))
                })
            },
            // An item written by an earlier, native flox holds the raw token.
            None => Ok(Some(stored.to_string())),
        }
    }

    /// Store `secret`, replacing any existing item.
    ///
    /// The item is deleted and recreated rather than updated in place: the
    /// application that *creates* an item is the one the Keychain trusts to
    /// read it back, so recreating it through `security` hands ownership to
    /// the Apple-signed tool instead of leaving it under a previous flox
    /// build's ACL. A failed delete is not fatal — `-U` still updates the
    /// existing item, and the add reports its own failure.
    pub fn set(&self, secret: &str) -> Result<(), SecurityCliError> {
        self.check_identifiers()?;
        let encoded = format!("{ENCODED_PREFIX}{}", BASE64.encode(secret));
        // The secret goes to `security -i` on stdin, never on the argv; the
        // interactive reader accepts one line of at most MAX_LINE_LEN bytes.
        let mut line = format!(
            "add-generic-password -U -s {} -a {} -w {encoded}",
            self.service, self.account
        );
        if let Some(keychain) = &self.keychain {
            let path = keychain
                .to_str()
                .filter(|path| !path.is_empty() && !path.contains(['\0', '\r', '\n']))
                .ok_or_else(|| {
                    SecurityCliError::Other(
                        "the keychain path cannot be represented in a security command line"
                            .to_string(),
                    )
                })?;
            // security's interactive parser strips quotes and treats a
            // backslash as escaping the next character, including in quotes.
            line.push_str(" \"");
            line.push_str(&path.replace('\\', "\\\\").replace('"', "\\\""));
            line.push('"');
        }
        line.push('\n');
        if line.len() > INTERACTIVE_LINE_LIMIT {
            return Err(SecurityCliError::Other(format!(
                "the credential is too large to store ({} of at most {INTERACTIVE_LINE_LIMIT} bytes)",
                line.len()
            )));
        }

        if let Err(e) = self.remove() {
            tracing::debug!(error = %e, "could not delete the existing keychain item before recreating it");
        }

        self.run(
            "add-generic-password",
            &["-i"],
            Some(line.as_bytes()),
            false,
        )?;
        Ok(())
    }

    /// Delete the item. Idempotent: a missing item is not a failure.
    pub fn remove(&self) -> Result<(), SecurityCliError> {
        self.run(
            "delete-generic-password",
            &[
                "delete-generic-password",
                "-s",
                &self.service,
                "-a",
                &self.account,
            ],
            None,
            true,
        )?;
        Ok(())
    }

    /// Reject identifiers that the `security -i` line parser could misread.
    ///
    /// Only [Self::set] needs this. It builds a command line for `security
    /// -i`, whose parser splits on whitespace and treats quotes and
    /// backslashes specially; [Self::get] and [Self::remove] pass the
    /// identifiers as argv entries, which no parser touches. The service is a
    /// fixed name, but the account is a FloxHub URL, and `Url` percent-encodes
    /// neither `'` in a path nor `\` in a fragment — so rather than quote,
    /// refuse what the parser could misread.
    fn check_identifiers(&self) -> Result<(), SecurityCliError> {
        fn is_safe(s: &str) -> bool {
            !s.is_empty()
                && !s
                    .chars()
                    .any(|c| c.is_whitespace() || matches!(c, '"' | '\'' | '\\'))
        }
        if !is_safe(&self.service) {
            return Err(SecurityCliError::Other(
                "the keyring service contains whitespace or quoting characters".to_string(),
            ));
        }
        if !is_safe(&self.account) {
            return Err(SecurityCliError::Other(
                "the keyring account contains whitespace or quoting characters".to_string(),
            ));
        }
        Ok(())
    }

    /// Run `security` with `args`, feeding `stdin` when given.
    ///
    /// Returns `Ok(Some(output))` on success. When `missing_is_ok`, the tool
    /// reporting that the item does not exist (the same "not found" condition
    /// for find and delete) is `Ok(None)` rather than a failure.
    fn run(
        &self,
        subcommand: &'static str,
        args: &[&str],
        stdin: Option<&[u8]>,
        missing_is_ok: bool,
    ) -> Result<Option<std::process::Output>, SecurityCliError> {
        let mut command = Command::new(&self.program);
        command
            .args(args)
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if stdin.is_none() {
            command.args(&self.keychain);
        }
        let mut child = command.spawn().map_err(|source| SecurityCliError::Spawn {
            program: self.program.clone(),
            source,
        })?;
        if let Some(input) = stdin {
            use std::io::Write;
            // A write failure surfaces as the tool's own exit status below.
            if let Some(mut pipe) = child.stdin.take() {
                let _ = pipe.write_all(input);
            }
        }
        let output = child
            .wait_with_output()
            .map_err(|source| SecurityCliError::Spawn {
                program: self.program.clone(),
                source,
            })?;
        if output.status.success() {
            return Ok(Some(output));
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if missing_is_ok && stderr.contains(NOT_FOUND_MARKER) {
            return Ok(None);
        }
        Err(SecurityCliError::Other(format!(
            "'security {subcommand}' failed ({}): {stderr}",
            output.status
        )))
    }
}

/// These tests need securityd and run in the macOS CI impure-test job, outside
/// the Nix build sandbox. Each operation explicitly addresses its own keychain.
#[cfg(all(test, feature = "impure-unit-tests"))]
mod macos_tests {
    use std::process::{Command, Stdio};

    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    use super::{SECURITY_PROGRAM, SecurityCli};

    struct TestKeychain {
        _dir: TempDir,
        cli: SecurityCli,
    }

    impl TestKeychain {
        fn new() -> Self {
            let dir = tempfile::Builder::new()
                .prefix("flox security ")
                .tempdir()
                .unwrap();
            // Exercise the same path quoting used for any explicit keychain.
            let keychain = dir.path().join("test 'quoted' \"keychain\"\\.keychain-db");
            let cli = SecurityCli::new("dev.flox.test", "https://hub.flox.test/", Some(keychain));
            // Construct the cleanup guard before creating the keychain so it
            // is also deleted if subsequent setup or an assertion panics.
            let fixture = Self { _dir: dir, cli };
            fixture.run(&["create-keychain", "-p", "test-password"]);
            fixture.run(&["unlock-keychain", "-p", "test-password"]);
            fixture.run(&["set-keychain-settings"]);
            fixture
        }

        fn run(&self, args: &[&str]) {
            let output = Command::new(SECURITY_PROGRAM)
                .args(args)
                .arg(self.cli.keychain.as_ref().unwrap())
                .stdin(Stdio::null())
                .output()
                .unwrap();
            assert!(output.status.success(), "security {args:?}: {output:?}");
        }

        fn store_raw(&self, secret: &str) {
            // Only synthetic test values go on argv. Bypass the backend's
            // encoding to reproduce the format stored by native flox.
            self.run(&[
                "add-generic-password",
                "-U",
                "-s",
                &self.cli.service,
                "-a",
                &self.cli.account,
                "-w",
                secret,
            ]);
        }
    }

    impl Drop for TestKeychain {
        fn drop(&mut self) {
            // delete-keychain also removes this keychain from the search
            // list; dropping TempDir alone would leave a dangling entry.
            let result = Command::new(SECURITY_PROGRAM)
                .arg("delete-keychain")
                .arg(self.cli.keychain.as_ref().unwrap())
                .stdin(Stdio::null())
                .output();
            if !std::thread::panicking() {
                let output = result.unwrap();
                assert!(output.status.success(), "delete-keychain: {output:?}");
            }
        }
    }

    #[test]
    #[cfg_attr(not(target_os = "macos"), ignore = "requires the macOS Keychain")]
    fn stores_replaces_and_removes_credentials() {
        let keychain = TestKeychain::new();
        let cli = &keychain.cli;

        assert_eq!(cli.get().unwrap(), None);
        cli.remove().unwrap();

        // Each write replaces an existing item after the first. Include
        // values that security would hex-encode without our base64 envelope.
        for secret in ["token", "tok en\n\r\t\"'\\", "token-🦊", "", "trailing\n"] {
            cli.set(secret).unwrap();
            assert_eq!(cli.get().unwrap(), Some(secret.to_string()));
        }

        cli.remove().unwrap();
        assert_eq!(cli.get().unwrap(), None);
        cli.remove().unwrap();
    }

    #[test]
    #[cfg_attr(not(target_os = "macos"), ignore = "requires the macOS Keychain")]
    fn reads_and_replaces_a_legacy_raw_token() {
        let keychain = TestKeychain::new();
        keychain.store_raw("legacy.raw.token");

        assert_eq!(keychain.cli.get().unwrap(), Some("legacy.raw.token".into()));

        keychain.cli.set("replacement").unwrap();
        assert_eq!(keychain.cli.get().unwrap(), Some("replacement".into()));
    }

    #[test]
    #[cfg_attr(not(target_os = "macos"), ignore = "requires the macOS Keychain")]
    fn rejects_invalid_encoded_credentials() {
        let keychain = TestKeychain::new();
        for (stored, expected) in [
            ("flox-base64:!", "not valid base64"),
            ("flox-base64:/w==", "not valid UTF-8"),
        ] {
            keychain.store_raw(stored);
            let error = keychain.cli.get().unwrap_err();
            assert!(error.to_string().contains(expected), "{error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    use super::*;

    const SERVICE: &str = "dev.flox.flox";
    const ACCOUNT: &str = "https://hub.flox.dev/";

    /// Field separator the fake uses to record one argv per line.
    const ARG_SEP: char = '\u{1f}';

    /// Records process arguments and stdin, with optional synthetic failures
    /// for testing our process handling. Keychain behavior uses the real tool
    /// in `macos_tests`.
    struct FakeSecurity {
        dir: TempDir,
    }

    impl FakeSecurity {
        fn new() -> Self {
            let dir = TempDir::new().unwrap();
            let bash = std::env::var_os("PATH")
                .and_then(|path| {
                    std::env::split_paths(&path)
                        .map(|p| p.join("bash"))
                        .find(|p| p.is_file())
                })
                .expect("bash on PATH");
            let script = format!(
                r#"#!{bash}
d="$(dirname "$0")"
{{ for a in "$@"; do printf '%s\x1f' "$a"; done; printf '\n'; }} >> "$d/calls"
if [ "$1" = "-i" ]; then cat >> "$d/stdin"; fi
[ -f "$d/stderr" ] && cat "$d/stderr" >&2
if [ -f "$d/status" ]; then exit "$(cat "$d/status")"; fi
exit 0
"#,
                bash = bash.display()
            );
            let program = dir.path().join("security");
            std::fs::write(&program, script).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            Self { dir }
        }

        fn program(&self) -> PathBuf {
            self.dir.path().join("security")
        }

        fn cli(&self) -> SecurityCli {
            SecurityCli::with_program(self.program(), SERVICE, ACCOUNT)
        }

        fn fail(&self, stderr: &str) {
            std::fs::write(self.dir.path().join("stderr"), stderr).unwrap();
            std::fs::write(self.dir.path().join("status"), "1").unwrap();
        }

        fn calls(&self) -> Vec<Vec<String>> {
            let Ok(recorded) = std::fs::read_to_string(self.dir.path().join("calls")) else {
                return Vec::new();
            };
            recorded
                .lines()
                .map(|line| {
                    line.split(ARG_SEP)
                        .filter(|arg| !arg.is_empty())
                        .map(str::to_string)
                        .collect()
                })
                .collect()
        }

        fn stdin(&self) -> String {
            std::fs::read_to_string(self.dir.path().join("stdin")).unwrap_or_default()
        }
    }

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn get_reports_the_failed_subcommand_and_stderr() {
        let fake = FakeSecurity::new();
        fake.fail("synthetic read failure\n");

        let error = fake.cli().get().unwrap_err();

        let message = error.to_string();
        assert!(
            message.contains("find-generic-password") && message.contains("synthetic read failure"),
            "message should name the subcommand and carry the tool's stderr: {message}"
        );
    }

    #[test]
    fn set_sends_the_secret_on_stdin_after_attempting_delete() {
        let fake = FakeSecurity::new();
        let encoded = BASE64.encode("s3cret");

        fake.cli().set("s3cret").unwrap();

        assert_eq!(fake.calls(), vec![
            args(&["delete-generic-password", "-s", SERVICE, "-a", ACCOUNT]),
            args(&["-i"]),
        ]);
        // The argv above carries no secret: the add is driven entirely by this
        // stdin line, so the token never reaches another user's `ps` output.
        assert_eq!(
            fake.stdin(),
            format!(
                "add-generic-password -U -s {SERVICE} -a {ACCOUNT} -w {ENCODED_PREFIX}{encoded}\n"
            )
        );
    }

    #[test]
    fn set_attempts_write_after_delete_failure_and_reports_write_failure() {
        let fake = FakeSecurity::new();
        fake.fail("synthetic process failure\n");
        // Our recovery policy attempts the write after any delete failure
        // and reports the write's error if that also fails.
        let error = fake.cli().set("s3cret").unwrap_err();

        assert!(
            error.to_string().contains("add-generic-password")
                && error.to_string().contains("synthetic process failure"),
            "unexpected error: {error:?}"
        );
        assert_eq!(fake.calls().len(), 2, "add must run after a failed delete");
    }

    #[test]
    fn remove_reports_the_failed_subcommand_and_stderr() {
        let fake = FakeSecurity::new();
        fake.fail("synthetic delete failure\n");

        let error = fake.cli().remove().unwrap_err();

        assert!(
            error.to_string().contains("delete-generic-password")
                && error.to_string().contains("synthetic delete failure"),
            "unexpected error: {error:?}"
        );
    }

    #[test]
    fn set_rejects_identifiers_with_whitespace_or_quotes_before_spawning() {
        let fake = FakeSecurity::new();
        let cli = SecurityCli::with_program(fake.program(), "dev flox", ACCOUNT);

        let error = cli.set("s3cret").unwrap_err();

        assert!(
            error.to_string().contains("keyring service"),
            "unexpected error: {error:?}"
        );
        assert!(fake.calls().is_empty(), "nothing should have been spawned");
    }

    #[test]
    fn set_rejects_unrepresentable_keychain_paths_before_spawning() {
        let fake = FakeSecurity::new();
        for path in ["", "/tmp/key\nchain", "/tmp/key\rchain", "/tmp/key\0chain"] {
            let cli = SecurityCli {
                keychain: Some(PathBuf::from(path)),
                ..fake.cli()
            };

            let error = cli.set("s3cret").unwrap_err();

            assert!(error.to_string().contains("keychain path"), "{error}");
            assert!(fake.calls().is_empty(), "nothing should have been spawned");
        }
    }

    #[test]
    fn a_secret_that_overflows_the_interactive_line_is_rejected_before_spawning() {
        let fake = FakeSecurity::new();
        // `security -i` reads at most 4095 bytes per line; base64 grows the
        // secret by a third, so 3500 raw bytes overflow it.
        let error = fake.cli().set(&"x".repeat(3500)).unwrap_err();

        assert!(
            error.to_string().contains("too large to store"),
            "unexpected error: {error:?}"
        );
        assert!(fake.calls().is_empty(), "nothing should have been spawned");
    }

    #[test]
    fn a_missing_security_tool_is_a_spawn_error() {
        let cli = SecurityCli::with_program("/nonexistent/security", SERVICE, ACCOUNT);

        let error = cli.get().unwrap_err();

        assert!(
            matches!(error, SecurityCliError::Spawn { .. }),
            "unexpected error: {error:?}"
        );
    }
}
