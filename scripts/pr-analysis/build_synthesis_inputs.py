#!/usr/bin/env python3
"""Build per-target synthesis input JSON files for Sonnet synthesizers.

Reads findings + evidence from data/pr_analysis.db and writes JSON files to
/tmp/synthesis/, one per synthesis target. Each Sonnet synthesizer reads its
file and produces one markdown deliverable.
"""
from __future__ import annotations

import json
import sqlite3
from pathlib import Path
from typing import Any

from lib.db import connect

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
AGENTS_MD_PATH = REPO_ROOT / "AGENTS.md"
OUTPUT_DIR = Path("/tmp/synthesis")
TRUNCATE_LIMIT = 1000

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def truncate(s: str | None, limit: int = TRUNCATE_LIMIT) -> str | None:
    if s is None:
        return None
    if len(s) <= limit:
        return s
    return s[:limit] + "[...]"


def parse_json_list(s: str | None) -> list[Any]:
    if not s:
        return []
    try:
        v = json.loads(s)
        if isinstance(v, list):
            return v
    except json.JSONDecodeError:
        pass
    return []


def finding_to_dict(row: sqlite3.Row) -> dict[str, Any]:
    return {
        "id": row["id"],
        "rule_statement": row["rule_statement"],
        "taxonomy": row["taxonomy"],
        "area": row["area"],
        "scope": row["scope"],
        "tier1_reviewer_count": row["tier1_reviewer_count"],
        "tier2_reviewer_count": row["tier2_reviewer_count"],
        "total_evidence_count": row["total_evidence_count"],
        "evidence_comment_ids": parse_json_list(row["evidence_comment_ids"]),
        "evidence_pr_numbers": parse_json_list(row["evidence_pr_numbers"]),
        "areas_seen": parse_json_list(row["areas_seen"]),
        "confidence_score": row["confidence_score"],
        "in_agents_md": row["in_agents_md"],
        "agents_md_section": row["agents_md_section"],
        "acceptance_rate": row["acceptance_rate"],
    }


def fetch_evidence(
    conn: sqlite3.Connection,
    comment_ids: list[int],
    max_blocks: int = 2,
) -> list[dict[str, Any]]:
    if not comment_ids:
        return []
    # Take first N comment_ids (preserves order in evidence_comment_ids).
    selected = comment_ids[:max_blocks]
    placeholders = ",".join("?" for _ in selected)
    rows = conn.execute(
        f"""
        SELECT lc.id, lc.pr_number, lc.path, lc.line, lc.author,
               lc.reviewer_tier, lc.body, lc.diff_hunk,
               cfc.final_code_snippet
        FROM line_comment lc
        LEFT JOIN comment_final_code cfc ON cfc.comment_id = lc.id
        WHERE lc.id IN ({placeholders})
        """,
        selected,
    ).fetchall()
    # Preserve order from comment_ids list.
    by_id = {r["id"]: r for r in rows}
    out: list[dict[str, Any]] = []
    for cid in selected:
        r = by_id.get(cid)
        if r is None:
            continue
        out.append(
            {
                "comment_id": r["id"],
                "pr_number": r["pr_number"],
                "path": r["path"],
                "line": r["line"],
                "author": r["author"],
                "tier": r["reviewer_tier"],
                "body": truncate(r["body"]),
                "diff_hunk": truncate(r["diff_hunk"]),
                "final_code_snippet": truncate(r["final_code_snippet"]),
            }
        )
    return out


def write_target(
    path: Path,
    target: str,
    agents_md: str,
    findings: list[dict[str, Any]],
) -> tuple[int, int]:
    payload = {
        "target": target,
        "agents_md": agents_md,
        "findings": findings,
    }
    path.write_text(json.dumps(payload, indent=2, ensure_ascii=False))
    with_evidence = sum(1 for f in findings if f.get("evidence"))
    return len(findings), with_evidence


# ---------------------------------------------------------------------------
# Per-target fetchers
# ---------------------------------------------------------------------------

REVIEW_TAXONOMIES = (
    "error-handling",
    "type-safety",
    "semantic-correctness",
    "testing",
    "provider-traits",
    "manifest-usage",
    "panic-discipline",
    "deprecated-patterns",
    "logging-tracing",
    "ld-floxlib",
    "control-flow",
)

STYLISTIC_TAXONOMIES = (
    "naming",
    "formatting-style",
    "imports",
    "user-facing-messages",
)


def fetch_findings(
    conn: sqlite3.Connection,
    where_sql: str,
    params: tuple[Any, ...] = (),
    order_by: str = "confidence_score DESC, total_evidence_count DESC, id ASC",
    limit: int | None = None,
) -> list[sqlite3.Row]:
    sql = f"SELECT * FROM finding WHERE {where_sql} ORDER BY {order_by}"
    if limit is not None:
        sql += f" LIMIT {limit}"
    return conn.execute(sql, params).fetchall()


