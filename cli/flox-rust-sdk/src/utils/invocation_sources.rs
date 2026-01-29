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
///
/// Applies hierarchical deduplication: if both "ci" and "ci.github-actions" exist,
/// only "ci.github-actions" is kept (more specific supersedes less specific).
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

    // Convert to vec for hierarchical deduplication
    let sources_vec: Vec<String> = sources.into_iter().collect();

    // Apply hierarchical deduplication: remove any source if a more specific version exists
    // e.g., if both "ci" and "ci.github-actions" exist, remove "ci"
    let mut result: Vec<String> = sources_vec
        .iter()
        .filter(|source| {
            // Keep this source only if NO other source is more specific
            // A source is "more specific" if it starts with this source + "."
            !sources_vec
                .iter()
                .any(|other| other != *source && other.starts_with(&format!("{}.", source)))
        })
        .cloned()
        .collect();

    // Sort for consistent ordering
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

    #[test]
    fn test_detect_invocation_sources_hierarchical_deduplication() {
        // Test case: both "ci" (inferred from CI env var) and "ci.github-actions" (explicit)
        // Should only keep "ci.github-actions" because it's more specific
        temp_env::with_vars(
            [
                ("FLOX_INVOCATION_SOURCE", Some("ci.github-actions")),
                ("CI", Some("true")),
            ],
            || {
                let sources = detect_invocation_sources();
                println!("Sources detected: {:?}", sources);
                assert!(sources.contains(&"ci.github-actions".to_string()));
                assert!(
                    !sources.contains(&"ci".to_string()),
                    "Generic 'ci' should be removed when 'ci.github-actions' exists"
                );
            },
        );
    }

    #[test]
    fn test_detect_invocation_sources_hierarchical_multiple_levels() {
        // Test case: "ci", "ci.github", "ci.github.actions"
        // Should only keep "ci.github.actions" (most specific)
        temp_env::with_var(
            "FLOX_INVOCATION_SOURCE",
            Some("ci,ci.github,ci.github.actions"),
            || {
                let sources = detect_invocation_sources();
                println!("Sources detected: {:?}", sources);
                assert!(sources.contains(&"ci.github.actions".to_string()));
                assert!(
                    !sources.contains(&"ci".to_string()),
                    "'ci' should be removed"
                );
                assert!(
                    !sources.contains(&"ci.github".to_string()),
                    "'ci.github' should be removed"
                );
                // Count only ci-related sources, there may be other detected sources from env
                let ci_sources: Vec<_> = sources.iter().filter(|s| s.starts_with("ci")).collect();
                assert_eq!(
                    ci_sources.len(),
                    1,
                    "Should only contain one ci-related source: ci.github.actions"
                );
            },
        );
    }

    #[test]
    fn test_detect_invocation_sources_hierarchical_different_roots() {
        // Test case: "ci" and "containerd" are different hierarchies
        // Both should be kept since neither is more specific than the other
        temp_env::with_vars(
            [("CI", Some("true")), ("FLOX_CONTAINERD", Some("1"))],
            || {
                let sources = detect_invocation_sources();
                println!("Sources detected: {:?}", sources);
                assert!(sources.contains(&"ci".to_string()));
                assert!(sources.contains(&"containerd".to_string()));
                // Note: May also contain other detected sources, so don't check exact count
                assert!(
                    sources.len() >= 2,
                    "Should contain at least ci and containerd"
                );
            },
        );
    }

    #[test]
    fn test_detect_invocation_sources_hierarchical_mixed() {
        // Test case: "ci.github-actions", "containerd", "agentic" (if agentic.claude-code exists)
        // Should keep "ci.github-actions", "containerd", but remove "agentic" if "agentic.X" exists
        temp_env::with_vars(
            [
                ("FLOX_INVOCATION_SOURCE", Some("ci.github-actions")),
                ("FLOX_CONTAINERD", Some("1")),
                ("CLAUDE_CODE_SSE_PORT", Some("12345")),
            ],
            || {
                let sources = detect_invocation_sources();
                assert!(sources.contains(&"ci.github-actions".to_string()));
                assert!(sources.contains(&"containerd".to_string()));
                assert!(sources.contains(&"agentic.claude-code.plugin".to_string()));
                // No generic "ci" or "agentic" should be present
                assert!(!sources.contains(&"ci".to_string()));
                assert!(!sources.contains(&"agentic".to_string()));
            },
        );
    }
}
