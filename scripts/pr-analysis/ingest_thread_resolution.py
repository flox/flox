#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = []
# ///
"""For every PR in the `pr` table, fetch review-thread resolution state
via GitHub GraphQL and apply it to `line_comment.thread_resolved` +
`thread_resolved_by` for every comment whose id is in a thread.

Idempotent: re-running overwrites existing values.

Does NOT touch threads whose comments are not yet ingested — if a comment
appears in a thread but not in line_comment, it's silently skipped (the
caller should run ingest_comments.py first).
"""
from __future__ import annotations

from lib.db import connect, transaction
from lib.gh import run_json

QUERY = """
query ThreadResolution($owner: String!, $repo: String!, $number: Int!, $cursor: String) {
  repository(owner: $owner, name: $repo) {
    pullRequest(number: $number) {
      reviewThreads(first: 50, after: $cursor) {
        pageInfo { hasNextPage endCursor }
        nodes {
          isResolved
          resolvedBy { login }
          comments(first: 100) {
            nodes { databaseId }
          }
        }
      }
    }
  }
}
"""


def fetch_threads(pr_number: int) -> list[dict]:
    """Return a list of {isResolved, resolvedBy, comment_ids} for all threads in PR."""
    threads: list[dict] = []
    cursor: str | None = None
    while True:
        args = [
            "api", "graphql",
            "-f", f"query={QUERY}",
            "-F", "owner=flox",
            "-F", "repo=flox",
            "-F", f"number={pr_number}",
        ]
        if cursor:
            args.extend(["-F", f"cursor={cursor}"])
        payload = run_json(args)
        rt = (
            payload.get("data", {})
            .get("repository", {})
            .get("pullRequest", {})
            .get("reviewThreads", {})
        )
        for node in rt.get("nodes", []) or []:
            comment_ids = [
                c["databaseId"]
                for c in (node.get("comments", {}).get("nodes") or [])
                if c and c.get("databaseId") is not None
            ]
            resolved_by = (node.get("resolvedBy") or {}).get("login")
            threads.append({
                "isResolved": bool(node.get("isResolved")),
                "resolvedBy": resolved_by,
                "comment_ids": comment_ids,
            })
        page_info = rt.get("pageInfo") or {}
        if not page_info.get("hasNextPage"):
            break
        cursor = page_info.get("endCursor")
        if not cursor:
            break
    return threads


def main() -> None:
    conn = connect()
    pr_numbers = [r["number"] for r in conn.execute("SELECT number FROM pr ORDER BY number")]
    total_threads = 0
    total_comments_marked = 0
    for i, n in enumerate(pr_numbers, start=1):
        threads = fetch_threads(n)
        with transaction(conn):
            for t in threads:
                total_threads += 1
                if not t["comment_ids"]:
                    continue
                placeholders = ",".join("?" * len(t["comment_ids"]))
                cur = conn.execute(
                    f"""UPDATE line_comment
                        SET thread_resolved = ?, thread_resolved_by = ?
                        WHERE id IN ({placeholders})""",
                    (1 if t["isResolved"] else 0, t["resolvedBy"], *t["comment_ids"]),
                )
                total_comments_marked += cur.rowcount or 0
        if i % 25 == 0 or i == len(pr_numbers):
            print(f"[{i}/{len(pr_numbers)}] threads={total_threads} comments_marked={total_comments_marked}")
    print(f"done. threads={total_threads} comments_marked={total_comments_marked}")


if __name__ == "__main__":
    main()