def attach_evidence_for_top_n(
    conn: sqlite3.Connection,
    findings_dicts: list[dict[str, Any]],
    findings_rows: list[sqlite3.Row],
    top_n: int | None,
) -> None:
    """Mutate findings_dicts: attach `evidence` for top N (or all if None)."""
    count = len(findings_dicts) if top_n is None else min(top_n, len(findings_dicts))
    for i in range(count):
        ev = fetch_evidence(conn, findings_dicts[i]["evidence_comment_ids"])
        findings_dicts[i]["evidence"] = ev


def build_skill_review(conn: sqlite3.Connection, agents_md: str) -> tuple[int, int]:
    placeholders = ",".join("?" for _ in REVIEW_TAXONOMIES)
    rows = fetch_findings(
        conn,
        f"confidence_score >= 0.5 AND taxonomy IN ({placeholders})",
        REVIEW_TAXONOMIES,
    )
    dicts = [finding_to_dict(r) for r in rows]
    attach_evidence_for_top_n(conn, dicts, rows, top_n=30)
    return write_target(
        OUTPUT_DIR / "skill-review.json",
        "flox-rust-review skill",
        agents_md,
        dicts,
    )


def build_skill_stylistic(conn: sqlite3.Connection, agents_md: str) -> tuple[int, int]:
    placeholders = ",".join("?" for _ in STYLISTIC_TAXONOMIES)
    rows = fetch_findings(
        conn,
        f"confidence_score >= 0.5 AND taxonomy IN ({placeholders})",
        STYLISTIC_TAXONOMIES,
    )
    dicts = [finding_to_dict(r) for r in rows]
    attach_evidence_for_top_n(conn, dicts, rows, top_n=30)
    return write_target(
        OUTPUT_DIR / "skill-stylistic.json",
        "flox-rust-stylistic-conventions skill",
        agents_md,
        dicts,
    )


def build_area_commands(conn: sqlite3.Connection, agents_md: str) -> tuple[int, int]:
    rows = fetch_findings(
        conn,
        "(area = 'commands' OR area LIKE 'commands/%') AND confidence_score >= 0.4",
    )
    dicts = [finding_to_dict(r) for r in rows]
    attach_evidence_for_top_n(conn, dicts, rows, top_n=20)
    return write_target(
        OUTPUT_DIR / "area-commands.json",
        "commands area CLAUDE.md",
        agents_md,
        dicts,
    )


def build_area_models_env(conn: sqlite3.Connection, agents_md: str) -> tuple[int, int]:
    rows = fetch_findings(
        conn,
        "area = 'models/environment' AND confidence_score >= 0.4",
    )
    dicts = [finding_to_dict(r) for r in rows]
    attach_evidence_for_top_n(conn, dicts, rows, top_n=20)
    return write_target(
        OUTPUT_DIR / "area-models-environment.json",
        "models/environment area CLAUDE.md",
        agents_md,
        dicts,
    )


def build_area_providers(conn: sqlite3.Connection, agents_md: str) -> tuple[int, int]:
    rows = fetch_findings(
        conn,
        "area = 'providers' AND confidence_score >= 0.4",
    )
    dicts = [finding_to_dict(r) for r in rows]
    attach_evidence_for_top_n(conn, dicts, rows, top_n=20)
    return write_target(
        OUTPUT_DIR / "area-providers.json",
        "providers area CLAUDE.md",
        agents_md,
        dicts,
    )


def build_gap_report(conn: sqlite3.Connection, agents_md: str) -> tuple[int, int]:
    rows = fetch_findings(
        conn,
        "in_agents_md = 0 AND confidence_score >= 0.5",
    )
    dicts = [finding_to_dict(r) for r in rows]
    # Evidence for ALL findings.
    attach_evidence_for_top_n(conn, dicts, rows, top_n=None)
    return write_target(
        OUTPUT_DIR / "gap-report.json",
        "gap-report.md proposing AGENTS.md amendments",
        agents_md,
        dicts,
    )


def build_cli_claude_md(conn: sqlite3.Connection, agents_md: str) -> tuple[int, int]:
    rows = fetch_findings(conn, "1=1", limit=50)
    dicts = [finding_to_dict(r) for r in rows]
    # No evidence per spec.
    return write_target(
        OUTPUT_DIR / "cli-claude-md.json",
        "cli/CLAUDE.md main Rust cross-cutting",
        agents_md,
        dicts,
    )


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def main() -> None:
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    agents_md = AGENTS_MD_PATH.read_text()
    conn = connect()
    try:
        targets = [
            ("skill-review.json", build_skill_review),
            ("skill-stylistic.json", build_skill_stylistic),
            ("area-commands.json", build_area_commands),
            ("area-models-environment.json", build_area_models_env),
            ("area-providers.json", build_area_providers),
            ("gap-report.json", build_gap_report),
            ("cli-claude-md.json", build_cli_claude_md),
        ]
        print(f"{'file':<35} {'findings':>10} {'w/ evidence':>14}")
        print("-" * 62)
        for name, fn in targets:
            n_findings, n_evidence = fn(conn, agents_md)
            print(f"{name:<35} {n_findings:>10} {n_evidence:>14}")
    finally:
        conn.close()


if __name__ == "__main__":
    main()
