#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = []
# ///
"""Pull the backing comments for high-evidence findings (evd >= 2) into a
single JSON file that the controller hands to a Sonnet subagent for a more
careful re-classification pass.

Output format: a list of comment objects to be re-classified:
  [
    {
      "comment_id": int,
      "current_taxonomy": str,
      "current_rule_statement": str,
      "current_confidence": float,
      "body": str,
      "diff_hunk": str,
      "final_code_snippet": str,
      "area": str,
      "thread_resolved": 0 | 1,
      "thread_resolved_by": str | null,
      "current_finding_id": int,
      "current_finding_rule": str,
    },
    ...
  ]
"""
from __future__ import annotations

import json
from pathlib import Path

from lib.db import connect

OUT_PATH = Path("/tmp/full_classify/sonnet_reclassify_input.json")


def main() -> None:
    conn = connect()
    # Pull comment_ids from findings with evd >= 2
    findings = conn.execute(
        """SELECT id, rule_statement, evidence_comment_ids
           FROM finding
           WHERE total_evidence_count >= 2
           ORDER BY total_evidence_count DESC, confidence_score DESC"""
    ).fetchall()
    print(f"high-evidence findings (evd>=2): {len(findings)}")
    comment_ids: set[int] = set()
    finding_for_comment: dict[int, tuple[int, str]] = {}
    for f in findings:
        ids = json.loads(f["evidence_comment_ids"])
        for cid in ids:
            comment_ids.add(cid)
            finding_for_comment.setdefault(cid, (f["id"], f["rule_statement"]))
    print(f"backing comments to re-classify: {len(comment_ids)}")

    rows = conn.execute(
        f"""SELECT c.comment_id, c.taxonomy, c.rule_statement, c.confidence,
               lc.body, lc.diff_hunk, lc.thread_resolved, lc.thread_resolved_by,
               lc.area, cfc.final_code_snippet
           FROM classification c
           JOIN line_comment lc ON lc.id = c.comment_id
           LEFT JOIN comment_final_code cfc ON cfc.comment_id = c.comment_id
           WHERE c.comment_id IN ({','.join('?' * len(comment_ids))})""",
        tuple(comment_ids),
    ).fetchall()

    out = []
    for r in rows:
        fid, frule = finding_for_comment[r["comment_id"]]
        out.append({
            "comment_id": r["comment_id"],
            "current_taxonomy": r["taxonomy"],
            "current_rule_statement": r["rule_statement"],
            "current_confidence": r["confidence"],
            "body": r["body"],
            "diff_hunk": r["diff_hunk"] or "",
            "final_code_snippet": r["final_code_snippet"] or "",
            "area": r["area"],
            "thread_resolved": int(r["thread_resolved"]) if r["thread_resolved"] is not None else 0,
            "thread_resolved_by": r["thread_resolved_by"],
            "current_finding_id": fid,
            "current_finding_rule": frule,
        })
    OUT_PATH.parent.mkdir(parents=True, exist_ok=True)
    OUT_PATH.write_text(json.dumps(out, indent=2))
    print(f"wrote {OUT_PATH} ({len(out)} entries)")


if __name__ == "__main__":
    main()
