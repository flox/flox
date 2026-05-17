# Task 8 — full-corpus findings digest

**Corpus:** 8-month window (2025-09-17 → 2026-05-17). 216 Rust-touching PRs from 335 merged.

## Corpus shape
| prs | line_comments_total | noise_dropped | classified_pool | classified | review_summaries | issue_comments | threads_resolved | threads_unresolved | findings |
|-----|---------------------|---------------|-----------------|------------|------------------|----------------|------------------|--------------------|----------|
| 216 | 1047                | 87            | 960             | 944        | 96               | 270            | 459              | 588                | 522      |

## Taxonomy distribution
|       taxonomy       |  n  | avg_conf |
|----------------------|-----|----------|
| other                | 366 | 0.19     |
| semantic-correctness | 128 | 0.71     |
| testing              | 88  | 0.72     |
| user-facing-messages | 73  | 0.75     |
| control-flow         | 68  | 0.73     |
| naming               | 55  | 0.74     |
| error-handling       | 51  | 0.74     |
| type-safety          | 39  | 0.74     |
| manifest-usage       | 17  | 0.71     |
| formatting-style     | 15  | 0.73     |
| imports              | 13  | 0.73     |
| logging-tracing      | 12  | 0.69     |
| provider-traits      | 12  | 0.76     |
| deprecated-patterns  | 7   | 0.74     |

## Reviewer distribution (top 12)
|     author      | tier | COUNT(*) |
|-----------------|------|----------|
| mkenigs         | 1    | 281      |
| ysndr           | 1    | 218      |
| dcarley         | 1    | 168      |
| zmitchell       | 3    | 82       |
| gilmishal       | 2    | 62       |
| billlevine      | 2    | 51       |
| djsauble        | 2    | 49       |
| Copilot         | 4    | 16       |
| brendaneamon    | 3    | 13       |
| limeytexan      | 3    | 9        |
| mrswastik-robot | 3    | 3        |
| tanjadev        | 3    | 2        |

## Area distribution (non-noise)
|        area        | COUNT(*) |
|--------------------|----------|
| cli/other          | 236      |
| activations        | 178      |
| commands           | 163      |
| providers          | 129      |
| models/environment | 74       |
| manifest           | 54       |
| other              | 49       |
| models/other       | 27       |
| cli/utils          | 19       |
| core               | 14       |
| commands/init      | 9        |
| commands/services  | 8        |

## Finding scope breakdown
|     scope     | COUNT(*) |
|---------------|----------|
| area-specific | 509      |
| cross-cutting | 13       |

## Finding clustering effectiveness
| evd | n_findings |
|-----|------------|
| 1   | 474        |
| 2   | 36         |
| 3   | 6          |
| 4   | 4          |
| 5   | 1          |
| 6   | 1          |

## AGENTS.md match distribution
| in_agents_md | COUNT(*) | avg_conf |
|--------------|----------|----------|
| 0            | 115      | 0.58     |
| 1            | 407      | 0.58     |

## was_addressed × thread_resolved
| was_addressed | thread_resolved | COUNT(*) |
|---------------|-----------------|----------|
|               | 0               | 240      |
|               | 1               | 20       |
| 0             | 0               | 183      |
| 0             | 1               | 11       |
| 1             | 0               | 135      |
| 1             | 1               | 355      |

