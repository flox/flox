#!/usr/bin/env python3
"""Build the enriched Task 9 user-review markdown document.

For each finding (cross-cutting first, then top area-specific, then all
gap candidates not in AGENTS.md), emit:
  - the synthesized rule statement
  - per-evidence: source comment body, diff hunk, merged final code
The reviewer can then verify each rule against real reviewer voices and
actual code.

Additionally:
  - 'other'-bucket clusters: rules with taxonomy='other' AND confidence>=0.5,
    grouped by Jaccard similarity over their rule_statements (no extra deps).
  - 10 random low-confidence 'other' samples to verify the noise filter.

Run via `uv run build_task9_review.py` (stdlib only; uv is not strictly
required, but the task description uses that command).
"""
from __future__ import annotations

import json
import re
import sqlite3
import textwrap
from pathlib import Path
from typing import Iterable

from lib.db import connect

HERE = Path(__file__).resolve().parent
OUT_PATH = HERE / "findings" / "task9-review.md"

MAX_BODY_FULL = 500
MAX_HUNK_FULL = 800
MAX_FINAL_FULL = 800
MAX_BODY_TIGHT = 200
MAX_HUNK_TIGHT = 400
MAX_FINAL_TIGHT = 400
MAX_BODY_OTHER = 600

EVIDENCE_LIMIT = 3
TOP_AREA_LIMIT = 50

# --------------------------------------------------------------------------- #
# Utility helpers
# --------------------------------------------------------------------------- #


def truncate(s: str | None, limit: int) -> str:
    if s is None:
        return "(none)"
    s = s.rstrip()
    if not s:
        return "(empty)"
    if len(s) <= limit:
        return s
    return s[: limit].rstrip() + " [...]"


def parse_json_list(raw: str) -> list:
    if not raw:
        return []
    try:
        return json.loads(raw)
    except (ValueError, TypeError):
        return []


def format_pr_list(raw: str, max_items: int = 8) -> str:
    items = parse_json_list(raw)
    if not items:
        return "(none)"
    head = items[:max_items]
    suffix = f" (+{len(items) - max_items} more)" if len(items) > max_items else ""
    return ", ".join(f"#{x}" for x in head) + suffix


def yn(val: int | None) -> str:
    if val is None:
        return "N/A"
    return "Y" if int(val) else "N"


def bool_str(val: int | None) -> str:
    if val is None:
        return "unknown"
    return "true" if int(val) else "false"


def round2(val: float | int | None) -> str:
    if val is None:
        return "n/a"
    try:
        return f"{float(val):.2f}"
    except (ValueError, TypeError):
        return str(val)


# --------------------------------------------------------------------------- #
# Evidence rendering
# --------------------------------------------------------------------------- #


def render_evidence(
    conn: sqlite3.Connection,
    comment_id: int,
    idx: int,
    *,
    tight: bool,
) -> list[str]:
    row = conn.execute(
        """SELECT lc.id, lc.pr_number, lc.author, lc.reviewer_tier,
                  lc.path, lc.line, lc.body, lc.diff_hunk, lc.thread_resolved,
                  cfc.final_code_snippet, cfc.snippet_available,
                  c.was_addressed, c.confidence
             FROM line_comment lc
             LEFT JOIN comment_final_code cfc ON cfc.comment_id = lc.id
             LEFT JOIN classification c ON c.comment_id = lc.id
            WHERE lc.id = ?""",
        (comment_id,),
    ).fetchone()
    if row is None:
        return [f"#### Evidence {idx}: comment_id={comment_id} (missing)\n"]

    body_limit = MAX_BODY_TIGHT if tight else MAX_BODY_FULL
    hunk_limit = MAX_HUNK_TIGHT if tight else MAX_HUNK_FULL
    final_limit = MAX_FINAL_TIGHT if tight else MAX_FINAL_FULL

    snippet_available = row["snippet_available"]
    final_snippet = row["final_code_snippet"]
    if not snippet_available or final_snippet is None:
        final_block = "(snippet not available — file deleted, renamed, or out-of-range at merge)"
    else:
        final_block = truncate(final_snippet, final_limit)

    path = row["path"] or "?"
    line = row["line"] if row["line"] is not None else "?"
    author = row["author"] or "?"
    tier = row["reviewer_tier"]

    lines: list[str] = []
    lines.append(
        f"#### Evidence {idx}: PR #{row['pr_number']} @ `{path}:{line}` — "
        f"{author} (Tier {tier})"
    )
    lines.append(
        f"- **Thread resolved:** {yn(row['thread_resolved'])}   "
        f"**was_addressed:** {bool_str(row['was_addressed'])}   "
        f"**classification confidence:** {round2(row['confidence'])}"
    )
    lines.append("")
    lines.append("**Source comment:**")
    body_text = truncate(row["body"], body_limit)
    for ln in body_text.splitlines() or [""]:
        lines.append(f"> {ln}")
    lines.append("")
    lines.append("**Diff hunk (what reviewer saw):**")
    lines.append("```")
    lines.append(truncate(row["diff_hunk"], hunk_limit))
    lines.append("```")
    lines.append("")
    lines.append("**Merged final code:**")
    lines.append("```")
    lines.append(final_block)
    lines.append("```")
    lines.append("")
    return lines


