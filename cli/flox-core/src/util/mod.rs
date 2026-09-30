pub mod message;

use std::collections::HashMap;
use std::env;
use std::ffi::OsString;
use std::process::Command;
use std::sync::OnceLock;

/// The CA bundle flox fills in for Nix software, compiled in at build time.
pub const NIXPKGS_CACERT_BUNDLE_CRT: &str = env!("NIXPKGS_CACERT_BUNDLE_CRT");

/// The locale variable this platform's C library reads, and the build-time
/// default flox fills in for it.
#[cfg(target_os = "macos")]
pub const LOCALE: (&str, &str) = ("PATH_LOCALE", env!("PATH_LOCALE"));
#[cfg(target_os = "linux")]
pub const LOCALE: (&str, &str) = ("LOCALE_ARCHIVE", env!("LOCALE_ARCHIVE"));

/// Environment variables to fill in for Nix-built executables.
///
/// This is the one statement of the rule. The Flox installer's `/usr/bin`
/// wrappers (flox-installers, `pkgs/env-wrapper`, from flox-installers#536)
/// and the activation tests (`cli/tests/activate.bats`,
/// `cli/tests/deactivate.bats`) follow it and point here.
///
/// Nixpkgs itself is broken in that the packages it creates depend upon a
/// variety of environment variables at runtime. On NixOS these are
/// convenient to set on a system-wide basis but that essentially masks the
/// problem, and it's not uncommon to see Nix packages trip over the
/// absence of environment variables when invoked on other Linux
/// distributions. flox fills in Nix-provided defaults for the ones we
/// know to be required, but only where the user has not set them: a
/// value the user chose is theirs. The build-time values bundle the files
/// they point at in flox's package closure.
///
/// The rule. "Set" means present in the environment, even if empty.
///
/// - `NIX_SSL_CERT_FILE` set to anything but this build's bundle: nothing.
///   It is the explicit user override, for an `SSL_CERT_FILE` in a format
///   the Nix TLS stack cannot read, or for testing.
/// - `NIX_SSL_CERT_FILE` unset and `SSL_CERT_FILE` set: nothing.
///   `SSL_CERT_FILE` is the system-wide contract that OpenSSL and most
///   other TLS stacks honor, Nix-built or not (runtimes with a trust store
///   of their own, such as Node, Python with certifi, Java, or rustls with
///   webpki-roots, do not), and Nix reads it when `NIX_SSL_CERT_FILE` is
///   unset. `SSL_CERT_DIR` plays no part in the rule: set on its own it
///   leaves `SSL_CERT_FILE` unset, so the default applies.
/// - Neither set: `NIX_SSL_CERT_FILE` is pointed at the bundled CA
///   certificates, for Nix software alone. `SSL_CERT_FILE` is never
///   written, because everything flox spawns and every activated shell
///   inherits these values, and non-Nix software must not be handed a
///   value it did not expect.
/// - `NIX_SSL_CERT_FILE` equal to this build's bundle: re-applied,
///   whatever `SSL_CERT_FILE` holds. This is the own-default clause, below.
/// - The locale variable (`LOCALE`): filled in when unset or equal to this
///   build's default; a user's value, empty included, is left alone.
///
/// The own-default clause exists because this function returns an export
/// map for another process rather than changing the environment in place.
/// flox applies the map to its own process at startup, then spawns
/// `flox-activations`, which computes the map again to decide what the
/// activation exports. Without the clause the child would take the
/// parent's default for the user's value and export nothing, and an
/// in-place activation would leave the shell without it. The installer's
/// wrapper needs no such clause: it changes its own environment and execs,
/// so its value simply persists. The clause also holds when
/// `SSL_CERT_FILE` has appeared since the parent decided. On Linux the TLS
/// library flox uses probes the system trust store on first use and writes
/// `SSL_CERT_FILE` and `SSL_CERT_DIR` into the process; flox records the
/// two before that can happen and hands the record to everything it
/// spawns (see [`capture_startup_tls_env`]), so the child sees them only
/// as the user left them. One consequence: a user who exports
/// `SSL_CERT_FILE` inside an activated shell still gets
/// `NIX_SSL_CERT_FILE` re-exported by nested activations, so to redirect
/// Nix-built software there they set `NIX_SSL_CERT_FILE` too.
///
/// Requirement: the bundle and locale compiled into flox, into
/// `flox-activations` and into the installer's wrappers must be the same
/// store paths, since the clause recognizes flox's own default by value.
/// That holds by construction: the installer builds from flox's own
/// package set (`passthru.pkgsFor`), and flox-cli and flox-activations take
/// `cacert` from that same set. The locale value is the Linux package from
/// that set; on Darwin it is the `PATH_LOCALE` the stdenv hook exports at
/// build time, which resolves to the same store path today but is not
/// pinned by name (flox/flox#4749). A flox not built
/// from that set, run through the wrappers, treats the wrapper's value as
/// the user's and exports nothing; subshells still inherit it from the
/// process environment, but an in-place activation does not export it.
///
/// `flox build` does not go through `flox-activations`; a local build
/// inherits these from the flox process instead.
pub fn default_nix_env_vars() -> HashMap<&'static str, String> {
    let mut env_map: HashMap<&str, String> = HashMap::new();

    let apply_cacert = match env::var_os("NIX_SSL_CERT_FILE") {
        Some(value) => value == NIXPKGS_CACERT_BUNDLE_CRT,
        None => env::var_os("SSL_CERT_FILE").is_none(),
    };
    if apply_cacert {
        env_map.insert("NIX_SSL_CERT_FILE", NIXPKGS_CACERT_BUNDLE_CRT.to_string());
    }

    let (locale_var, locale_default) = LOCALE;
    if env::var_os(locale_var).is_none_or(|value| value == locale_default) {
        env_map.insert(locale_var, locale_default.to_string());
    }

    env_map
}

