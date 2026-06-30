# Pilot iteration 4 — cross-window validation

**Window:** 2025-10-15 → 2025-11-15 (6-7 months back). Different from iter3 which sampled 2026-04-15 → 2026-05-15 (last month).

## Iter 3 vs Iter 4 comparison

| Metric | Iter 3 (recent) | Iter 4 (6-7mo back) |
|---|---|---|
| PRs | 13 | 36 |
| Comments classified | 94 | 78 |
| `other` % | 31% | **51%** |
| Findings total | 52 | 36 |
| Findings evd>1 | 9 (17%) | 2 (6%) |
| in_agents_md | 73% | 81% |
| was_addressed=NULL | 11 | 30 |
| threads_resolved | 72/115 (63%) | 18/80 (23%) |

## Iter 4 taxonomy distribution (notice the diversity)
|       taxonomy       | n  | avg_conf |
|----------------------|----|----------|
| other                | 40 | 0.17     |
| testing              | 9  | 0.7      |
| error-handling       | 8  | 0.75     |
| naming               | 7  | 0.82     |
| semantic-correctness | 7  | 0.71     |
| control-flow         | 3  | 0.73     |
| type-safety          | 2  | 0.7      |
| manifest-usage       | 1  | 0.9      |
| provider-traits      | 1  | 0.72     |

## Iter 4 top findings — high quality substantive rules
|        area        | t1 | evd | md | conf |                                                 rule                                                 |
|--------------------|----|-----|----|------|------------------------------------------------------------------------------------------------------|
| models/environment | 1  | 2   | 1  | 0.75 | Use separate error variants or wrap in io::Error with ErrorKind::Other instead of string merging.    |
| activations        | 1  | 2   | 1  | 0.75 | Use variable names that clarify data type (e.g. watchdog_bin for path).                              |
| models/environment | 1  | 1   | 1  | 0.71 | Wrap errors consistently across error variants to enable product-level message formatting.           |
| models/environment | 1  | 1   | 1  | 0.71 | Use correct error enum variants for clone vs fetch operations; explain refactoring rationale.        |
| cli/other          | 1  | 1   | 1  | 0.71 | Extend error enums instead of erasing type information; use Box<Error> over string compression.      |
| cli/other          | 1  | 1   | 1  | 0.71 | Use Box<dyn std::error::Error + Send + Sync> when only the message is needed, but keep error handlin |
| other              | 1  | 1   | 0  | 0.71 | Verify CI checks catch generated schema drift in automated builds.                                   |
| commands           | 1  | 1   | 1  | 0.71 | Add unit tests by accepting &mut impl Write to enable buffer-based testing.                          |
| commands           | 1  | 1   | 1  | 0.71 | Write tests to verify concurrent locking behavior and environment generation switching logic.        |
| commands           | 1  | 1   | 1  | 0.71 | Use descriptor fields rather than reconstructing path from catalog and attr_path.                    |
| commands           | 1  | 1   | 1  | 0.71 | Document when an intermediate type will be converted; avoid misleading temporary state.              |
| commands           | 1  | 1   | 1  | 0.71 | Avoid unnecessary operations before checking guard conditions; defer expensive calls until required. |
| commands           | 1  | 1   | 1  | 0.71 | Position frequently-used flags at top level, not nested in subcommand variants, for better UX.       |
| models/environment | 1  | 1   | 1  | 0.71 | Understand error handling logic before refactoring; ensure all cases properly handled.               |
| models/environment | 1  | 1   | 0  | 0.71 | Acknowledge manual verification; document trade-offs between flakiness and test coverage.            |

## Iter 4 gap candidates (NOT in AGENTS.md)
|        area        | t1 | evd | conf |                                             rule                                             |
|--------------------|----|-----|------|----------------------------------------------------------------------------------------------|
| other              | 1  | 1   | 0.71 | Verify CI checks catch generated schema drift in automated builds.                           |
| models/environment | 1  | 1   | 0.71 | Acknowledge manual verification; document trade-offs between flakiness and test coverage.    |
| core               | 1  | 1   | 0.71 | Rename modules to reflect their semantic purpose (context vs data).                          |
| models/environment | 1  | 1   | 0.51 | Clarify lock lifetime requirements; ensure resource guards held for entire critical section. |
| core               | 0  | 1   | 0.21 | Document whether PathBuf represents absolute or relative paths in type or comment.           |
| activations        | 0  | 1   | 0.21 | Preserve stderr/stdout interleaving order to maintain context in errors.                     |
| activations        | 0  | 1   | 0.11 | Preserve intentional verbosity mappings when refactoring log levels.                         |