def render_finding_block(
    conn: sqlite3.Connection,
    finding: sqlite3.Row,
    *,
    tight: bool = False,
) -> list[str]:
    lines: list[str] = []
    lines.append(f"### F#{finding['id']}: {finding['rule_statement']}")
    lines.append(
        f"- **Taxonomy:** `{finding['taxonomy']}`   "
        f"**Area:** `{finding['area']}`   "
        f"**Scope:** `{finding['scope']}`"
    )
    lines.append(
        f"- **Reviewer-tier breakdown:** "
        f"T1={finding['tier1_reviewer_count']}, "
        f"T2={finding['tier2_reviewer_count']}"
    )
    lines.append(
        f"- **Evidence:** {finding['total_evidence_count']} comments across PRs "
        f"{format_pr_list(finding['evidence_pr_numbers'])}"
    )
    section = finding["agents_md_section"] or "—"
    lines.append(
        f"- **Confidence:** {round2(finding['confidence_score'])}   "
        f"**In AGENTS.md?:** {yn(finding['in_agents_md'])} "
        f"({section})   "
        f"**Cross-area count:** {finding['cross_area_count']}"
    )
    accept = finding["acceptance_rate"]
    if accept is not None:
        lines.append(f"- **Acceptance rate:** {round2(accept)}")
    lines.append("")

    comment_ids = parse_json_list(finding["evidence_comment_ids"])
    if not comment_ids:
        lines.append("_No evidence comment IDs recorded._\n")
        return lines

    for i, cid in enumerate(comment_ids[:EVIDENCE_LIMIT], start=1):
        try:
            cid_int = int(cid)
        except (TypeError, ValueError):
            continue
        lines.extend(render_evidence(conn, cid_int, i, tight=tight))
    return lines


# --------------------------------------------------------------------------- #
# 'Other'-bucket helpers
# --------------------------------------------------------------------------- #


_TOKEN_RE = re.compile(r"[a-z]{4,}")


def tokenize_for_jaccard(text: str) -> set[str]:
    return set(_TOKEN_RE.findall((text or "").lower()))


def jaccard_cluster(
    items: list[dict],
    threshold: float = 0.35,
) -> list[list[int]]:
    """Greedy single-link clustering by token Jaccard."""
    token_sets = [tokenize_for_jaccard(it["rule_statement"]) for it in items]
    clusters: list[list[int]] = []
    cluster_tokens: list[set[str]] = []
    for i, toks in enumerate(token_sets):
        best_j = -1
        best_sim = 0.0
        for j, ct in enumerate(cluster_tokens):
            if not toks and not ct:
                sim = 1.0
            elif not toks or not ct:
                sim = 0.0
            else:
                sim = len(toks & ct) / len(toks | ct)
            if sim >= threshold and sim > best_sim:
                best_sim = sim
                best_j = j
        if best_j == -1:
            clusters.append([i])
            cluster_tokens.append(set(toks))
        else:
            clusters[best_j].append(i)
            cluster_tokens[best_j] |= toks
    clusters.sort(key=len, reverse=True)
    return clusters


