//! PII-safe redaction of `anyhow::Error` strings into stable category
//! fingerprints suitable for emission on `cli.command_completed` events.
//!
//! Categories are emitted on every failed CLI invocation and must not
//! contain user-supplied filesystem paths, flake references, UUIDs,
//! URLs, email addresses, IP addresses, large opaque tokens, or
//! numeric identifiers. The redactor substitutes stable sentinels
//! (`<path>`, `<url>`, `<email>`, `<ip>`, `<uuid>`, `<hex>`, `<token>`,
//! `<n>`) so those values are never transmitted verbatim. False
//! positives on the category side (e.g. a short stable token that
//! happens to match the digit pattern is wrongly redacted to `<n>`)
//! are acceptable; leaks are not.
//!
//! ## Pipeline
//!
//! 1. Extract the top frame of the `anyhow::Error` chain via
//!    [`anyhow::Error::to_string`] (anyhow concatenates the chain
//!    onto one line; the leading frame is the most-specific message).
//! 2. Bound the input at [`MAX_INPUT_BYTES`] on a UTF-8 char boundary
//!    so a pathological multi-megabyte error message can't pin the
//!    regex passes for arbitrarily long.
//! 3. Apply the redaction passes in the order documented in
//!    [`redact`] — order matters because earlier passes consume
//!    substrings later passes would otherwise match.
//! 4. Bound the output at [`MAX_OUTPUT_BYTES`] on a UTF-8 char
//!    boundary — the wire contract. This truncation runs AFTER
//!    redaction so a sensitive value at the end of a long error
//!    string is redacted before being truncated, eliminating the
//!    partial-token leak class.
//!
//! ## Known limitations
//!
//! - Bare unstructured short identifiers in English-language error
//!   messages (e.g. `error: alice not authorized` where `alice` is a
//!   ~5-7 char username with no surrounding `/` or `@`) are not
//!   reliably distinguishable from English words without an allowlist
//!   and may slip through. Most flox errors that carry usernames
//!   carry them in URL, path, or `@email`-shaped contexts that the
//!   passes catch.
//! - Backslash-separated Windows paths are caught by [`PATH_RE`], but
//!   flox is not built or tested against Windows; the catch is a
//!   defence-in-depth gesture rather than a tested guarantee.

use std::sync::LazyLock;

use regex::Regex;

/// Maximum length, in bytes, of the input string accepted by
/// [`categorize`]. Long enough to cover any realistic anyhow top-
/// frame; short enough to bound regex compute even on adversarial
/// input.
const MAX_INPUT_BYTES: usize = 4096;

/// Maximum length, in bytes, of the output string [`categorize`]
/// produces. Wider than the spec's 64-byte input bound because
/// redaction sentinels expand the byte count slightly — the wire
/// contract is the bound on the *redacted* output, not on the raw
/// input.
const MAX_OUTPUT_BYTES: usize = 96;

// Pre-compiled regexes — cheap to share across the process via
// `LazyLock`, expensive to recompile per emission. The Rust `regex`
// crate guarantees linear-time matching, so passes over large
// inputs cannot trigger catastrophic backtracking.

/// Any-scheme URL (`https://`, `http://`, `ssh://`, `s3://`, `file://`,
/// `git+https://`, etc.). Captures the scheme + everything up to
/// whitespace or end-of-string. Placed first so a URL embedded in an
/// error message is fully replaced before the path / token passes
/// would split it into fragments.
static URL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b[a-z][a-z0-9+.\-]*://\S+").expect("URL_RE compiles"));

/// Flake-style references without `://` separators — `flake:foo`,
/// `github:flox/flox`, `nixpkgs:hello`, `path:./local-flake`,
/// `gitlab:org/repo`, `git:repo.git`. The `://` form is covered by
/// [`URL_RE`] above.
static FLAKE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:flake|github|gitlab|nixpkgs|path|git):[^\s/]\S*").expect("FLAKE_RE compiles")
});

/// Email addresses — `local-part@domain.tld`. Includes the trailing
/// label so subdomains and TLDs are consumed wholly.
static EMAIL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}\b").expect("EMAIL_RE compiles")
});

