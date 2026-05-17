#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = []
# ///
"""Fetch the merged-PR list for the configured window and upsert into `pr` and
`pr_file` tables.

Idempotent: re-running updates rows in place.
"""
from __future__ import annotations

import argparse
import datetime as dt
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from lib.areas import is_rust
from lib.db import connect, transaction
from lib.gh import run_json

REPO = "flox/flox"
FIELDS = "number,title,author,state,mergedAt,baseRefOid,headRefOid,mergeCommit,url,files"


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--since", required=True, help="YYYY-MM-DD")
    parser.add_argument("--limit", type=int, default=1000)
    parser.add_argument("--rust-only", action="store_true", default=True,
                        help="(default true) only upsert PRs touching .rs files")
    parser.add_argument("--all", dest="rust_only", action="store_false")
    args = parser.parse_args()

    prs = run_json([
        "pr", "list",
        "--repo", REPO,
        "--state", "merged",
        "--search", f"merged:>={args.since}",
        "--limit", str(args.limit),
        "--json", FIELDS,
    ])

    now = dt.datetime.now(dt.UTC).isoformat()
    conn = connect()
    rust_prs = 0
    with transaction(conn):
        for pr in prs:
            files = pr.get("files") or []
            touches_rust = any(is_rust(f["path"]) for f in files)
            if args.rust_only and not touches_rust:
                continue
            rust_prs += 1
            author = pr.get("author") or {}
            merge_commit = pr.get("mergeCommit") or {}
            conn.execute(
                """INSERT OR REPLACE INTO pr
                   (number, title, author, author_type, state, merged_at,
                    base_sha, head_sha, merge_commit_sha, url, fetched_at)
                   VALUES (?,?,?,?,?,?,?,?,?,?,?)""",
                (
                    pr["number"],
                    pr["title"],
                    author.get("login", "unknown"),
                    author.get("type", "User"),
                    pr["state"],
                    pr["mergedAt"],
                    pr.get("baseRefOid"),
                    pr.get("headRefOid"),
                    merge_commit.get("oid"),
                    pr["url"],
                    now,
                ),
            )
            conn.execute("DELETE FROM pr_file WHERE pr_number = ?", (pr["number"],))
            conn.executemany(
                """INSERT INTO pr_file (pr_number, path, status, additions, deletions)
                   VALUES (?,?,?,?,?)""",
                [
                    (pr["number"], f["path"], None, f.get("additions"), f.get("deletions"))
                    for f in files
                ],
            )
    print(f"Ingested PRs: total={len(prs)} rust_touching={rust_prs}")


if __name__ == "__main__":
    main()