def render_other_high_conf(conn: sqlite3.Connection) -> list[str]:
    rows = conn.execute(
        """SELECT c.comment_id, c.rule_statement, c.confidence,
                  lc.area, lc.pr_number, lc.path, lc.line, lc.author,
                  lc.reviewer_tier, lc.body, lc.diff_hunk, lc.thread_resolved,
                  cfc.final_code_snippet, cfc.snippet_available,
                  c.was_addressed
             FROM classification c
             JOIN line_comment lc ON lc.id = c.comment_id
             LEFT JOIN comment_final_code cfc ON cfc.comment_id = c.comment_id
            WHERE c.taxonomy = 'other'
              AND c.rule_statement <> ''
              AND c.confidence >= 0.5
            ORDER BY c.confidence DESC""",
    ).fetchall()

    items = [dict(r) for r in rows]
    if not items:
        return ["_No high-confidence 'other'-bucket classifications._\n"]

    clusters = jaccard_cluster(items, threshold=0.34)

    out: list[str] = []
    out.append(f"_Found {len(items)} high-confidence 'other'-bucket "
               f"classifications in {len(clusters)} clusters._\n")
    for k, cluster in enumerate(clusters, start=1):
        cluster_items = [items[i] for i in cluster]
        common = sorted(
            set.intersection(
                *[tokenize_for_jaccard(it["rule_statement"]) for it in cluster_items]
            )
            if cluster_items
            else set()
        )
        out.append(f"### Other-cluster {k}  (size={len(cluster_items)})")
        if common:
            out.append(f"_Common tokens: {', '.join(sorted(common))}_")
        out.append("")
        for it in cluster_items:
            out.append(
                f"#### PR #{it['pr_number']} @ `{it['path']}:{it['line']}` — "
                f"{it['author']} (Tier {it['reviewer_tier']}, conf={round2(it['confidence'])})"
            )
            out.append(f"- **Rule statement:** {it['rule_statement']}")
            out.append(
                f"- **Area:** `{it['area']}`   "
                f"**Thread resolved:** {yn(it['thread_resolved'])}   "
                f"**was_addressed:** {bool_str(it['was_addressed'])}"
            )
            out.append("")
            out.append("**Source comment:**")
            for ln in truncate(it["body"], MAX_BODY_OTHER).splitlines() or [""]:
                out.append(f"> {ln}")
            out.append("")
            out.append("**Diff hunk:**")
            out.append("```")
            out.append(truncate(it["diff_hunk"], MAX_HUNK_FULL))
            out.append("```")
            out.append("")
            out.append("**Merged final code:**")
            out.append("```")
            if it["snippet_available"] and it["final_code_snippet"] is not None:
                out.append(truncate(it["final_code_snippet"], MAX_FINAL_FULL))
            else:
                out.append("(snippet not available)")
            out.append("```")
            out.append("")
    return out


def render_other_low_conf_samples(conn: sqlite3.Connection) -> list[str]:
    rows = conn.execute(
        """SELECT c.comment_id, c.rule_statement, c.confidence,
                  lc.area, lc.pr_number, lc.path, lc.line, lc.author,
                  lc.reviewer_tier, lc.body
             FROM classification c
             JOIN line_comment lc ON lc.id = c.comment_id
            WHERE c.taxonomy = 'other'
              AND c.confidence < 0.3
            ORDER BY RANDOM()
            LIMIT 10""",
    ).fetchall()
    out: list[str] = []
    out.append(
        "_10 random samples from the 'other'-bucket classifications with "
        "confidence < 0.3 — these should look like reviewer noise (acks, "
        "questions, nits unrelated to a rule)._\n"
    )
    for i, r in enumerate(rows, start=1):
        out.append(
            f"#### Sample {i}: PR #{r['pr_number']} @ `{r['path']}:{r['line']}` "
            f"— {r['author']} (Tier {r['reviewer_tier']}, conf={round2(r['confidence'])})"
        )
        out.append(f"- **Area:** `{r['area']}`")
        if r["rule_statement"]:
            out.append(f"- **Rule statement (probably noise):** {r['rule_statement']}")
        out.append("")
        out.append("**Source comment:**")
        for ln in truncate(r["body"], MAX_BODY_OTHER).splitlines() or [""]:
            out.append(f"> {ln}")
        out.append("")
    return out


# --------------------------------------------------------------------------- #
# Top-level builder
# --------------------------------------------------------------------------- #


