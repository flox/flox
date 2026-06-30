# Pilot retrospective — iteration 3 (run on 2026-05-17T07:22:35Z)

## Iteration comparison summary

| Metric | Iter 1 | Iter 2 | Iter 3 |
|---|---|---|---|
| Comments to classify | 115 | 94 (noise filter) | 94 |
| `other` count | 62 (54%) | 41 (44%) | 29 (31%) |
| Findings total | 51 | 49 | 52 |
| Findings with evd > 1 (clustered) | 2 (4%) | 4 (8%) | 9 (17%) |
| Findings tagged in_agents_md=1 | 51 (100%) | 0 (0%) | 38 (73%) |
| was_addressed=NULL | — | ~38 | 11 |
| Batch retries needed | 1/4 | 0/7 | 0/7 |

## Corpus shape
| prs | line_comments_total | noise_dropped | classified | review_summaries | pr_top_level_comments | threads_resolved | threads_unresolved | findings |
|-----|---------------------|---------------|------------|------------------|-----------------------|------------------|--------------------|----------|
| 13  | 115                 | 21            | 94         | 7                | 29                    | 72               | 43                 | 52       |

## Taxonomy distribution
|       taxonomy       | n  | avg_conf |
|----------------------|----|----------|
| other                | 29 | 0.15     |
| testing              | 19 | 0.76     |
| control-flow         | 12 | 0.77     |
| semantic-correctness | 12 | 0.74     |
| user-facing-messages | 11 | 0.77     |
| naming               | 4  | 0.81     |
| formatting-style     | 3  | 0.77     |
| error-handling       | 2  | 0.83     |
| imports              | 1  | 0.7      |
| logging-tracing      | 1  | 0.78     |

## was_addressed × thread_resolved cross-tab
| was_addressed | thread_resolved | COUNT(*) |
|---------------|-----------------|----------|
|               | 0               | 11       |
| 0             | 0               | 19       |
| 1             | 0               | 7        |
| 1             | 1               | 57       |

## Finding-evidence distribution (clustering effectiveness)
| evd | n_findings |
|-----|------------|
| 1   | 43         |
| 2   | 6          |
| 3   | 2          |
| 4   | 1          |

## AGENTS.md match distribution (sections matched)
| in_agents_md |     section      | n  |
|--------------|------------------|----|
| 1            | Conventions      | 36 |
| 0            | (no match)       | 14 |
| 1            | Common Commands  | 1  |
| 1            | Project Overview | 1  |

