//! PII-safe redaction of `anyhow::Error` strings into stable category
//! fingerprints suitable for emission on `cli.command_completed` events.
//!
//! The redaction rules are intentionally over-aggressive — false positives
//! on the category side (e.g. a short stable token that happens to match
//! the digit pattern is wrongly redacted to `<n>`) are acceptable, but
//! leaked PII is not. Categories must be safe to ship to a public
//! warehouse and visible in any operator dashboard.
//!
//! ## Pipeline
//!
//! 1. Extract the top frame of the `anyhow::Error` chain via
//!    [`anyhow::Error::to_string`] (anyhow concatenates the chain
//!    onto one line; the leading frame is the most-specific message).
//! 2. Truncate the input to [`MAX_INPUT_LEN`] bytes so over-long
//!    error messages don't hit pathological regex runtime.
//! 3. Apply the redaction passes in the order documented below — order
//!    matters because earlier passes consume substrings the later
//!    passes would otherwise match.
//! 4. Return the result. The output may be longer than the input by
//!    a few bytes (sentinels are 5-8 chars).

use std::sync::LazyLock;

use regex::Regex;

/// Maximum length, in bytes, of the input string accepted by
/// [`categorize`]. Longer messages are truncated before redaction.
const MAX_INPUT_LEN: usize = 64;

// Pre-compiled regexes — cheap to share across the process via
// `LazyLock`, expensive to recompile per emission.

/// Path-shaped substrings — sequences of `[A-Za-z0-9/_.-]` of length
/// 8+ that contain at least one `/`. Catches absolute paths, relative
/// paths starting with `./`, dotfile paths, and most file extensions.
static PATH_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[A-Za-z0-9/_.\-]*/[A-Za-z0-9/_.\-]{7,}|[A-Za-z0-9/_.\-]{7,}/[A-Za-z0-9/_.\-]*")
        .expect("PATH_RE compiles")
});

/// Canonical 8-4-4-4-12 UUID shape, case-insensitive.
static UUID_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b")
        .expect("UUID_RE compiles")
});

/// Flake-reference URLs — `flake:` / `git+https:` / `github:` /
/// `nixpkgs:` / `path:` prefixes. Captures the scheme + everything up
/// to whitespace or end-of-string.
static FLAKE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:flake|git\+https|git\+ssh|github|gitlab|nixpkgs|path):[^\s]+")
        .expect("FLAKE_RE compiles")
});

/// Runs of 4+ consecutive ASCII digits not already absorbed by the
/// path / UUID / flake-ref passes. Catches PIDs, build numbers,
/// timestamps embedded in messages, etc.
static DIGITS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\d{4,}").expect("DIGITS_RE compiles"));

/// Redact an `anyhow::Error` into a PII-safe category fingerprint
/// suitable for emission on `cli.command_completed`. See the module
/// rustdoc for the full pipeline.
pub fn categorize(err: &anyhow::Error) -> String {
    let raw = err.to_string();
    let head: &str = if raw.len() > MAX_INPUT_LEN {
        // Slice on a UTF-8 boundary at-or-before MAX_INPUT_LEN so we
        // don't panic on multi-byte chars (a stray utf-8 sequence
        // at the boundary is rare in error messages but cheap to
        // defend against).
        let mut cut = MAX_INPUT_LEN;
        while cut > 0 && !raw.is_char_boundary(cut) {
            cut -= 1;
        }
        &raw[..cut]
    } else {
        &raw
    };
    redact(head)
}

