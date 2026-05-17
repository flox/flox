# Rust PR Review Analysis — Pipeline + Outputs

## Context

This work mines 6-8 months of merged PRs from the `flox/flox` repo (a Nix-based virtual environment manager written primarily in Rust) to extract review-validated coding rules. The corpus window is 2025-09-18 → 2026-05-15 (216 Rust-touching PRs, ~1,047 line-comments). All work happens in a git worktree:
`/Users/stevemorin/c/flox_repos/flox/.claude/worktrees/rust-pr-analysis-skill`, branch `worktree-rust-pr-analysis-skill`.

## Analysis approach (locked in)

**Hybrid D + E with B as substrate:**
- (D) Comment-anchored resolution triples: every line-anchored review comment paired with the merged-final-state code at that path
- (E) AGENTS.md as ground truth: every extracted rule cross-referenced against the existing `AGENTS.md` (key-token substring overlap, ≥3 distinctive tokens ≥4 chars in any AGENTS.md section)
- (B) SQLite substrate: all data lives in `scripts/pr-analysis/data/pr_analysis.db`

Comments are classified by a Haiku-4.5 subagent (orchestrated, no API key — the parent Claude session dispatches per batch). Findings are clustered using `sentence-transformers all-MiniLM-L6-v2` embeddings at cosine threshold 0.65.

## Reviewer weighting (locked in)

| Tier | Reviewers | Weight |
|---|---|---|
| 1 (most opinionated) | `ysndr`, `mkenigs`, `dcarley` | 3.0× |
| 2 | `djsauble`, `gilmishal`, `billlevine` | 2.0× |
| 3 (other humans) | everyone else | 1.0× |
| 4 (bots) | `*[bot]`, `Copilot` | 0.0 (excluded) |

Cross-cutting findings require `cross_area_count >= 2 AND tier1_reviewer_count >= 1`.

## Hot areas (locked in)

Three subsystems get dedicated synthesis passes:
- `cli/flox/src/commands/`
- `cli/flox-rust-sdk/src/models/environment/`
- `cli/flox-rust-sdk/src/providers/`

## Pipeline (built, tested, committed)

Under `scripts/pr-analysis/`:

| Script | Purpose |
|---|---|
| `init_db.py --reset` | apply schema.sql, seed reviewer rows |
| `ingest_prs.py --since X --until Y --limit N --rust-only` | fetch PR metadata via `gh pr list` |
| `ingest_comments.py` | line-comments via `gh api pulls/:n/comments`, with noise filter + commit_id + UPSERT (not REPLACE — preserves child rows) |
| `ingest_review_summaries.py` | review-summary bodies |
| `ingest_pr_comments.py` | top-level issue comments |
| `ingest_final_code.py` | ~40-line snippet at merge_commit_sha, cached per file |
| `ingest_thread_resolution.py` | GraphQL `pulls/:n/reviewThreads` for resolution state |
| `audit_coverage.py [--ingest-only]` | 4 invariants: ID parity with GitHub, no orphans, every non-noise/non-bot classified, no `cli/*` falls into `area='other'` |
| `classify_via_subagent.py prepare/ingest --batch-size 15` | subagent-orchestrated Haiku classifier with thread_resolved hint + prompt_hash versioning |
| `aggregate_findings.py` | MiniLM embedding clustering at cosine 0.65 + key-token AGENTS.md matching |
| `build_dashboard.py` | regenerates `rust-pr-analysis-dashboard-01.html` |
| `build_noise_deep_dive.py` | regenerates `rust-pr-analysis-noise-deep-dive-01.html` |

Library modules under `scripts/pr-analysis/lib/`:
- `db.py`, `gh.py`, `reviewers.py`, `areas.py` (path → area mapping, including `assets/environment-interpreter/` → `activations`), `taxonomy.py` (15 entries seeded from AGENTS.md sections), `noise_filter.py` (URL-only, suggestion-only, lgtm/praise/nit prefix, max 40-char body for praise/nit), `classify_helpers.py` (SYSTEM_PROMPT, build_user_prompt, taxonomy_block, parse, prompt_hash).

Schema includes: `pr`, `pr_file`, `line_comment` (with `is_noise`, `commit_id`, `thread_resolved`, `thread_resolved_by`), `comment_final_code`, `classification` (with `prompt_hash`, `was_addressed`), `finding` (with `scope`, `in_agents_md`, `agents_md_section`, `confidence_score`), `review_summary`, `pr_comment`, `reviewer`, `synthesis_log`.

## Calibration journey (already done — don't redo)

3 pilot iterations on 13 PRs, plus full Task 8 corpus:

| Metric | Iter 1 | Iter 2 | Iter 3 | Task 8 |
|---|---|---|---|---|
| `other` classifications % | 54% | 44% | 31% | 39% |
| Findings with evd > 1 | 4% | 8% | 17% | (see Task 8 dashboard) |
| `in_agents_md=1` | 100% (useless-true) | 0% (useless-false) | 73% (defensible) | 73% |
| `was_addressed=NULL` | ~38 | ~38 | 11 | similar |

Key calibration choices that produced the defensible state:
- Noise filter at ingest (drops ~8% of comments structurally)
- Cluster threshold = MiniLM cosine 0.65
- AGENTS.md matching = key-token substring (≥3 distinctive tokens ≥4 chars)
- `thread_resolved` passed as hint to classifier
- Batch size = 15 (eliminated Haiku self-doubt retries)
- `INSERT … ON CONFLICT(id) DO UPDATE` in `ingest_comments` (preserves children)