/// IPv4 dotted-quad addresses. Caught before [`DIGITS_RE`] because
/// IPv4 octets are 1-3 digits and would otherwise slip through the
/// 4+-digit threshold.
static IPV4_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(?:\d{1,3}\.){3}\d{1,3}\b").expect("IPV4_RE compiles"));

/// IPv6 addresses — a leading non-empty hex segment followed by
/// two or more `:hex?` groups. Catches both compressed forms
/// (`fe80::abcd`, `2001:db8::1`) and most uncompressed forms.
/// Fully-compressed loopback (`::1`) is not caught — it is not
/// PII.
static IPV6_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b[0-9a-fA-F]{1,4}(?::[0-9a-fA-F]{0,4}){2,7}").expect("IPV6_RE compiles")
});

/// Canonical 8-4-4-4-12 UUID shape, case-insensitive. Caught before
/// [`HEX_RE`] so canonical UUIDs land on the more-specific sentinel.
static UUID_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b")
        .expect("UUID_RE compiles")
});

/// Path-shaped substrings — sequences containing at least one `/`
/// or `\` separator with non-trivial segment content on either
/// side. Catches absolute paths, relative paths starting with `./`,
/// dotfile paths, Windows backslash paths, and most file
/// extensions. Lower threshold than the original spec so short
/// well-known paths (`/etc/foo`, `/var/lo`) don't slip through the
/// post-truncation tail.
static PATH_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?:[A-Za-z]:[\\/])?[A-Za-z0-9_.\-]*[\\/][A-Za-z0-9_.\-]+(?:[\\/][A-Za-z0-9_.\-]+)*",
    )
    .expect("PATH_RE compiles")
});

/// Long hexadecimal runs (8+ chars), with optional `0x` prefix.
/// Catches no-dash UUIDs (32 hex chars), SHA-1 / SHA-256 fingerprints,
/// MAC-address-shaped runs, and hex memory addresses. Placed after
/// [`UUID_RE`] so canonical UUIDs still get their specific sentinel.
static HEX_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(?:0x)?[0-9a-fA-F]{8,}\b").expect("HEX_RE compiles"));

/// Opaque token-shaped runs (20+ chars of `[A-Za-z0-9_+/=-]`).
/// Catches API keys, JWTs, base64-encoded blobs, and other opaque
/// secret-shaped strings. The 20-char threshold avoids redacting
/// most English-language words appearing in error messages
/// (`environment` is 11; `unrecognized` is 12).
static TOKEN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b[A-Za-z0-9_+/=\-]{20,}\b").expect("TOKEN_RE compiles"));

/// Runs of 4+ consecutive ASCII digits not already absorbed by the
/// earlier passes. Catches PIDs, build numbers, embedded timestamps,
/// port numbers. The 4-digit minimum preserves single-digit status
/// codes (the most common short numeric in error messages).
static DIGITS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\d{4,}").expect("DIGITS_RE compiles"));

/// Redact an `anyhow::Error` into a PII-safe category fingerprint
/// suitable for emission on `cli.command_completed`. See the module
/// rustdoc for the full pipeline.
pub fn categorize(err: &anyhow::Error) -> String {
    let raw = err.to_string();
    let head = bounded_slice(&raw, MAX_INPUT_BYTES);
    let redacted = redact(head);
    bounded_slice(&redacted, MAX_OUTPUT_BYTES).to_string()
}

/// Return a `&str` slice of at most `max` bytes from `s`, cut on a
/// UTF-8 char boundary. If `s` is already shorter than `max`, the
/// full string is returned.
fn bounded_slice(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut cut = max;
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    &s[..cut]
}

