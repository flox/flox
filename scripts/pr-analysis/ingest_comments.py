#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = []
# ///
"""For every PR already in the `pr` table, fetch line-comments and upsert
into `line_comment` with reviewer tier/weight and area tag pre-computed.

Idempotent.
"""
from __future__ import annotations

from lib.areas import area_for_path
from lib.db import connect, transaction
from lib.gh import run_json
from lib.noise_filter import is_noise
from lib.reviewers import classify


def fetch_comments(pr_number: int) -> list[dict]:
    return run_json([
        "api", "--paginate",
        f"repos/flox/flox/pulls/{pr_number}/comments",
    ]) or []


def main() -> None:
    conn = connect()
    pr_numbers = [r["number"] for r in conn.execute("SELECT number FROM pr ORDER BY number")]
    total_comments = 0
    for i, n in enumerate(pr_numbers, start=1):
        comments = fetch_comments(n)
        with transaction(conn):
            for c in comments:
                user = c.get("user") or {}
                login = user.get("login", "unknown")
                author_type = user.get("type", "User")
                rev = classify(login, author_type)
                conn.execute(
                    """INSERT INTO line_comment
                       (id, pr_number, author, author_type, created_at,
                        path, line, original_line, side, diff_hunk, body, is_noise,
                        in_reply_to_id, area, reviewer_weight, reviewer_tier,
                        commit_id, original_commit_id)
                       VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)
                       ON CONFLICT(id) DO UPDATE SET
                         pr_number=excluded.pr_number,
                         author=excluded.author,
                         author_type=excluded.author_type,
                         created_at=excluded.created_at,
                         path=excluded.path,
                         line=excluded.line,
                         original_line=excluded.original_line,
                         side=excluded.side,
                         diff_hunk=excluded.diff_hunk,
                         body=excluded.body,
                         is_noise=excluded.is_noise,
                         in_reply_to_id=excluded.in_reply_to_id,
                         area=excluded.area,
                         reviewer_weight=excluded.reviewer_weight,
                         reviewer_tier=excluded.reviewer_tier,
                         commit_id=excluded.commit_id,
                         original_commit_id=excluded.original_commit_id""",
                    (
                        c["id"],
                        n,
                        login,
                        author_type,
                        c["created_at"],
                        c["path"],
                        c.get("line"),
                        c.get("original_line"),
                        c.get("side"),
                        c.get("diff_hunk"),
                        c["body"],
                        1 if is_noise(c["body"]) else 0,
                        c.get("in_reply_to_id"),
                        area_for_path(c["path"]),
                        rev.weight,
                        rev.tier,
                        c.get("commit_id"),
                        c.get("original_commit_id"),
                    ),
                )
        total_comments += len(comments)
        if i % 25 == 0 or i == len(pr_numbers):
            print(f"[{i}/{len(pr_numbers)}] cumulative comments={total_comments}")
    print(f"done. total comments ingested: {total_comments}")


if __name__ == "__main__":
    main()
