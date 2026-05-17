# Pilot retrospective — iteration 2 (run on 2026-05-17T06:55:24Z)

Compared with iteration 1:
- Noise filter active (drops URL-only, suggestion-only, lgtm/praise comments)
- CLUSTER_THRESHOLD lowered 0.6 → 0.35
- agents_md_coverage rewritten to compare rule_statement text vs AGENTS.md content
- 'imports' taxonomy description broadened
- Batch size halved 30 → 15 (no Haiku self-doubt retries this iteration)
- Storage additions: review_summary, pr_comment, commit_id, thread resolution state, prompt_hash

## Corpus shape
| prs | line_comments_total | noise_dropped | classified | review_summaries | pr_top_level_comments | threads_resolved | threads_unresolved | findings |
|-----|---------------------|---------------|------------|------------------|-----------------------|------------------|--------------------|----------|
| 13  | 115                 | 21            | 94         | 7                | 29                    | 72               | 43                 | 49       |

## Area coverage (non-noise)
|        area        | COUNT(*) |
|--------------------|----------|
| activations        | 48       |
| cli/other          | 22       |
| commands           | 15       |
| models/environment | 7        |
| manifest           | 1        |
| cli/utils          | 1        |

## Taxonomy distribution
|       taxonomy       | n  | avg_conf |
|----------------------|----|----------|
| other                | 41 | 0.18     |
| semantic-correctness | 12 | 0.78     |
| testing              | 12 | 0.76     |
| user-facing-messages | 10 | 0.73     |
| naming               | 7  | 0.83     |
| control-flow         | 4  | 0.84     |
| formatting-style     | 4  | 0.71     |
| error-handling       | 2  | 0.76     |
| logging-tracing      | 1  | 0.6      |
| type-safety          | 1  | 0.8      |

## 'Other' bucket — high-confidence (candidates for taxonomy expansion in Task 8.5)

## Clustering effectiveness — finding-evidence distribution
| evd_per_finding | n_findings |
|-----------------|------------|
| 1               | 45         |
| 2               | 4          |

## Top 25 findings
|        area        |     scope     | t1 | evd | xa | md | conf |                                            rule                                            |
|--------------------|---------------|----|-----|----|----|------|--------------------------------------------------------------------------------------------|
| cli/other          | area-specific | 1  | 2   | 1  | 0  | 0.75 | Consider combining slow test cases into single expect script for efficiency.               |
| activations        | area-specific | 1  | 1   | 1  | 0  | 0.71 | Refactor duplicated activation logic into shared helpers to eliminate spaghetti and ensure |
| activations        | area-specific | 1  | 1   | 1  | 0  | 0.71 | Apply environment variables via a single shared function to avoid duplication across activ |
| activations        | area-specific | 1  | 1   | 1  | 0  | 0.71 | Extract environment variable assembly logic into reusable components to enable consistent  |
| activations        | area-specific | 1  | 1   | 1  | 0  | 0.71 | Evaluate whether EnvDiff and ActivationDiff can share implementation or should remain sepa |
| activations        | area-specific | 1  | 1   | 1  | 0  | 0.71 | Keep diff size minimal by removing unused helper functions introduced during refactoring.  |
| activations        | area-specific | 1  | 1   | 1  | 0  | 0.71 | Place test-only code in separate #[cfg(test)] blocks to keep implementation scanning clean |
| activations        | area-specific | 1  | 1   | 1  | 0  | 0.71 | Use formatdoc! for multiline formatted strings instead of raw format!.                     |
| cli/other          | area-specific | 1  | 1   | 1  | 0  | 0.71 | Update test descriptions when error messages change to match actual behavior and scenarios |
| cli/other          | area-specific | 1  | 1   | 1  | 0  | 0.71 | Drop unclear tests or rewrite them to clearly test intended behavior.                      |
| cli/other          | area-specific | 1  | 1   | 1  | 0  | 0.71 | Consolidate redundant test cases into comprehensive ones that cover same scope.            |
| cli/other          | area-specific | 1  | 1   | 1  | 0  | 0.71 | Use exact line matching with whitespace normalization to prevent false positives in test a |
| commands           | area-specific | 1  | 1   | 1  | 0  | 0.71 | Prefix unused parameters with underscore; remove entirely if truly unused to avoid confusi |
| models/environment | area-specific | 1  | 1   | 1  | 0  | 0.71 | Document design decisions in code comments to guide future maintainers and code reviewers. |
| models/environment | area-specific | 1  | 1   | 1  | 0  | 0.71 | Enhance existing comments with supporting evidence to clarify assumptions and reduce futur |
| commands           | area-specific | 1  | 1   | 1  | 0  | 0.71 | Choose terminology that accurately reflects whether values are paths or abstract targets.  |
| activations        | area-specific | 1  | 1   | 1  | 0  | 0.61 | Consider backwards compatibility when changing feature defaults between versions.          |
| activations        | area-specific | 1  | 1   | 1  | 0  | 0.61 | Clarify output formatting decisions with comments.                                         |
| activations        | area-specific | 1  | 1   | 1  | 0  | 0.61 | Derive Display trait when possible instead of manual implementation.                       |
| activations        | area-specific | 1  | 1   | 1  | 0  | 0.61 | Consider combining slow test cases into single expect script for efficiency.               |
| activations        | area-specific | 0  | 2   | 1  | 0  | 0.55 | Rename types to reflect their semantic purpose; DiffSerializer clearly conveys serializati |
| cli/other          | area-specific | 0  | 2   | 1  | 0  | 0.55 | Add comments noting when feature-flag-dependent tests should be removed.                   |
| cli/other          | area-specific | 0  | 2   | 1  | 0  | 0.55 | Consolidate multiple partial-string assertions into single targeted assertion.             |
| activations        | area-specific | 1  | 1   | 1  | 0  | 0.51 | Avoid redundant pattern matching by moving logic into polymorphic methods.                 |
| activations        | area-specific | 0  | 1   | 1  | 0  | 0.51 | Preserve helper functions when they serve shared logic and multiple callers depend on them |

