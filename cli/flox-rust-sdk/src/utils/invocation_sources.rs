use std::collections::HashSet;
use std::env;
use std::sync::LazyLock;

/// Heuristics table for inferring invocation sources from environment
/// Each entry: (env_var_name, expected_value_or_none, invocation_source_tag)
/// Use None for expected_value to check env var presence only
const INFERENCE_HEURISTICS: &[(&str, Option<&str>, &str)] = &[
    // CI and containerd contexts
    ("CI", None, "ci"),
    ("FLOX_CONTAINERD", None, "containerd"),
    // Terminal programs
    ("TERM_PROGRAM", Some("vscode"), "term.vscode"),
    ("TERM_PROGRAM", Some("kiro"), "agentic.kiro"),
    // Claude Code detection
    (
        "CLAUDE_CODE_ENTRYPOINT",
        Some("cli"),
        "agentic.claude-code.cli",
    ),
    ("CLAUDE_CODE_SSE_PORT", None, "agentic.claude-code.plugin"),
    // Other agentic tools
    ("ANTIGRAVITY_AGENT", Some("1"), "agentic.antigravity"),
    ("GEMINI_CLI", None, "agentic.gemini"),
];

/// Detect invocation sources from environment heuristics
fn detect_heuristics() -> impl Iterator<Item = String> {
    INFERENCE_HEURISTICS
        .iter()
        .filter_map(|(env_var, expected_value, source)| {
            let matches = match expected_value {
                Some(expected) => env::var(env_var).as_deref() == Ok(expected),
                None => env::var(env_var).is_ok(),
            };
            if matches {
                Some(source.to_string())
            } else {
                None
            }
        })
}

/// Detect all invocation sources for the current CLI invocation
///
/// Returns a deduplicated vector of invocation source identifiers.
/// Sources are detected from:
/// 1. Explicit FLOX_INVOCATION_SOURCE environment variable (comma-separated)
/// 2. Inference heuristics for CI, containerd, agentic tooling, and other contexts
pub fn detect_invocation_sources() -> Vec<String> {
    let mut sources = HashSet::new();

    // Explicit sources from FLOX_INVOCATION_SOURCE
    if let Ok(explicit) = env::var("FLOX_INVOCATION_SOURCE") {
        for source in explicit.split(',').map(str::trim) {
            if !source.is_empty() {
                sources.insert(source.to_string());
            }
        }
    }

    // Apply all inference heuristics (CI, containerd, agentic tools, etc.)
    sources.extend(detect_heuristics());

    // Convert to sorted vec for consistent ordering
    let mut result: Vec<String> = sources.into_iter().collect();
    result.sort();
    result
}

/// Detected invocation sources for this CLI run, computed once at startup
pub static INVOCATION_SOURCES: LazyLock<Vec<String>> = LazyLock::new(detect_invocation_sources);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_invocation_sources_explicit() {
        temp_env::with_var("FLOX_INVOCATION_SOURCE", Some("vscode.terminal"), || {
            let sources = detect_invocation_sources();
            assert!(sources.contains(&"vscode.terminal".to_string()));
        });
    }

    #[test]
    fn test_detect_invocation_sources_multiple_explicit() {
        temp_env::with_var(
            "FLOX_INVOCATION_SOURCE",
            Some("ci.github-actions,agentic.flox-mcp"),
            || {
                let sources = detect_invocation_sources();
                assert!(sources.contains(&"ci.github-actions".to_string()));
                assert!(sources.contains(&"agentic.flox-mcp".to_string()));
            },
        );
    }

    #[test]
    fn test_detect_invocation_sources_ci() {
        temp_env::with_var("CI", Some("true"), || {
            let sources = detect_invocation_sources();
            assert!(sources.contains(&"ci".to_string()));
        });
    }

    #[test]
    fn test_detect_invocation_sources_vscode_terminal() {
        temp_env::with_var("TERM_PROGRAM", Some("vscode"), || {
            let sources = detect_invocation_sources();
            assert!(sources.contains(&"term.vscode".to_string()));
        });
    }

    #[test]
    fn test_detect_invocation_sources_containerd() {
        temp_env::with_var("FLOX_CONTAINERD", Some("1"), || {
            let sources = detect_invocation_sources();
            assert!(sources.contains(&"containerd".to_string()));
        });
    }

    #[test]
    fn test_detect_invocation_sources_agentic_heuristic() {
        temp_env::with_var("CLAUDE_CODE_ENTRYPOINT", Some("cli"), || {
            let sources = detect_invocation_sources();
            assert!(sources.contains(&"agentic.claude-code.cli".to_string()));
        });
    }

    #[test]
    fn test_detect_invocation_sources_deduplication() {
        temp_env::with_vars(
            [("FLOX_INVOCATION_SOURCE", Some("ci")), ("CI", Some("true"))],
            || {
                let sources = detect_invocation_sources();
                // Should only contain "ci" once despite both explicit and inferred
                assert_eq!(sources.iter().filter(|s| *s == "ci").count(), 1);
            },
        );
    }

    #[test]
    fn test_detect_invocation_sources_nested() {
        temp_env::with_vars(
            [
                ("FLOX_INVOCATION_SOURCE", Some("vscode.terminal")),
                ("CLAUDE_CODE_SSE_PORT", Some("12345")),
            ],
            || {
                let sources = detect_invocation_sources();
                assert!(sources.contains(&"vscode.terminal".to_string()));
                assert!(sources.contains(&"agentic.claude-code.plugin".to_string()));
            },
        );
    }

    #[test]
    fn test_detect_invocation_sources_sorted() {
        temp_env::with_var("FLOX_INVOCATION_SOURCE", Some("zebra,apple,middle"), || {
            let sources = detect_invocation_sources();
            let sorted_sources = {
                let mut s = sources.clone();
                s.sort();
                s
            };
            assert_eq!(sources, sorted_sources);
        });
    }
}