/// The redaction passes, ordered. Exposed for unit testing without
/// requiring an `anyhow::Error` construction wrapper.
///
/// Order:
/// 1. URL — most specific (scheme + `://` + content)
/// 2. FLAKE-REF — flake-style refs without `://`
/// 3. EMAIL — `@`-shaped local@domain
/// 4. IPV4 — dotted-quad; before DIGITS so octets aren't fragmented
/// 5. IPV6 — colon-hex; before HEX so compressed forms aren't fragmented
/// 6. UUID — canonical 8-4-4-4-12; before HEX so canonical UUIDs
///    land on the specific sentinel
/// 7. PATH — anything with `/` or `\` separator
/// 8. HEX — 8+ hex chars; catches no-dash UUIDs, SHAs, addresses
/// 9. TOKEN — 20+ char opaque tokens; catches API keys, JWTs
/// 10. DIGITS — final sweep for 4+ digit runs
fn redact(input: &str) -> String {
    let s = URL_RE.replace_all(input, "<url>").into_owned();
    let s = FLAKE_RE.replace_all(&s, "<flake-ref>").into_owned();
    let s = EMAIL_RE.replace_all(&s, "<email>").into_owned();
    let s = IPV4_RE.replace_all(&s, "<ip>").into_owned();
    let s = IPV6_RE.replace_all(&s, "<ip>").into_owned();
    let s = UUID_RE.replace_all(&s, "<uuid>").into_owned();
    let s = PATH_RE.replace_all(&s, "<path>").into_owned();
    let s = HEX_RE.replace_all(&s, "<hex>").into_owned();
    let s = TOKEN_RE.replace_all(&s, "<token>").into_owned();
    let s = DIGITS_RE.replace_all(&s, "<n>").into_owned();
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- Positive cases ----------

    /// Pipeline-level: a realistic top-level anyhow error redacts as
    /// expected and `categorize` wires `.to_string` → redact → bound
    /// in the right order.
    #[test]
    fn categorize_redacts_path_in_anyhow_error() {
        let err = anyhow::anyhow!("could not open /Users/alice/secret/config.toml");
        let cat = categorize(&err);
        assert!(cat.contains("<path>"), "no sentinel: {cat:?}");
    }

    #[test]
    fn redact_replaces_absolute_path() {
        assert_eq!(
            redact("could not open /etc/passwd"),
            "could not open <path>"
        );
    }

    #[test]
    fn redact_replaces_relative_dotfile_path() {
        let out = redact("missing manifest at ./.flox/env/manifest.toml");
        assert!(out.contains("<path>"), "no sentinel: {out:?}");
    }

    #[test]
    fn redact_replaces_uuid() {
        assert_eq!(
            redact("activation 459a165f-5221-4c27-b736-19d4b0d3a084 failed"),
            "activation <uuid> failed"
        );
    }

    #[test]
    fn redact_replaces_uppercase_uuid() {
        let out = redact("env 459A165F-5221-4C27-B736-19D4B0D3A084 broken");
        assert!(out.contains("<uuid>"), "no sentinel: {out:?}");
    }

    #[test]
    fn redact_replaces_flake_ref() {
        let out = redact("could not resolve github:flox/flox#default");
        assert!(out.contains("<flake-ref>"), "no sentinel: {out:?}");
    }

    #[test]
    fn redact_replaces_digit_run() {
        let out = redact("subprocess 12345 exited with status 1");
        assert!(out.contains("<n>"), "no sentinel: {out:?}");
        assert!(out.contains("status 1"), "single-digit eaten: {out:?}");
    }

    // ---------- Negative-control cases (would-have-leaked) ----------
    //
    // These were added after a reviewer probe found ~10 PII leak
    // classes the original four-pass redactor did not catch. Each
    // test asserts a specific sensitive substring does NOT survive
    // redaction. The redactor's positive-case behaviour is
    // intentionally over-aggressive; these tests guard the contract
    // against future regex relaxations.

    /// `https://`-prefixed URLs (and any other scheme) are caught —
    /// `FLAKE_RE` only matched a specific scheme list and missed
    /// these.
    #[test]
    fn redact_does_not_leak_https_url() {
        let out = redact("error sending request for url (https://ingest.example.com/api/key)");
        assert!(!out.contains("ingest.example.com"), "host leaked: {out:?}");
        assert!(!out.contains("api/key"), "path leaked: {out:?}");
        assert!(out.contains("<url>"), "no sentinel: {out:?}");
    }

    #[test]
    fn redact_does_not_leak_http_url() {
        let out = redact("connection refused: http://10.0.0.1:8080/health");
        assert!(!out.contains("10.0.0.1"), "host leaked: {out:?}");
        assert!(!out.contains("/health"), "path leaked: {out:?}");
        assert!(out.contains("<url>"), "no sentinel: {out:?}");
    }

    #[test]
    fn redact_does_not_leak_ssh_url() {
        let out = redact("could not clone ssh://git@example.com:22/org/repo.git");
        assert!(!out.contains("example.com"), "host leaked: {out:?}");
        assert!(!out.contains("repo.git"), "tail leaked: {out:?}");
        assert!(out.contains("<url>"), "no sentinel: {out:?}");
    }

    /// IPv4 octet runs are 1-3 digits and slipped through the
    /// 4+-digit `DIGITS_RE` threshold.
    #[test]
    fn redact_does_not_leak_ipv4() {
        let out = redact("connection to 192.168.1.100 failed");
        assert!(!out.contains("192.168"), "octet leaked: {out:?}");
        assert!(out.contains("<ip>"), "no sentinel: {out:?}");
    }

    /// Compressed IPv6 addresses (`::1`, `fe80::abcd`) include
    /// hex digits separated by colons; the standalone `<hex>` pass
    /// would not catch the colon-separated form.
    #[test]
    fn redact_does_not_leak_ipv6() {
        let out = redact("connection refused fe80::abcd:1234:5678");
        assert!(!out.contains("fe80"), "address leaked: {out:?}");
        assert!(out.contains("<ip>"), "no sentinel: {out:?}");
    }

    /// Hex memory addresses (`0x...`) slipped through entirely —
    /// `DIGITS_RE` only matched decimal.
    #[test]
    fn redact_does_not_leak_hex_address() {
        let out = redact("crashed at 0x7fff5fbff8a8");
        assert!(!out.contains("7fff5fbff8a8"), "address leaked: {out:?}");
        assert!(out.contains("<hex>"), "no sentinel: {out:?}");
    }

    /// SHA-256 fingerprints (64 hex chars) and SHA-1 (40 hex chars)
    /// are caught by `HEX_RE` post-fix.
    #[test]
    fn redact_does_not_leak_sha_fingerprint() {
        let out = redact(
            "expected sha256:abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
        );
        assert!(
            !out.contains("abcdef0123456789"),
            "fingerprint leaked: {out:?}"
        );
        assert!(out.contains("<hex>"), "no sentinel: {out:?}");
    }

    /// `local-part@domain.tld` shape is caught by `EMAIL_RE`.
    #[test]
    fn redact_does_not_leak_email() {
        let out = redact("user alice@example.com not authorized");
        assert!(!out.contains("alice@"), "local-part leaked: {out:?}");
        assert!(!out.contains("example.com"), "domain leaked: {out:?}");
        assert!(out.contains("<email>"), "no sentinel: {out:?}");
    }

    /// Windows backslash paths are caught by the widened `PATH_RE`
    /// even though flox is not a Windows target — defense in depth.
    #[test]
    fn redact_does_not_leak_windows_path() {
        let out = redact(r"could not read C:\Users\alice\AppData\secret.txt");
        assert!(!out.contains("alice"), "username leaked: {out:?}");
        assert!(!out.contains("secret.txt"), "tail leaked: {out:?}");
        assert!(out.contains("<path>"), "no sentinel: {out:?}");
    }

    /// No-dash 32-hex-char UUIDs (and similar opaque identifiers)
    /// are caught by `HEX_RE`.
    #[test]
    fn redact_does_not_leak_no_dash_uuid() {
        let out = redact("token 459a165f52214c27b73619d4b0d3a084 invalid");
        assert!(!out.contains("459a165f52214c27"), "id leaked: {out:?}");
        assert!(out.contains("<hex>"), "no sentinel: {out:?}");
    }

    /// API-key-shaped opaque tokens are caught by `TOKEN_RE`. The
    /// fixture uses non-hex letters so the run hits `TOKEN_RE`
    /// directly rather than being eaten by `HEX_RE` first, and a
    /// synthetic clearly-fake prefix so secret-scanning systems do
    /// not flag it.
    #[test]
    fn redact_does_not_leak_api_key() {
        // 28+ chars of `[g-z_]`: clearly not a real Stripe-shaped key.
        let synthetic = "FAKE_TOKEN_ggggggggggggggggggg";
        let input = format!("auth failed with {synthetic}");
        let out = redact(&input);
        assert!(!out.contains(synthetic), "token leaked: {out:?}");
        assert!(out.contains("<token>"), "no sentinel: {out:?}");
    }

    /// JWT-shaped tokens — three dot-separated base64-alphabet
    /// segments — are caught by `TOKEN_RE` because each segment is a
    /// 20+ char base64 run. Synthetic segments using only non-hex
    /// letters so `HEX_RE` doesn't consume them before `TOKEN_RE`
    /// runs.
    #[test]
    fn redact_does_not_leak_jwt_shape() {
        // Three synthetic 22-char segments of non-hex letters.
        let synthetic = "gggggggggggggggggggggg.hhhhhhhhhhhhhhhhhhhhhh.iiiiiiiiiiiiiiiiiiiiii";
        let input = format!("bearer {synthetic}");
        let out = redact(&input);
        assert!(
            !out.contains("gggggggggggggggggggggg"),
            "segment leaked: {out:?}"
        );
        assert!(out.contains("<token>"), "no sentinel: {out:?}");
    }

    /// Truncation-tail leak: a long error message with a sensitive
    /// path at byte 60+ would have shipped a partial path under the
    /// pre-redact truncation order. The redact-before-truncate
    /// order fixes this.
    #[test]
    fn redact_does_not_leak_truncated_path_tail() {
        let noisy = "x".repeat(54);
        let raw = format!("{noisy} /etc/passwd");
        let err = anyhow::anyhow!("{raw}");
        let cat = categorize(&err);
        assert!(!cat.contains("/etc"), "partial path leaked: {cat:?}");
        assert!(!cat.contains("passwd"), "tail leaked: {cat:?}");
    }

    /// Chained `anyhow` errors via `.context()` should redact based
    /// on the OUTER (top-frame) context only; the inner contexts are
    /// not the top frame per `anyhow::Error::to_string`'s contract.
    /// This pins the "top frame" semantic the module rustdoc claims.
    #[test]
    fn categorize_uses_top_frame_of_anyhow_chain() {
        let inner = anyhow::anyhow!("inner /Users/alice/secret missing");
        let outer = inner.context("dispatch failed at boundary");
        let cat = categorize(&outer);
        assert!(
            cat.contains("dispatch failed at boundary"),
            "wrong frame: {cat:?}"
        );
        assert!(!cat.contains("/Users/alice"), "inner path leaked: {cat:?}");
        assert!(!cat.contains("alice"), "username leaked: {cat:?}");
    }

    // ---------- Truncation behaviour ----------

    /// Output is bounded at `MAX_OUTPUT_BYTES` — verified on a long
    /// input whose redacted form would otherwise overflow.
    #[test]
    fn categorize_truncates_output_to_max_bound() {
        let long = "a".repeat(2000);
        let err = anyhow::anyhow!("{long}");
        let cat = categorize(&err);
        assert!(
            cat.len() <= MAX_OUTPUT_BYTES,
            "output too long: {} bytes",
            cat.len()
        );
    }

    /// UTF-8 boundary fix-up: when the byte limit lands mid-char,
    /// the slice cuts back to the previous boundary. A 4-byte char
    /// sequence straddling byte 96 exercises this. `é` is 2 bytes
    /// (`0xC3 0xA9`).
    #[test]
    fn categorize_truncates_at_utf8_boundary() {
        // 95 ASCII chars + `é` (2 bytes, straddles bytes 95-96).
        // The byte at index 96 is not a char boundary — the fix-up
        // loop must back off to 95.
        let prefix = "x".repeat(95);
        let err = anyhow::anyhow!("{prefix}éééé");
        let cat = categorize(&err);
        assert!(cat.len() <= MAX_OUTPUT_BYTES, "over bound: {}", cat.len());
        // A panic on a non-char-boundary slice would have failed
        // the test; reaching here proves the fix-up works.
    }

    /// Input bound is generous — anyhow top frames are rarely large,
    /// but a pathological multi-megabyte input must not pin the
    /// regex passes. The output is still bounded; the input bound
    /// just caps the regex compute window.
    #[test]
    fn categorize_handles_large_input() {
        let huge = "a".repeat(100_000);
        let err = anyhow::anyhow!("{huge}");
        let cat = categorize(&err);
        assert!(cat.len() <= MAX_OUTPUT_BYTES);
    }
}