## Top 25 findings (by confidence)
|        area        |     scope     | t1 | evd | md | conf |                                            rule                                            |
|--------------------|---------------|----|-----|----|------|--------------------------------------------------------------------------------------------|
| activations        | area-specific | 1  | 3   | 1  | 0.79 | Refactor apply_activation_env to extract shared environment variable assembly logic for re |
| cli/other          | area-specific | 1  | 2   | 1  | 0.75 | Use sed to strip whitespace before line-exact grep matching to avoid false positives in ba |
| cli/other          | area-specific | 1  | 3   | 1  | 0.72 | Use single assert_output checks for shell-agnostic test validation.                        |
| activations        | area-specific | 1  | 1   | 0  | 0.71 | Defer refactoring of in-place activation rendering to a separate PR to minimize unrelated  |
| activations        | area-specific | 1  | 1   | 1  | 0.71 | Remove deprecated or intermediate helper functions to minimize diff size when cleaner alte |
| activations        | area-specific | 1  | 1   | 1  | 0.71 | Evaluate whether EnvDiff and ActivationDiff can be unified or if their differing semantics |
| activations        | area-specific | 1  | 1   | 1  | 0.71 | Recognize unreachable code paths and remove or clearly document them rather than adding er |
| activations        | area-specific | 1  | 1   | 1  | 0.71 | Use formatdoc! or indoc! macros for multiline formatted strings.                           |
| activations        | area-specific | 1  | 1   | 1  | 0.71 | Use version suffixes in environment variable names to avoid namespace collisions.          |
| cli/other          | area-specific | 1  | 1   | 1  | 0.71 | Remove tests that do not verify their stated intent.                                       |
| cli/other          | area-specific | 1  | 1   | 1  | 0.71 | Remove redundant test coverage superseded by comprehensive tests.                          |
| models/environment | area-specific | 1  | 1   | 1  | 0.71 | Understand edge cases and document assumptions about package output defaults               |
| models/environment | area-specific | 1  | 1   | 0  | 0.71 | Document edge case behaviors explicitly with reasoning instead of leaving incomplete       |
| models/environment | area-specific | 1  | 1   | 1  | 0.71 | Clarify comments when they would guide future code review and understanding                |
| commands           | area-specific | 1  | 1   | 1  | 0.71 | Use domain-specific nouns (targets, build outputs) instead of generic terms (artifacts) in |
| activations        | area-specific | 1  | 1   | 0  | 0.71 | Document all CLI flags in usage messages.                                                  |
| commands           | area-specific | 1  | 1   | 1  | 0.71 | Reuse shell type enums to catch unsupported shells at compile time.                        |
| cli/other          | area-specific | 1  | 1   | 1  | 0.71 | Prefer printenv over eval for simpler quoting in shell tests.                              |
| cli/other          | area-specific | 0  | 4   | 0  | 0.63 | Combine TODO comments with simplified assertions for feature-flag dependent tests.         |
| activations        | area-specific | 1  | 1   | 1  | 0.61 | Balance code organization (per-shell modules) against coherence of related logic.          |
| cli/other          | area-specific | 1  | 1   | 1  | 0.61 | Write tests that verify authentication requirements in non-interactive contexts            |
| commands           | area-specific | 1  | 1   | 0  | 0.61 | Prefix unused parameters with underscore to signal intentional non-use                     |
| activations        | area-specific | 1  | 1   | 1  | 0.61 | Organize shell-specific functions into their respective shell modules.                     |
| cli/other          | area-specific | 0  | 2   | 1  | 0.55 | Consolidate overlapping assertions; each test should verify one semantic property.         |
| cli/other          | area-specific | 1  | 2   | 1  | 0.55 | Measure test execution time and ensure new integration tests don't slow the suite excessiv |

## 'Other' bucket — any high-confidence candidates? (>=0.4)

## Gap candidates: findings NOT in AGENTS.md (review for novelty)
|        area        | t1 | evd | conf |                                                     rule                                                      |
|--------------------|----|-----|------|---------------------------------------------------------------------------------------------------------------|
| activations        | 1  | 1   | 0.71 | Defer refactoring of in-place activation rendering to a separate PR to minimize unrelated changes in this PR. |
| models/environment | 1  | 1   | 0.71 | Document edge case behaviors explicitly with reasoning instead of leaving incomplete                          |
| activations        | 1  | 1   | 0.71 | Document all CLI flags in usage messages.                                                                     |
| cli/other          | 0  | 4   | 0.63 | Combine TODO comments with simplified assertions for feature-flag dependent tests.                            |
| commands           | 1  | 1   | 0.61 | Prefix unused parameters with underscore to signal intentional non-use                                        |
| activations        | 1  | 2   | 0.55 | Describe manual testing approach in test documentation for future automation efforts.                         |
| activations        | 1  | 1   | 0.51 | Test backward compatibility with older CLI feature states.                                                    |
| activations        | 0  | 1   | 0.51 | Rename ActivationDiff to DiffSerializer to clarify its role in serialization.                                 |
| cli/other          | 1  | 1   | 0.51 | Test hook automatic firing behavior, not just registration.                                                   |
| commands           | 1  | 1   | 0.51 | Avoid redundant validation when upstream guards are sufficient.                                               |
| commands           | 0  | 2   | 0.35 | Use message::deleted or message::updated for cleanup/removal, reserve message::created for new artifacts      |
| commands           | 0  | 2   | 0.35 | Use consistent verb-led phrasing across related match arms in messages                                        |
| activations        | 0  | 1   | 0.31 | Document behavioral rationale by examining reference implementations when purpose is unclear.                 |
| commands           | 0  | 1   | 0.31 | Consider bulleted lists for enumerations with multiple items instead of comma-separated                       |
