# pr-analysis pipeline

Extracts review-validated coding rules from 6 months of merged PRs in
`flox/flox`. Produces one cross-cutting skill plus three area-specific
`CLAUDE.md` files plus a gap report against `AGENTS.md`.

## Prereqs

- `gh` CLI authenticated for `flox/flox`
- `uv` on PATH
- `ANTHROPIC_API_KEY` exported

## Status

This directory is built up across multiple tasks. **Task 1 ships only `init_db.py`** (plus the schema, lib modules, and tests). The other scripts referenced in the run order below — `ingest_prs.py`, `ingest_comments.py`, `ingest_final_code.py`, `classify_comments.py`, `audit_coverage.py`, `aggregate_findings.py`, `synthesize_*.py` — land in subsequent tasks. Running the full pipeline before later tasks complete will error with `No such file or directory`.

## Cost expectations

Classifying the full corpus (Task 8) runs ~1,400 line-comments through Claude Haiku 4.5; expect **~$1–3** in Anthropic API spend per full run. The pilot (Task 7) classifies ~100–200 comments and costs **~$0.30 per iteration**. The synthesis stages (Tasks 10–12) use Claude Sonnet 4.6 and add **~$1.50** combined. GitHub API calls are free under the standard authenticated rate limit.

## Run order

### Pilot (Task 7): 15-PR calibration run

```bash
uv run scripts/pr-analysis/init_db.py --reset
uv run scripts/pr-analysis/ingest_prs.py --since 2025-11-16 --limit 25
uv run scripts/pr-analysis/ingest_comments.py
uv run scripts/pr-analysis/ingest_review_summaries.py
uv run scripts/pr-analysis/ingest_pr_comments.py
uv run scripts/pr-analysis/ingest_final_code.py
uv run scripts/pr-analysis/ingest_thread_resolution.py    # added in 7.5b
uv run scripts/pr-analysis/audit_coverage.py --ingest-only
uv run scripts/pr-analysis/classify_via_subagent.py prepare --batch-size 15
# controller dispatches Haiku subagents per batch
uv run scripts/pr-analysis/classify_via_subagent.py ingest --in-dir /tmp/pilot_classify
uv run scripts/pr-analysis/audit_coverage.py
uv run scripts/pr-analysis/aggregate_findings.py
# then build the retro digest per Task 7 Step 7
```

### Full corpus (Task 8 onward): all 157 Rust PRs

```bash
uv run scripts/pr-analysis/init_db.py --reset
uv run scripts/pr-analysis/ingest_prs.py --since 2025-11-16
uv run scripts/pr-analysis/ingest_comments.py
uv run scripts/pr-analysis/ingest_review_summaries.py
uv run scripts/pr-analysis/ingest_pr_comments.py
uv run scripts/pr-analysis/ingest_final_code.py
uv run scripts/pr-analysis/ingest_thread_resolution.py    # added in 7.5b
uv run scripts/pr-analysis/audit_coverage.py --ingest-only
uv run scripts/pr-analysis/classify_via_subagent.py prepare --batch-size 15
# controller dispatches Haiku subagents per batch
uv run scripts/pr-analysis/classify_via_subagent.py ingest --in-dir /tmp/full_classify
uv run scripts/pr-analysis/audit_coverage.py
uv run scripts/pr-analysis/aggregate_findings.py
uv run scripts/pr-analysis/synthesize_cross_cutting.py
uv run scripts/pr-analysis/synthesize_area.py --area commands
uv run scripts/pr-analysis/synthesize_area.py --area models/environment
uv run scripts/pr-analysis/synthesize_area.py --area providers
uv run scripts/pr-analysis/synthesize_gap_report.py
```

Every ingest script is idempotent (`INSERT OR REPLACE`). Stages can resume within a run. Across runs, `init_db.py --reset` is the clean-slate switch.