## AGENTS.md matching distribution
| in_agents_md | n  | avg_conf |
|--------------|----|----------|
| 0            | 49 | 0.53     |

## Sample matched AGENTS.md sections (verify the new heuristic is matching meaningfully)

## Thread resolution as a signal — was_addressed vs thread_resolved
| was_addressed | thread_resolved | COUNT(*) |
|---------------|-----------------|----------|
|               | 0               | 22       |
|               | 1               | 14       |
| 0             | 0               | 5        |
| 0             | 1               | 8        |
| 1             | 0               | 10       |
| 1             | 1               | 35       |

## Lowest-confidence classifications (prompt sanity check)
| comment_id |  author  | taxonomy | conf |                                       body                                       | rule |
|------------|----------|----------|------|----------------------------------------------------------------------------------|------|
| 3237190143 | mkenigs  | other    | 0.05 | nonblocking: we're going to have to unset this on deactivate. We can grab it in  |      |
| 3237493696 | djsauble | other    | 0.05 | https://linear.app/floxdotdev/issue/DEV-81/placeholder-restore-sneaky-environmen |      |
| 3232032681 | dcarley  | other    | 0.1  | Oops, I got inner and outer the wrong way round, but you get the idea.           |      |
| 3235627981 | mkenigs  | other    | 0.1  | Haha unironically might not be a bad idea. thar_be_dragons_dont_touch_without_ma |      |
| 3236366626 | mkenigs  | other    | 0.1  | Fixing                                                                           |      |
| 3236367515 | mkenigs  | other    | 0.1  | Fixing                                                                           |      |
| 3236378609 | mkenigs  | other    | 0.1  | Adding comment                                                                   |      |
| 3183731152 | djsauble | other    | 0.15 | Follow-up changes: https://github.com/flox/flox/pull/4202/commits/13a546b3bda387 |      |
| 3183805696 | djsauble | other    | 0.15 | Struct renaming: https://github.com/flox/flox/pull/4202/commits/42c8f5d7e407e9c3 |      |
| 3184140213 | djsauble | other    | 0.15 | Reverted `activate_in_place` and restored `render_legacy_exports` to match main: |      |

## Coverage audit
auditing 13 PRs (mode=full)
info: review_summary rows=7, pr_comment rows=29
---
OK
