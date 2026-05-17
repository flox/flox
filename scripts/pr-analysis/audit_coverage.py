#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = []
# ///
"""Coverage audit: prove every line-comment on every ingested PR landed
in the DB and was classified.

Exits non-zero (and prints a summary) if any of these invariants fail:
  1. The set of comment IDs in the DB for a PR matches what GitHub reports.
  2. Every comment has a comment_final_code row (snippet may be null).
  3. Every non-bot comment has a classification row.
     (Skipped under --ingest-only — used between ingest and classification.)
  4. No line_comment has area = 'other' for a path under cli/ (would
     indicate a missing prefix in lib/areas.py).
"""
from __future__ import annotations

import argparse
import subprocess
import sys

from lib.db import connect


def gh_comment_ids(pr_number: int) -> set[int]:
    proc = subprocess.run(
        ["gh", "api", "--paginate",
         f"repos/flox/flox/pulls/{pr_number}/comments",
         "-q", ".[].id"],
        capture_output=True, text=True, check=True,
    )
    return {int(line) for line in proc.stdout.split() if line.strip()}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--ingest-only",
        action="store_true",
        help="run between ingest and classification: skips invariant 3 "
             "(every non-bot comment classified)",
    )
    args = parser.parse_args()

    conn = connect()
    failures: list[str] = []

    prs = [r["number"] for r in conn.execute("SELECT number FROM pr ORDER BY number")]
    print(f"auditing {len(prs)} PRs (mode={'ingest-only' if args.ingest_only else 'full'})")

    # Invariant 1: per-PR comment-id parity with GitHub.
    for n in prs:
        gh_ids = gh_comment_ids(n)
        db_ids = {
            r["id"] for r in conn.execute(
                "SELECT id FROM line_comment WHERE pr_number = ?", (n,)
            )
        }
        missing = gh_ids - db_ids
        extra = db_ids - gh_ids
        if missing or extra:
            failures.append(
                f"PR #{n}: gh={len(gh_ids)} db={len(db_ids)} "
                f"missing_from_db={sorted(missing)} extra_in_db={sorted(extra)}"
            )

    # Invariant 2: every comment has a comment_final_code row.
    orphans = conn.execute(
        """SELECT lc.id FROM line_comment lc
           LEFT JOIN comment_final_code cfc ON cfc.comment_id = lc.id
           WHERE cfc.comment_id IS NULL"""
    ).fetchall()
    if orphans:
        failures.append(f"{len(orphans)} comments have no comment_final_code row")

    # Invariant 3: every non-bot comment has a classification row.
    if not args.ingest_only:
        unclassified = conn.execute(
            """SELECT lc.id FROM line_comment lc
               LEFT JOIN classification c ON c.comment_id = lc.id
               WHERE lc.reviewer_tier != 4 AND lc.is_noise = 0
                 AND c.comment_id IS NULL"""
        ).fetchall()
        if unclassified:
            failures.append(f"{len(unclassified)} non-bot, non-noise comments are unclassified")

    # Invariant 4: no 'other' area for cli/ paths.
    leaked = conn.execute(
        """SELECT id, path FROM line_comment
           WHERE area = 'other' AND path LIKE 'cli/%'"""
    ).fetchall()
    if leaked:
        sample = ", ".join(f"#{r['id']}({r['path']})" for r in leaked[:5])
        failures.append(
            f"{len(leaked)} cli/* comments fell into area='other' (lib/areas.py gap): {sample}"
        )

    print("---")
    if failures:
        print("FAIL")
        for f in failures:
            print(" - " + f)
        return 1
    print("OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