def fetch_findings(
    conn: sqlite3.Connection,
    where: str,
    params: tuple = (),
    order: str = "f.confidence_score DESC, f.id ASC",
    limit: int | None = None,
) -> list[sqlite3.Row]:
    sql = (
        "SELECT f.id, f.theme, f.rule_statement, f.area, f.scope, f.taxonomy, "
        "       f.tier1_reviewer_count, f.tier2_reviewer_count, "
        "       f.total_evidence_count, f.cross_area_count, f.areas_seen, "
        "       f.evidence_comment_ids, f.evidence_pr_numbers, "
        "       f.confidence_score, f.in_agents_md, f.agents_md_section, "
        "       f.acceptance_rate "
        "  FROM finding f "
        f" WHERE {where} "
        f" ORDER BY {order}"
    )
    if limit is not None:
        sql += f" LIMIT {int(limit)}"
    return conn.execute(sql, params).fetchall()


def build_document(conn: sqlite3.Connection) -> str:
    total = conn.execute("SELECT COUNT(*) FROM finding").fetchone()[0]
    classifications = conn.execute(
        "SELECT COUNT(*) FROM classification"
    ).fetchone()[0]
    pr_count = conn.execute("SELECT COUNT(*) FROM pr").fetchone()[0]

    lines: list[str] = []
    lines.append("# Task 9 user-review document")
    lines.append("")
    lines.append(
        f"**Source:** {pr_count} PRs, {classifications} classifications, "
        f"{total} findings after dedup."
    )
    lines.append("**Reviewer's job:** verify each rule captures a real Rust "
                 "convention; flag false positives.")
    lines.append("")
    lines.append("## How to read")
    lines.append("")
    lines.append("For each finding below:")
    lines.append("- `Rule:` the synthesized one-sentence rule.")
    lines.append("- `Source comment:` what the reviewer wrote.")
    lines.append("- `Diff hunk:` the code the reviewer was looking at "
                 "(often the BEFORE state).")
    lines.append("- `Merged final code:` the code at that location after "
                 "merge (often the AFTER).")
    lines.append("- `Evidence count:` how many comments support this rule.")
    lines.append("- `Reviewer voices:` who said it (tier).")
    lines.append("- `In AGENTS.md?:` whether the existing AGENTS.md already "
                 "encodes this rule.")
    lines.append("")
    lines.append("Long diff hunks and final code snippets are truncated to "
                 "~800 chars with a `[...]` suffix.")
    lines.append("")

    # --- Cross-cutting --------------------------------------------------- #
    cross = fetch_findings(conn, "f.scope = 'cross-cutting'")
    lines.append("## Cross-cutting findings (top of the skill)")
    lines.append("")
    lines.append(
        f"_{len(cross)} cross-cutting findings, ordered by confidence "
        "descending._"
    )
    lines.append("")
    for f in cross:
        lines.extend(render_finding_block(conn, f, tight=False))

    # --- Top area-specific ---------------------------------------------- #
    top_area = fetch_findings(
        conn,
        "f.scope = 'area-specific'",
        limit=TOP_AREA_LIMIT,
    )
    lines.append(f"## Top area-specific findings — {len(top_area)} highest confidence")
    lines.append("")
    for f in top_area:
        lines.extend(render_finding_block(conn, f, tight=False))

    # --- Gap candidates (not in AGENTS.md) ------------------------------ #
    gaps = fetch_findings(conn, "f.in_agents_md = 0")
    lines.append(f"## Gap candidates — rules NOT in AGENTS.md ({len(gaps)} total, ordered by confidence)")
    lines.append("")
    lines.append("_Tighter rendering: comment body truncated to 200 chars, "
                 "diff hunk and final code to 400 chars._")
    lines.append("")
    for f in gaps:
        lines.extend(render_finding_block(conn, f, tight=True))

    # --- 'Other'-bucket high-confidence rules --------------------------- #
    lines.append("## High-confidence 'other'-bucket rules (Task 8.5 candidates)")
    lines.append("")
    lines.extend(render_other_high_conf(conn))

    # --- 'Other'-bucket low-confidence samples -------------------------- #
    lines.append("## Sample of 'other'-bucket comments classified with LOW confidence")
    lines.append("")
    lines.extend(render_other_low_conf_samples(conn))

    return "\n".join(lines) + "\n"


def main() -> None:
    conn = connect()
    text = build_document(conn)
    OUT_PATH.parent.mkdir(parents=True, exist_ok=True)
    OUT_PATH.write_text(text)
    line_count = text.count("\n")
    print(f"wrote {OUT_PATH.relative_to(HERE)} ({line_count} lines)")


if __name__ == "__main__":
    main()
