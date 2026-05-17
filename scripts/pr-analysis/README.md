# pr-analysis pipeline

Extracts review-validated coding rules from 6 months of merged PRs in
`flox/flox`. Produces one cross-cutting skill plus three area-specific
`CLAUDE.md` files plus a gap report against `AGENTS.md`.

## Prereqs

- `gh` CLI authenticated for `flox/flox`
- `uv` on PATH
- `ANTHROPIC_API_KEY` exported

## Run order

### Pilot (Task 7): 15-PR calibration run

```bash
uv run scripts/pr-analysis/init_db.py --reset
uv run scripts/pr-analysis/ingest_prs.py --since 2025-11-16 --limit 25
uv run scripts/pr-analysis/ingest_comments.py
uv run scripts/pr-analysis/ingest_final_code.py
uv run scripts/pr-analysis/audit_coverage.py --ingest-only
uv run scripts/pr-analysis/classify_comments.py --concurrency 4
uv run scripts/pr-analysis/audit_coverage.py
uv run scripts/pr-analysis/aggregate_findings.py
# then build the retro digest per Task 7 Step 7
```

### Full corpus (Task 8 onward): all 157 Rust PRs

```bash
uv run scripts/pr-analysis/init_db.py --reset
uv run scripts/pr-analysis/ingest_prs.py --since 2025-11-16
uv run scripts/pr-analysis/ingest_comments.py
uv run scripts/pr-analysis/ingest_final_code.py
uv run scripts/pr-analysis/audit_coverage.py --ingest-only
uv run scripts/pr-analysis/classify_comments.py --concurrency 8
uv run scripts/pr-analysis/audit_coverage.py
uv run scripts/pr-analysis/aggregate_findings.py
uv run scripts/pr-analysis/synthesize_cross_cutting.py
uv run scripts/pr-analysis/synthesize_area.py --area commands
uv run scripts/pr-analysis/synthesize_area.py --area models/environment
uv run scripts/pr-analysis/synthesize_area.py --area providers
uv run scripts/pr-analysis/synthesize_gap_report.py
```

Every ingest script is idempotent (`INSERT OR REPLACE`). Stages can resume within a run. Across runs, `init_db.py --reset` is the clean-slate switch.
