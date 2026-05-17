# Rust PR Analysis Journey — Log 01

**Session:** `rust-pr-analysis-skill`
**Worktree:** `/Users/stevemorin/c/flox_repos/flox/.claude/worktrees/rust-pr-analysis-skill`
**Branch:** `worktree-rust-pr-analysis-skill`
**Started:** 2026-05-16 (plan file dated 2026-05-16)
**Latest commit at log time:** `c49aee683` (iter-3 retro digest)
**Total commits during session:** 17

---

## Overall goal

Mine 6 months of merged PRs touching Rust files in `flox/flox` to extract review-validated coding rules, materialize them as one cross-cutting skill plus three area-specific `CLAUDE.md` files (`commands/`, `models/environment/`, `providers/`), and produce a gap report against the existing `AGENTS.md`.

---

## Chronological log (one line per event)

1. Asked whether a worktree existed at `.claude/worktrees/rust-pr-analysis-skill` — answered no, directory empty
2. User asked to use `EnterWorktree` skill to create it
3. Created worktree on branch `worktree-rust-pr-analysis-skill` rooted at `origin/main`
4. User wanted to explore approaches for extracting review-validated coding rules from 6 months of PRs
5. Presented 5 approaches (A: gh JSONL, B: SQLite warehouse, C: SQLite+LLM classify, D: comment+resolution triples, E: AGENTS.md as truth) with recommendation: hybrid D+E with B as substrate
6. User asked for a volume check
7. Volume check ran: 249 merged PRs, 157 Rust-touching, ~1,470 projected line-comments, top 6 reviewers cover 95% of signal, hot areas commands (207) / models/environment (115) / providers (103)
8. Refined recommendation to no-sampling, reviewer-weighted, LLM-judged resolution against merged final state
9. User confirmed approach, requested detailed plan with tier-1 (`ysndr`, `mkenigs`, `dcarley` = 3.0×) and tier-2 (`djsauble`, `gilmishal`, `billlevine` = 2.0×) weighting, hot-area weighting, area-specific CLAUDE.md outputs, intermediate findings table
10. Wrote 11-task plan to `docs/superpowers/plans/2026-05-16-flox-rust-pr-analysis-skill.md`
11. User added a constraint: build infra → partial extraction on 10-15 PRs → reflect/retro → full run; ensure every comment reviewed
12. Restructured plan to 13 tasks: 1-6 build with smoke runs, 7 pilot+retro (HUMAN-IN-LOOP), 8 full corpus, 9-13 syntheses + final review
13. User chose Subagent-Driven Development (option 1) for execution
14. Created TaskCreate entries for 13 plan tasks
15. **Task 1 (bootstrap)** dispatched — schema.sql, lib/{db,reviewers,areas,taxonomy}.py, init_db.py, tests
16. Task 1 implementer reported DONE_WITH_CONCERNS — fixed plan bugs: `conftest.py parents[2]→parents[3]`, added `pytest.ini` for test discovery, added `.gitignore` (commit `f0e2a435b`, 18 tests)
17. Spec reviewer ✅ — all 18 plan steps verified byte-for-byte
18. Code-quality reviewer: 2 Important + 8 Minor — pytest.ini foot-gun, UTC timestamp inconsistency, etc.
19. Applied 7 fixes — renamed tests with `test_` prefix, UTC, .db-journal cleanup, prefix-map test, HOT_AREAS test, README API-cost notes (commit `a6b4fd046`, 20 tests)
20. Re-review ✅ — Task 1 closed
21. **Task 2 (gh wrapper + PR ingest)** dispatched
22. Task 2 implementer reported DONE_WITH_CONCERNS — found two plan bugs: `mergeCommitOid` doesn't exist on `gh pr list --json` (used `mergeCommit.oid`), `author.type` doesn't exist (left as broken default "User") (commit `14a94d6e8`, 23 tests)
23. Spec reviewer: ✅ — confirmed `pr.author_type` never read downstream, cosmetic-only
24. Code-quality reviewer: 3 Important + 5 Minor
25. Applied 6 fixes — bot detection via `is_bot`, removed redundant `sys.path.insert`, extracted `_run` helper in `lib/gh.py`, `GhError` symmetry, `--all` help text (commit `8fc66aae0`, 23 tests)
26. **Task 3 (line-comment ingest)** dispatched — DONE with 0 comments ingested (both bot-authored smoke PRs had no review comments) (commit `e5ec264a8`, 24 tests)
27. **Task 4 (final-code snippet extraction)** dispatched — DONE with `extract_window` unit tests (commit `565fcadaf`, 28 tests)
28. **Task 5 (LLM classifier)** dispatched — `ANTHROPIC_API_KEY` not exported; built code with early-return for empty DB, parser tests (commit `50f9e0ef8`, 33 tests)
29. **Task 6 (findings aggregator)** dispatched — pure clustering + scoring functions with unit tests (commit `349748ad7`, 37 tests)
30. **Task 7 Steps 1-4 (pilot ingest)** dispatched — built `audit_coverage.py`, reset DB, ingested 25 PRs → 13 Rust-touching, 115 line-comments; audit passed
31. Pilot ingest surfaced 6 comments in `area='other'` under `assets/environment-interpreter/` (PR #4233 activation hint series); flagged for retro decision
32. User questioned the SDK-binary approach for classifier — proposed using a subagent (Haiku model) instead
33. User answered: add `assets/environment-interpreter/ → activations` mapping now, proceed with pilot, switch to subagent classifier path
34. Added prefix-map entry + test, re-ingested comments to refresh area tagging (commit `c1d471ddc`, 38 tests)
35. Built `classify_via_subagent.py` + `lib/classify_helpers.py` (extracted shared prompt code so the subagent path is independent of `anthropic` Python dep) (commit `2db113b1`, 43 tests); prepared 4 batches of 30 comments each
36. **Pilot iteration 1 classification** — dispatched 4 parallel Haiku subagents; batch 1 (70KB) failed because the model self-doubted the Read tool; batches 2, 3, 4 (58/66/74KB) succeeded on first try
37. Retried batch 1 with a tighter prompt that told the model not to second-guess Read — succeeded immediately
38. Ingested 115 classifications; post-classify audit FAILED — `comment_final_code` rows had been wiped during the area-fix re-ingest by SQLite's `ON DELETE CASCADE` on `INSERT OR REPLACE`
39. Re-ran `ingest_final_code.py` to refetch snippets; audit OK
40. Aggregated: 51 findings, 49 with evd=1, 100% tagged `in_agents_md=1` (heuristic was just substring-matching taxonomy section names against AGENTS.md), `other` bucket at 62/115 = 54% (mostly URL-only / suggestion-block / praise comments)
41. Surfaced pilot iter-1 retro to user with three calibration problems: efficiency (54% wasted on noise), accuracy (clustering and AGENTS.md heuristics both broken), error-reduction (batch-1 self-doubt + schema cascade gotcha)
42. User picked all 4 recommendations: add noise filter at ingest, fix both heuristics (clustering threshold 0.6→0.35; AGENTS.md text-similarity comparison), schema-preserving UPSERT in `ingest_comments`, batch size 30→15 + broader `imports` taxonomy description
43. Applied 5 calibration changes — `lib/noise_filter.py` (URL/suggestion/praise/lgtm regexes) + `is_noise` column + UPSERT semantics + `CLUSTER_THRESHOLD=0.35` + `agents_md_coverage` Jaccard rewrite + broader `imports` description (commit `781dbf26d`, 55 tests)
44. User asked two design questions: (Q1) plan a conditional `other`-bucket re-classification pass after the full corpus; (Q2) audit whether we're storing enough information
45. Presented storage gap audit: 4 high-value adds (review-summary bodies, thread resolution state from GraphQL, `commit_id` at comment time, classifier prompt/taxonomy hash) + 2 optional (issue comments, synthesizer raw outputs)
46. User picked all: Task 8.5 conditional re-pass + all 4 core adds + both optional adds + Task 7.5 re-pilot-first iteration scope
47. **Task 7.5a (schema + REST ingests)** dispatched — added schema columns (`commit_id`, `original_commit_id`, `thread_resolved`, `thread_resolved_by`, `thread_resolved_at`, `prompt_hash`), new tables (`review_summary`, `pr_comment`, `synthesis_log`), new scripts `ingest_review_summaries.py` + `ingest_pr_comments.py`, fixture tests (commit `f5963bdb5`, 57 tests)
48. **Task 7.5b (GraphQL + prompt hash)** dispatched — new `ingest_thread_resolution.py` using `gh api graphql` for `reviewThreads`, `prompt_hash` helper in `classify_helpers`, prompt_hash propagated through prepare/ingest, informational invariants added to audit (commit `80a712941`, 63 tests)
49. **Re-pilot iteration 2 ingest** — reset DB, ingested 13 PRs, 115 line-comments, **21 noise-filtered (18%)**, 94 to classify, 7 review summaries, 29 issue comments, 115 thread resolutions populated (72 resolved / 43 unresolved); audit OK
50. Prepared 7 batches of 15 comments each (smaller batches to avoid Haiku self-doubt)
51. Dispatched 7 parallel Haiku subagents — **all 7 succeeded on first try** (no retries needed)
52. Ingested 94 classifications: `other` dropped to 41 (44%), `imports` still 0 hits, `provider-traits`/`manifest-usage`/etc still 0
53. Aggregated: 49 findings, 4 evd>1 (8% — marginal improvement), **0% `in_agents_md`=1** (heuristic swung from useless-true to useless-false because rules use imperative vocab and AGENTS.md uses descriptive prose at Jaccard 0.25)
54. Surfaced iter-2 to user with two issues: AGENTS.md matching at 0% needs recalibration, clustering still mostly evd=1
55. User picked: lower threshold + key-token substring for AGENTS.md, **switch clustering to embedding similarity** (the rabbit hole), pass `thread_resolved` as classifier hint, re-pilot iter 3
56. Applied 3 calibration changes — `agents_md_coverage` rewritten as key-token substring (≥3 distinctive tokens ≥4 chars overlapping any AGENTS.md section), `cluster_rule_statements` rewritten with MiniLM `all-MiniLM-L6-v2` embeddings at cosine 0.65, thread_resolved/thread_resolved_by passed into batch JSON and system prompt (commit `bfc871482`, 67 tests; embedding-test downloads ~80MB MiniLM model on first run)
57. **Re-pilot iteration 3 classification** — wiped existing 94 classifications, re-prepared 7 batches with new `prompt_hash` and `thread_resolved` fields, dispatched 7 parallel Haiku subagents — all 7 succeeded
58. Ingested 94 classifications: `other` dropped to 29 (31% — from 54%), `imports` got first hit, `was_addressed=NULL` dropped from ~38 to 11 (thread_resolved hint working)
59. Aggregated: 52 findings, **9 evd>1 (17% — quadrupled from iter 1)**, **38 in_agents_md=1 (73% — defensible)**, was_addressed × thread_resolved cross-tab shows 76/94 on the diagonal (81% agreement between LLM judgment and GitHub thread state)
60. Iter-3 retro digest committed (commit `c49aee683`, 105 lines) — three iterations of metrics shown side-by-side
61. Surfaced iter-3 to user as ready for Task 8 full corpus (with two residual signals: 43/52 findings still evd=1, AGENTS.md monolithic "Conventions" section dominates matches)
62. User: one more pilot — different time window (originally proposed 1 month back from 6mo cutoff) to validate calibration generalizes beyond the recent activation-hint-dominated sample
63. Dispatched a subagent to add `--until` flag to `ingest_prs.py` and run iter-4 pilot on 2025-10-16..2025-11-15 window
64. User interrupted: widen the window to 2025-09-16..2025-11-15 (2 months instead of 1)
65. User renamed session to `rust-pr-analysis-skill`
66. User asked to pause analysis and produce this journey log (markdown + HTML)

---

## Pipeline architecture (current state)

```
scripts/pr-analysis/
├── schema.sql                          # 10 tables incl. line_comment, review_summary,
│                                        # pr_comment, comment_final_code, classification,
│                                        # finding, synthesis_log
├── init_db.py                          # --reset wipes DB, --wal/-shm/-journal cleanup
├── ingest_prs.py                       # --since/--until/--limit/--rust-only/--all
├── ingest_comments.py                  # line-comments, noise filter, commit_id, UPSERT
├── ingest_review_summaries.py          # pulls/:n/reviews body text
├── ingest_pr_comments.py               # issues/:n/comments top-level discussion
├── ingest_final_code.py                # ~40-line snippet at merge_commit_sha, cached
├── ingest_thread_resolution.py         # GraphQL reviewThreads, applies to line_comment
├── audit_coverage.py                   # 4 invariants + informational counts
├── classify_via_subagent.py            # prepare/ingest, subagent-orchestrated (no API key needed)
├── classify_comments.py                # legacy Anthropic-SDK path (kept for users with key)
├── aggregate_findings.py               # MiniLM embedding clustering @ cosine 0.65,
│                                        # key-token AGENTS.md substring matching
├── lib/
│   ├── db.py                           # connection factory, transaction ctx manager
│   ├── reviewers.py                    # Tier 1/2/3/Bot weights
│   ├── areas.py                        # path → area mapping, HOT_AREAS, prefix-order test
│   ├── taxonomy.py                     # 15 entries seeded from AGENTS.md sections
│   ├── classify_helpers.py             # SYSTEM_PROMPT, build_user_prompt, taxonomy_block,
│   │                                    # parse, prompt_hash (SHA256)
│   ├── noise_filter.py                 # URL-only / suggestion-only / lgtm / praise regexes
│   └── gh.py                           # _run, run_json, paginate_jsonl wrappers
├── tests/                              # 67 tests, ~7s wall time after first run
└── findings/
    └── pilot-retro.md                  # iter-3 retro digest (latest)
```

---

## Iteration comparison

| Metric                          | Iter 1     | Iter 2          | Iter 3         |
|---------------------------------|------------|-----------------|----------------|
| Window                          | last ~2 mo | last ~2 mo      | last ~2 mo     |
| PRs ingested                    | 13         | 13              | 13             |
| Comments raw                    | 115        | 115             | 115            |
| Comments to classify            | 115        | 94 (noise filt) | 94             |
| Batches                         | 4 × 30     | 7 × 15          | 7 × 15         |
| Batch retries                   | 1/4 (25%)  | 0/7             | 0/7            |
| `other` classifications         | 62 (54%)   | 41 (44%)        | **29 (31%)**   |
| Findings total                  | 51         | 49              | 52             |
| Findings with evd > 1           | 2 (4%)     | 4 (8%)          | **9 (17%)**    |
| `in_agents_md=1`                | 51 (100%)  | 0 (0%)          | **38 (73%)**   |
| `was_addressed=NULL`            | ~38        | ~38             | **11**         |
| `imports` taxonomy hits         | 0          | 0               | **1**          |

---

## Pending decisions / next steps

1. **Iter-4 pilot** on the 2025-09-16..2025-11-15 window (paused mid-dispatch when user asked for this log)
2. **Task 8** — full 157-PR corpus run end-to-end with calibrated pipeline
3. **Task 8.5** — conditional `other`-bucket re-classification (runs only if `other` > 25%)
4. **Tasks 9-13** — findings checkpoint, cross-cutting synthesis, three area `CLAUDE.md` files, gap report, final review

---

## Commits during session (17 total)

| # | SHA         | Description                                                                |
|---|-------------|----------------------------------------------------------------------------|
| 1 | `f0e2a435b` | Task 1 bootstrap (initial)                                                 |
| 2 | `a6b4fd046` | Task 1 code-review feedback (test prefixes, UTC, etc)                      |
| 3 | `14a94d6e8` | Task 2 gh wrapper + PR ingest (initial)                                    |
| 4 | `8fc66aae0` | Task 2 code-review feedback (bot detection, _run helper)                   |
| 5 | `e5ec264a8` | Task 3 line-comment ingest                                                 |
| 6 | `565fcadaf` | Task 4 snippet extraction                                                  |
| 7 | `50f9e0ef8` | Task 5 LLM classifier (Anthropic SDK path)                                 |
| 8 | `349748ad7` | Task 6 findings aggregator                                                 |
| 9 | `c1d471ddc` | audit_coverage script + activations area mapping fix                       |
|10 | `2db113b1`  | classify_via_subagent.py + extracted lib/classify_helpers.py               |
|11 | `781dbf26d` | Task 7 retro calibration: noise filter, clustering, AGENTS.md text, UPSERT |
|12 | `f5963bdb5` | Task 7.5a storage additions (REST): reviews, issue comments, commit_id     |
|13 | `80a712941` | Task 7.5b GraphQL thread resolution + prompt hash                          |
|14 | `9f1151fc8` | Iter-2 retro digest                                                        |
|15 | `bfc871482` | Iter-3 calibration: key-token AGENTS.md, MiniLM embeddings, thread hint    |
|16 | `c49aee683` | Iter-3 retro digest                                                        |
|17 |  *current*  | (no commit yet for iter-4)                                                 |

---

## Key lessons surfaced during the session

1. **Plan bugs are normal**, especially around external APIs whose schemas you don't have authoritative documentation for. `gh pr list --json` doesn't expose `mergeCommitOid` or `author.type` — both were assumed in the original plan. Implementer subagents that adapt cleanly (and flag the deviation) are more valuable than ones that blindly follow.
2. **Heuristic calibration is a swing problem.** AGENTS.md matching went 100% (useless-true) → 0% (useless-false) → 73% (defensible) across three iterations of the same idea expressed three different ways (substring match on section names → Jaccard against full text → key-token substring against per-section text).
3. **Embeddings are the right tool when one-line rules are involved.** Even paraphrased duplicates only Jaccard at ~0.3-0.4; MiniLM cosine puts them at ~0.55-0.65. Threshold choice is empirical; 0.65 is defensible but may need adjustment per corpus.
4. **`INSERT OR REPLACE` with `ON DELETE CASCADE` is a footgun.** SQLite implements REPLACE as DELETE+INSERT, which cascades to children. Fix is `INSERT … ON CONFLICT(id) DO UPDATE SET …`.
5. **Subagent self-doubt is real.** A Haiku subagent failed on a 70KB Read claiming "exceeds token limits" while three sibling agents handled 58-74KB files fine. Smaller batches (~15 items instead of 30) eliminated the trigger entirely.
6. **Calibrating on one window risks overfitting.** The recent 15-PR window was 52% `activations` (recency bias from one fix series). Iter-4 on a different window is checking whether the pipeline generalizes.