## Outputs already produced

1. **Plan**: `docs/superpowers/plans/2026-05-16-flox-rust-pr-analysis-skill.md`
2. **Journey log** (md + html): `rust-pr-analysis-jouney-01.{md,html}` — chronological log of 66 events + iteration comparison + lessons
3. **Main dashboard**: `rust-pr-analysis-dashboard-01.html` — 86 KB, all SVG, covers: KPIs, PRs/commits/LOC over time, top reviewers/authors/committers, reviewer × area heatmap, area/taxonomy segmentation, 2 cross-cutting findings, was_addressed × thread_resolved cross-tab
4. **Noise deep-dive**: `rust-pr-analysis-noise-deep-dive-01.html` — 90 KB, forensic breakdown of which regex catches what (45 suggestion-only, 22 lgtm, 16 url, 4 praise/nit), tier rates (T1 5.4%, T2 18.6%, T3 9.4%), stylistic taxonomy table (163 rules, 54 gap candidates), recommendation: keep filter as-is

## Remaining work — TWO skill outputs wanted

### Output A: `flox-rust-review` skill (the main cross-cutting skill)

Path: `.claude/skills/flox-rust-review/SKILL.md`

Source data: `finding` table, `scope='cross-cutting'` rows. With current calibration, expect a small number of cross-cutting findings (~2 at current threshold; could be ~13 if cluster threshold dropped to 0.55). Either way, this skill should focus on the highest-confidence, most-cross-area rules.

Content structure:
- Frontmatter with skill name + description
- Opening paragraph summarizing what reviewers consistently enforce
- Rules grouped by taxonomy id (in title case as section headings)
- Each rule: **bold one-line rule**, 1-2 sentence rationale, "Evidence:" with up to 3 PR numbers
- Closing "Where to look first" section listing the three hot areas

Cite PR numbers as `#NNNN` linking to `https://github.com/flox/flox/pull/NNNN`.

### Output B: `flox-rust-stylistic-conventions` skill (NEW — extracted from analysis)

Path: `.claude/skills/flox-rust-stylistic-conventions/SKILL.md`

Source data: classifications where `taxonomy IN ('naming', 'formatting-style', 'imports', 'control-flow', 'logging-tracing')` AND `confidence >= 0.6` — 163 total in the current corpus, 54 of which are NOT covered by AGENTS.md (the gap candidates).

Why a separate skill: the noise deep-dive surfaced that stylistic rules are systematically under-codified — 33% gap rate vs 27% repo-wide. The team enforces these in review but hasn't written them down. They're the highest-teaching-value, lowest-architectural-overhead rules in the corpus.

Top gap candidates to lead with:
1. Use `formatdoc!` for multiline strings instead of `format!` with backslash continuations
2. Remove `dbg!` macros before submission
3. No `test_` prefix on test functions (`#[test]` already marks them)
4. Use `pub(super)` or `pub(crate)` instead of bare `pub` for internal items
5. Follow `str_to_x` naming for parser-style functions

Content structure: same as Output A, but explicitly scoped to stylistic taxonomies. Add a "Stylistic rules already in AGENTS.md" section linking back so this skill complements rather than duplicates the main AGENTS.md content.

### Also produce: per-area `CLAUDE.md` files

In addition to the two skills, write area-specific `CLAUDE.md` files at:
- `cli/flox/src/commands/CLAUDE.md`
- `cli/flox-rust-sdk/src/models/environment/CLAUDE.md`
- `cli/flox-rust-sdk/src/providers/CLAUDE.md`

Each summarizes area-specific findings + references the parent skills for cross-cutting rules. Source: `finding` table filtered by area (matching the prefix mapping in `lib/areas.py`).

### Also produce: gap report

Path: `scripts/pr-analysis/findings/gap-report.md`

Source data: findings where `in_agents_md=0` (the 27% of all findings + 33% of stylistic findings). Three sections:
1. Proposed new AGENTS.md rules (each with evidence PR + suggested section)
2. Existing AGENTS.md rules still actively enforced (high evidence count despite already being documented — may need clearer examples)
3. Reviewer-voice notes — one paragraph each on ysndr / mkenigs / dcarley summarizing the feedback patterns recurring in their reviews

## Synthesis approach

Use Claude Sonnet 4.6 (one synthesis call per artifact, capped at ~4000 output tokens) to draft each skill / CLAUDE.md / gap report from a JSON payload of the relevant `finding` rows. Store each synthesis run in the `synthesis_log` table for auditability. Cite only PR numbers that actually exist in the `finding.evidence_pr_numbers` JSON arrays — never invent citations.

## Discipline

- All Python is pure stdlib + `uv` script headers (no global pip installs except `sentence-transformers` in `aggregate_findings.py`)
- All HTML reports are self-contained (inline SVG, no JS libraries, no CDN)
- Test functions use `test_` prefix (Python convention; AGENTS.md's "no prefix" rule applies to Rust where `#[test]` identifies them)
- Commits use Conventional Commits; pre-commit hooks must pass (via `nix develop -c git commit ...` if `IN_NIX_SHELL` is unset)
- Schema versioning via `prompt_hash` on classifications — never silently mix classifications from different prompt/taxonomy versions
- Worktree shares `.git` with the main repo; `git log main` works from anywhere