## TOP 13 CROSS-CUTTING FINDINGS
|                                      theme                                       | evd | xa | t1 | md | conf |                                           rule                                            |
|----------------------------------------------------------------------------------|-----|----|----|----|------|-------------------------------------------------------------------------------------------|
| Review comment addressing code change.                                           | 5   | 3  | 2  | 0  | 1.0  | Review comment addressing code change.                                                    |
| Use complete sentences in errors; suggest next steps; follow brand and emoji con | 1   | 4  | 1  | 1  | 0.84 | Use complete sentences in errors; suggest next steps; follow brand and emoji conventions. |
| Use complete sentences in errors; suggest next steps; follow brand and emoji con | 1   | 4  | 1  | 1  | 0.84 | Use complete sentences in errors; suggest next steps; follow brand and emoji conventions. |
| Use complete sentences in errors; suggest next steps; follow brand and emoji con | 1   | 4  | 1  | 1  | 0.84 | Use complete sentences in errors; suggest next steps; follow brand and emoji conventions. |
| Use descriptive names following established patterns in the same file.           | 2   | 2  | 1  | 1  | 0.81 | Use descriptive names following established patterns in the same file.                    |
| Use descriptive names following established patterns in the same file.           | 1   | 2  | 1  | 1  | 0.77 | Use descriptive names following established patterns in the same file.                    |
| Review comment addressing code change.                                           | 1   | 3  | 1  | 0  | 0.74 | Review comment addressing code change.                                                    |
| Review comment addressing code change.                                           | 1   | 3  | 1  | 0  | 0.74 | Review comment addressing code change.                                                    |
| Add tests to cover new functionality and edge cases.                             | 1   | 2  | 1  | 0  | 0.67 | Add tests to cover new functionality and edge cases.                                      |
| Add tests to cover new functionality and edge cases.                             | 1   | 2  | 1  | 0  | 0.67 | Add tests to cover new functionality and edge cases.                                      |
| Fix logic errors to match intended behavior.                                     | 1   | 2  | 1  | 0  | 0.67 | Fix logic errors to match intended behavior.                                              |
| Fix logic errors to match intended behavior.                                     | 1   | 2  | 1  | 0  | 0.67 | Fix logic errors to match intended behavior.                                              |
| Verify logic moved to other functions to ensure nothing is lost.                 | 1   | 2  | 1  | 1  | 0.57 | Verify logic moved to other functions to ensure nothing is lost.                          |

