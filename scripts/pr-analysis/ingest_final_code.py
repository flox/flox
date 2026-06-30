#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = []
# ///
"""For each line_comment, fetch the file at the PR's merge_commit_sha and
extract a ~40-line window around the comment's anchor line. Stores into
`comment_final_code`. Idempotent.

We use `gh api repos/flox/flox/contents/<path>?ref=<sha>` which returns
base64-encoded file content. Files that no longer exist (deleted) record
snippet_available = 0.
"""
from __future__ import annotations

import base64

from lib.db import connect, transaction
from lib.gh import GhError, run_json

RADIUS = 20  # ~40-line window


def extract_window(lines: list[str], anchor_line: int | None, radius: int) -> str:
    if anchor_line is None:
        return ""
    start = max(1, anchor_line - radius)
    end = min(len(lines), anchor_line + radius)
    out = []
    for i in range(start, end + 1):
        out.append(f"{i}:{lines[i-1]}")
    return "\n".join(out)


def fetch_file(path: str, ref: str) -> list[str] | None:
    try:
        payload = run_json([
            "api", f"repos/flox/flox/contents/{path}?ref={ref}",
        ])
    except GhError:
        return None
    if not payload or payload.get("encoding") != "base64":
        return None
    try:
        text = base64.b64decode(payload["content"]).decode("utf-8", errors="replace")
    except Exception:
        return None
    return text.splitlines()


def main() -> None:
    conn = connect()
    rows = conn.execute(
        """SELECT lc.id, lc.path, lc.line, lc.original_line, p.merge_commit_sha
           FROM line_comment lc
           JOIN pr p ON p.number = lc.pr_number
           WHERE lc.id NOT IN (SELECT comment_id FROM comment_final_code)"""
    ).fetchall()
    total = len(rows)
    print(f"need to fetch snippets for {total} comments")
    file_cache: dict[tuple[str, str], list[str] | None] = {}
    done = 0
    for r in rows:
        key = (r["path"], r["merge_commit_sha"])
        if key not in file_cache:
            file_cache[key] = fetch_file(r["path"], r["merge_commit_sha"])
        file_lines = file_cache[key]
        anchor = r["line"] if r["line"] is not None else r["original_line"]
        if file_lines is None:
            snippet, available = None, 0
        else:
            snippet = extract_window(file_lines, anchor, RADIUS)
            available = 1
        with transaction(conn):
            conn.execute(
                """INSERT OR REPLACE INTO comment_final_code
                   (comment_id, final_code_snippet, snippet_available)
                   VALUES (?,?,?)""",
                (r["id"], snippet, available),
            )
        done += 1
        if done % 100 == 0 or done == total:
            print(f"[{done}/{total}] cached files={len(file_cache)}")
    print("done")


if __name__ == "__main__":
    main()