/// The redaction passes, ordered. Exposed for unit testing without
/// requiring an `anyhow::Error` construction wrapper.
fn redact(input: &str) -> String {
    // Order matters: flake-refs first (they often contain `/` and
    // would otherwise be eaten by the path pass), then paths
    // (consume `/`-bearing substrings), then UUIDs (still match
    // their bounded shape independent of paths), then digit runs
    // (the last sweep over what survived).
    let s = FLAKE_RE.replace_all(input, "<flake-ref>").into_owned();
    let s = PATH_RE.replace_all(&s, "<path>").into_owned();
    let s = UUID_RE.replace_all(&s, "<uuid>").into_owned();
    let s = DIGITS_RE.replace_all(&s, "<n>").into_owned();
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pipeline-level test: an `anyhow::Error` constructed from a
    /// realistic top-level message is redacted as expected. Confirms
    /// `categorize` wires `.to_string` → truncate → redact correctly.
    #[test]
    fn categorize_redacts_path_in_anyhow_error() {
        let err = anyhow::anyhow!("could not open /Users/dsawyer/secret/config.toml");
        let cat = categorize(&err);
        assert!(!cat.contains("/Users"), "raw path leaked: {cat:?}");
        assert!(!cat.contains("dsawyer"), "username leaked: {cat:?}");
        assert!(cat.contains("<path>"), "path sentinel missing: {cat:?}");
    }

    /// The path pass replaces user-supplied filesystem paths with the
    /// `<path>` sentinel even when the surrounding error message is a
    /// stable English-language phrase.
    #[test]
    fn redact_replaces_path_with_sentinel() {
        let out = redact("could not open /etc/passwd");
        assert_eq!(out, "could not open <path>");
    }

    /// Dotfile paths and relative paths starting with `./` are
    /// caught.
    #[test]
    fn redact_catches_relative_dotfile_paths() {
        let out = redact("missing manifest at ./.flox/env/manifest.toml");
        assert!(!out.contains("manifest.toml"), "tail leaked: {out:?}");
        assert!(!out.contains(".flox"), "dotdir leaked: {out:?}");
        assert!(out.contains("<path>"), "no sentinel: {out:?}");
    }

    /// Canonical UUIDs are replaced with the `<uuid>` sentinel.
    #[test]
    fn redact_replaces_uuid_with_sentinel() {
        let out = redact("activation 459a165f-5221-4c27-b736-19d4b0d3a084 failed");
        assert_eq!(out, "activation <uuid> failed");
    }

    /// Case-insensitive UUID matching: uppercase hex still redacts.
    #[test]
    fn redact_catches_uppercase_uuid() {
        let out = redact("env 459A165F-5221-4C27-B736-19D4B0D3A084 broken");
        assert!(out.contains("<uuid>"), "no sentinel: {out:?}");
    }

    /// Flake references — `path:`, `github:`, `git+https:`, `nixpkgs:`
    /// — are replaced with the `<flake-ref>` sentinel.
    #[test]
    fn redact_replaces_flake_ref_with_sentinel() {
        let out = redact("could not build path:./local-flake#hello");
        assert!(
            !out.contains("path:./local-flake"),
            "raw flake leaked: {out:?}"
        );
        assert!(out.contains("<flake-ref>"), "no sentinel: {out:?}");

        let out = redact("could not resolve github:flox/flox#default");
        assert!(
            out.contains("<flake-ref>"),
            "github flake-ref not caught: {out:?}"
        );
    }

    /// Digit runs of 4+ chars are replaced with `<n>` so PIDs, build
    /// numbers, and embedded timestamps don't leak.
    #[test]
    fn redact_replaces_digit_run_with_sentinel() {
        let out = redact("subprocess 12345 exited with status 1");
        assert!(!out.contains("12345"), "PID leaked: {out:?}");
        assert!(out.contains("<n>"), "no digit sentinel: {out:?}");
        // The single-digit `1` should survive (status code).
        assert!(out.contains("status 1"), "single-digit eaten: {out:?}");
    }

    /// Truncation honors UTF-8 char boundaries so a multi-byte char
    /// straddling the 64-byte limit does not panic the redactor.
    #[test]
    fn categorize_truncates_long_input_at_utf8_boundary() {
        let prefix = "x".repeat(60);
        let err = anyhow::anyhow!("{prefix}ééééééé");
        let _ = categorize(&err); // panic on bad slice would fail the test
    }

    /// The MAX_INPUT_LEN truncation kicks in for long messages —
    /// the redacted output is bounded by the truncated input plus a
    /// few sentinel expansion bytes.
    #[test]
    fn categorize_truncates_overlong_input() {
        let long = "a".repeat(200);
        let err = anyhow::anyhow!("{long}");
        let cat = categorize(&err);
        assert!(
            cat.len() <= MAX_INPUT_LEN,
            "output too long: {} bytes",
            cat.len()
        );
    }

    /// Combined: a realistic edit-failure message redacts both the
    /// path and any embedded ids correctly.
    #[test]
    fn redact_combines_passes_on_realistic_message() {
        let raw = "failed to read /tmp/manifest-abc12345.toml for env myenv";
        let out = redact(raw);
        assert!(!out.contains("/tmp"), "path leaked: {out:?}");
        assert!(!out.contains("abc12345"), "id-shape leaked: {out:?}");
        assert!(out.contains("<path>"), "no path sentinel: {out:?}");
    }
}