## Top 25 area-specific by confidence
|        area        | t1 | evd | md | conf |                                            rule                                            |
|--------------------|----|-----|----|------|--------------------------------------------------------------------------------------------|
| cli/other          | 2  | 6   | 1  | 0.87 | Simplify test assertions by eliminating nested run statements for cleaner verification.    |
| cli/other          | 1  | 4   | 1  | 0.83 | Use domain types like Url instead of &str for type safety at function boundaries.          |
| commands           | 1  | 3   | 1  | 0.79 | Use pkgs/default.nix for package storage to match nixpkgs conventions and existing example |
| activations        | 1  | 3   | 1  | 0.79 | Extract shared helper functions only when genuinely needed by multiple callers; avoid over |
| providers          | 2  | 3   | 1  | 0.79 | Remove deprecated trait implementations and replace with new patterns; avoid extending dep |
| commands           | 1  | 2   | 1  | 0.75 | Accept flake refs for package sources and make nixpkgs source configurable.                |
| commands           | 1  | 2   | 1  | 0.75 | Verify that shell completion works end-to-end in interactive shells, not just static outpu |
| cli/other          | 1  | 2   | 1  | 0.75 | Write complete sentences in docstrings; explain intent and valid options clearly.          |
| models/environment | 2  | 2   | 1  | 0.75 | Add unit or integration tests for newly added code paths and error handling logic.         |
| models/environment | 2  | 2   | 1  | 0.75 | Verify comment semantics match actual implementation behavior before accepting changes.    |
| models/environment | 2  | 2   | 1  | 0.75 | Create separate error variants or wrap diverse source errors in io::Error with ErrorKind:: |
| cli/other          | 1  | 2   | 1  | 0.75 | Follow established naming conventions like str_to_x for parser functions to ensure consist |
| activations        | 1  | 2   | 1  | 0.75 | Extract shared activation data structures into a common crate to reduce duplication.       |
| providers          | 2  | 2   | 1  | 0.75 | Add error variants to enum rather than parsing string output; classify errors at provider  |
| providers          | 1  | 2   | 1  | 0.75 | Use relative paths in error messages to reduce noise and match user expectations about pro |
| models/other       | 1  | 2   | 1  | 0.75 | Parsing order must split outputs (^) before version (@) to avoid ambiguity in argument par |
| models/other       | 1  | 2   | 1  | 0.75 | Avoid adding Ord/PartialOrd traits unless semantically justified by actual use cases.      |
| manifest           | 2  | 2   | 1  | 0.75 | Allow ^output specifications without #attr in flake URLs to match Nix behavior.            |
| manifest           | 1  | 2   | 1  | 0.75 | Drop exhaustive test coverage in favor of representative assertion cases to avoid redundan |
| providers          | 2  | 2   | 1  | 0.75 | Use Manifest constructor helpers like `migrated_manifest()` instead of calling `migrate_ty |
| providers          | 1  | 2   | 0  | 0.75 | Add structured tracing logs for all authentication flow branches.                          |
| cli/other          | 1  | 4   | 1  | 0.73 | Use generic terminology (e.g. 'provider', 'token auth') unless implementation-specific det |
| providers          | 1  | 4   | 1  | 0.73 | Use complete sentences in errors; suggest next steps; follow brand and emoji conventions.  |
| commands           | 1  | 1   | 1  | 0.71 | Extend error handling to cover all ConcreteEnvironment variants, not just Path.            |
| commands           | 1  | 1   | 1  | 0.71 | Distinguish auth status between Kerberos and Auth0 modes in user-facing messages.          |

## Top 15 GAP candidates (not in AGENTS.md, by confidence)
|        area        |     scope     | t1 | evd | conf |                                                 rule                                                 |
|--------------------|---------------|----|-----|------|------------------------------------------------------------------------------------------------------|
| activations        | cross-cutting | 2  | 5   | 1.0  | Review comment addressing code change.                                                               |
| providers          | area-specific | 1  | 2   | 0.75 | Add structured tracing logs for all authentication flow branches.                                    |
| models/environment | cross-cutting | 1  | 1   | 0.74 | Review comment addressing code change.                                                               |
| cli/other          | cross-cutting | 1  | 1   | 0.74 | Review comment addressing code change.                                                               |
| commands           | area-specific | 1  | 1   | 0.71 | Use select! to wait for either signal handler or CLI completion, dropping tempdir on exit.           |
| commands           | area-specific | 1  | 1   | 0.71 | Use unreachable!() for impossible states guaranteed by parser invariants.                            |
| cli/other          | area-specific | 1  | 1   | 0.71 | Clarify what 'environment's build context' means in documentation.                                   |
| commands           | area-specific | 1  | 1   | 0.71 | Frame breaking changes as benefits; explain new features' advantages to users.                       |
| commands           | area-specific | 1  | 1   | 0.71 | Add man page references or mark with TODO when feature flags gate CLI subcommands.                   |
| commands           | area-specific | 1  | 1   | 0.71 | Use precise terminology: 'targets' instead of 'artifacts' when paths are unavailable.                |
| models/environment | area-specific | 1  | 1   | 0.71 | Clarify whether bug fixes are related to the primary change; document unrelated fixes separately.    |
| models/environment | area-specific | 1  | 1   | 0.71 | Preserve force-flag behavior that resets local state to upstream even when branches are ahead.       |
| models/environment | area-specific | 1  | 1   | 0.71 | Document edge cases in comments (e.g. outputsToInstall=None) to guide future refactoring and code re |
| models/environment | area-specific | 1  | 1   | 0.71 | Document rarity of edge cases with evidence (nixpkgs stdenv behavior) to justify deliberate shortcut |
| cli/other          | area-specific | 1  | 1   | 0.71 | Add diagnostic messages for unsupported authentication modes on incompatible builds.                 |

## 'Other' bucket — high-confidence candidates (Task 8.5 source if any)
| n_high_conf_other |
|-------------------|
| 19                |
