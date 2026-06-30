#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = []
# ///
"""For every PR in the `pr` table, fetch top-level issue comments and upsert
into `pr_comment`. Idempotent.
"""
from __future__ import annotations

from lib.db import connect, transaction
from lib.gh import run_json


def fetch_comments(pr_number: int) -> list[dict]:
    return run_json([
        "api", "--paginate",
        f"repos/flox/flox/issues/{pr_number}/comments",
    ]) or []


def main() -> None:
    conn = connect()
    pr_numbers = [r["number"] for r in conn.execute("SELECT number FROM pr ORDER BY number")]
    total = 0
    for i, n in enumerate(pr_numbers, start=1):
        comments = fetch_comments(n)
        with transaction(conn):
            for c in comments:
                user = c.get("user") or {}
                conn.execute(
                    """INSERT INTO pr_comment
                       (id, pr_number, author, author_type, body, created_at)
                       VALUES (?,?,?,?,?,?)
                       ON CONFLICT(id) DO UPDATE SET
                         pr_number=excluded.pr_number,
                         author=excluded.author,
                         author_type=excluded.author_type,
                         body=excluded.body,
                         created_at=excluded.created_at""",
                    (
                        c["id"],
                        n,
                        user.get("login", "unknown"),
                        user.get("type", "User"),
                        c["body"],
                        c["created_at"],
                    ),
                )
                total += 1
        if i % 25 == 0 or i == len(pr_numbers):
            print(f"[{i}/{len(pr_numbers)}] cumulative issue-comments={total}")
    print(f"done. total issue-comments ingested: {total}")


if __name__ == "__main__":
    main()