/// The variables native-tls's OpenSSL backend writes into the process on
/// Linux when the first TLS client is built (through `openssl-probe`), with
/// the system trust store's paths. flox never writes them itself, and what
/// it spawns must not inherit them either.
const PROBED_TLS_VARS: [&str; 2] = ["SSL_CERT_FILE", "SSL_CERT_DIR"];

static STARTUP_TLS_ENV: OnceLock<Vec<(&'static str, Option<OsString>)>> = OnceLock::new();

/// Record `SSL_CERT_FILE` and `SSL_CERT_DIR` as they were when flox
/// started. Call this first thing in `main`, before Sentry, any HTTP
/// client or the async runtime, so that the record predates the probe.
pub fn capture_startup_tls_env() {
    let _ = STARTUP_TLS_ENV.set(
        PROBED_TLS_VARS
            .iter()
            .map(|&name| (name, env::var_os(name)))
            .collect(),
    );
}

/// Give `command` the startup state of `SSL_CERT_FILE` and `SSL_CERT_DIR`,
/// so that what flox and the executive spawn (activations, services and
/// their hooks, builds, the develop shell, `flox run` and extensions)
/// does not inherit values the TLS library wrote into this process since.
/// Apply it before any environment the user controls, so a hook's export
/// of either variable still wins. Does nothing if
/// [`capture_startup_tls_env`] was never called.
pub fn restore_startup_tls_env(command: &mut Command) {
    let Some(startup) = STARTUP_TLS_ENV.get() else {
        return;
    };
    for (name, value) in startup {
        match value {
            Some(value) => {
                command.env(name, value);
            },
            None => {
                command.env_remove(name);
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUNDLE: &str = NIXPKGS_CACERT_BUNDLE_CRT;

    /// Run `default_nix_env_vars` with exactly the given certificate and
    /// locale variables present (`None` clears one), so the ambient
    /// environment of the test runner does not leak in.
    fn with_env(
        nix_ssl_cert_file: Option<&str>,
        ssl_cert_file: Option<&str>,
        locale: Option<&str>,
    ) -> HashMap<&'static str, String> {
        temp_env::with_vars(
            [
                ("NIX_SSL_CERT_FILE", nix_ssl_cert_file),
                ("SSL_CERT_FILE", ssl_cert_file),
                (LOCALE.0, locale),
            ],
            default_nix_env_vars,
        )
    }

    fn map(pairs: &[(&'static str, &str)]) -> HashMap<&'static str, String> {
        pairs.iter().map(|(k, v)| (*k, v.to_string())).collect()
    }

    /// One row of the rule: a label, the three inputs, and the whole map
    /// expected back.
    struct Case(
        &'static str,
        Option<&'static str>,
        Option<&'static str>,
        Option<&'static str>,
        HashMap<&'static str, String>,
    );

    /// Every row of the rule, as whole maps. "Set" includes empty.
    #[test]
    fn rule_matrix() {
        let (lv, ld) = LOCALE;
        let both = map(&[("NIX_SSL_CERT_FILE", BUNDLE), (lv, ld)]);
        let locale_only = map(&[(lv, ld)]);
        let cacert_only = map(&[("NIX_SSL_CERT_FILE", BUNDLE)]);
        let cases = [
            Case("neither set", None, None, None, both.clone()),
            Case(
                "user SSL_CERT_FILE",
                None,
                Some("/user/certs.pem"),
                None,
                locale_only.clone(),
            ),
            Case(
                "empty SSL_CERT_FILE counts as set",
                None,
                Some(""),
                None,
                locale_only.clone(),
            ),
            Case(
                "SSL_CERT_FILE equal to the bundle is still the user's",
                None,
                Some(BUNDLE),
                None,
                locale_only.clone(),
            ),
            Case(
                "user NIX_SSL_CERT_FILE",
                Some("/user/nix-certs.pem"),
                None,
                None,
                locale_only.clone(),
            ),
            Case(
                "empty NIX_SSL_CERT_FILE counts as set",
                Some(""),
                None,
                None,
                locale_only.clone(),
            ),
            Case(
                "user NIX_SSL_CERT_FILE beside user SSL_CERT_FILE",
                Some("/user/nix-certs.pem"),
                Some("/user/certs.pem"),
                None,
                locale_only.clone(),
            ),
            Case(
                "own default, SSL_CERT_FILE unset",
                Some(BUNDLE),
                None,
                None,
                both.clone(),
            ),
            Case(
                "own default, SSL_CERT_FILE empty",
                Some(BUNDLE),
                Some(""),
                None,
                both.clone(),
            ),
            Case(
                "own default, SSL_CERT_FILE appeared since (probe or user)",
                Some(BUNDLE),
                Some("/usr/lib/ssl/cert.pem"),
                None,
                both.clone(),
            ),
            Case(
                "own default, SSL_CERT_FILE equal to the bundle",
                Some(BUNDLE),
                Some(BUNDLE),
                None,
                both.clone(),
            ),
            Case(
                "user locale",
                None,
                None,
                Some("/user/locale"),
                cacert_only.clone(),
            ),
            Case(
                "empty locale counts as set",
                None,
                None,
                Some(""),
                cacert_only.clone(),
            ),
            Case("own locale default", None, None, Some(ld), both.clone()),
        ];
        for Case(label, nix, ssl, locale, expected) in cases {
            assert_eq!(with_env(nix, ssl, locale), expected, "{label}");
        }
    }

    /// The startup record restores what the probe may have written, and
    /// only that: an absent variable is removed, a present one (empty
    /// included) is set back to its startup value, and everything else on
    /// the command is left alone. The record is process-wide and set once,
    /// so this is the only test in the binary that calls capture.
    #[test]
    fn restore_gives_children_the_startup_tls_env() {
        temp_env::with_vars(
            [("SSL_CERT_FILE", None::<&str>), ("SSL_CERT_DIR", Some(""))],
            capture_startup_tls_env,
        );
        // As if the probe ran since.
        let mut command = Command::new("true");
        command.env("SSL_CERT_FILE", "/usr/lib/ssl/cert.pem");
        command.env("SSL_CERT_DIR", "/usr/lib/ssl/certs");
        command.env("NIX_SSL_CERT_FILE", BUNDLE);
        restore_startup_tls_env(&mut command);
        let envs: HashMap<_, _> = command
            .get_envs()
            .map(|(k, v)| (k.to_os_string(), v.map(|v| v.to_os_string())))
            .collect();
        let expected: HashMap<OsString, Option<OsString>> = HashMap::from([
            (OsString::from("SSL_CERT_FILE"), None),
            (OsString::from("SSL_CERT_DIR"), Some(OsString::from(""))),
            (
                OsString::from("NIX_SSL_CERT_FILE"),
                Some(OsString::from(BUNDLE)),
            ),
        ]);
        assert_eq!(envs, expected);
    }
}
