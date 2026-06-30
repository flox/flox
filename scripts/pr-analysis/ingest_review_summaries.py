#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = []
# ///
"""For every PR in the `pr` table, fetch review-summary bodies (non-empty)
and upsert into `review_summary`. Idempotent.
"""
from __future__ import annotations

from lib.db import connect, transaction
from lib.gh import run_json
from lib.reviewers import classify


def fetch_reviews(pr_number: int) -> list[dict]:
    return run_json([
        "api", "--paginate",
        f"repos/flox/flox/pulls/{pr_number}/reviews",
    ]) or []


def main() -> None:
    conn = connect()
    pr_numbers = [r["number"] for r in conn.execute("SELECT number FROM pr ORDER BY number")]
    total = 0
    for i, n in enumerate(pr_numbers, start=1):
        reviews = fetch_reviews(n)
        with transaction(conn):
            for r in reviews:
                body = (r.get("body") or "").strip()
                if not body:
                    continue
                user = r.get("user") or {}
                login = user.get("login", "unknown")
                author_type = user.get("type", "User")
                rev = classify(login, author_type)
                conn.execute(
                    """INSERT INTO review_summary
                       (id, pr_number, author, author_type, state, body,
                        submitted_at, commit_id)
                       VALUES (?,?,?,?,?,?,?,?)
                       ON CONFLICT(id) DO UPDATE SET
                         pr_number=excluded.pr_number,
                         author=excluded.author,
                         author_type=excluded.author_type,
                         state=excluded.state,
                         body=excluded.body,
                         submitted_at=excluded.submitted_at,
                         commit_id=excluded.commit_id""",
                    (
                        r["id"],
                        n,
                        login,
                        author_type,
                        r.get("state", "COMMENTED"),
                        body,
                        r["submitted_at"],
                        r.get("commit_id"),
                    ),
                )
                total += 1
        if i % 25 == 0 or i == len(pr_numbers):
            print(f"[{i}/{len(pr_numbers)}] cumulative review-summaries={total}")
    print(f"done. total review-summaries ingested: {total}")


if __name__ == "__main__":
    main()
