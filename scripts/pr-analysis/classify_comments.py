#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = ["anthropic>=0.40"]
# ///
"""Run Claude Haiku 4.5 over every un-classified line_comment and write a
row into `classification`.

Concurrent via a thread pool with bounded parallelism (Anthropic SDK uses
httpx; threads are fine).
"""
from __future__ import annotations

import argparse
import datetime as dt
import sys
from concurrent.futures import ThreadPoolExecutor, as_completed

from lib.classify_helpers import (
    SYSTEM_PROMPT,
    build_user_prompt,
    parse,
    taxonomy_block,
)
from lib.db import connect, transaction
from lib.llm import CLASSIFY_MODEL, call_with_retry, client

__all__ = [
    "SYSTEM_PROMPT",
    "build_user_prompt",
    "parse",
    "taxonomy_block",
    "classify_one",
    "main",
]


def classify_one(anthropic, row: dict, tax_block: str) -> tuple[int, dict, str]:
    user = build_user_prompt(
        row["body"], row["diff_hunk"] or "", row["final_code_snippet"] or "", tax_block
    )
    raw = call_with_retry(
        anthropic, model=CLASSIFY_MODEL, system=SYSTEM_PROMPT, user=user
    )
    return row["id"], parse(raw), raw


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--concurrency", type=int, default=8)
    parser.add_argument("--limit", type=int, default=None,
                        help="for smoke runs; classifies at most N comments")
    args = parser.parse_args()

    conn = connect()
    sql = """
        SELECT lc.id, lc.body, lc.diff_hunk, cfc.final_code_snippet
        FROM line_comment lc
        LEFT JOIN comment_final_code cfc ON cfc.comment_id = lc.id
        WHERE lc.id NOT IN (SELECT comment_id FROM classification)
          AND lc.reviewer_tier != 4
    """
    if args.limit:
        sql += f" LIMIT {args.limit}"
    rows = [dict(r) for r in conn.execute(sql).fetchall()]
    print(f"to classify: {len(rows)}")
    if not rows:
        return

    tax_block = taxonomy_block()
    anthropic = client()
    now = dt.datetime.now(dt.UTC).isoformat()
    done = 0
    with ThreadPoolExecutor(max_workers=args.concurrency) as ex:
        futs = {ex.submit(classify_one, anthropic, r, tax_block): r["id"] for r in rows}
        for fut in as_completed(futs):
            cid = futs[fut]
            try:
                comment_id, parsed, raw = fut.result()
            except Exception as exc:
                print(f"comment {cid} failed: {exc}", file=sys.stderr)
                continue
            with transaction(conn):
                conn.execute(
                    """INSERT OR REPLACE INTO classification
                       (comment_id, taxonomy, was_addressed, rule_statement,
                        confidence, classifier_model, classified_at, raw_response)
                       VALUES (?,?,?,?,?,?,?,?)""",
                    (
                        comment_id,
                        parsed["taxonomy"],
                        1 if parsed["was_addressed"] is True else (0 if parsed["was_addressed"] is False else None),
                        parsed["rule_statement"],
                        float(parsed["confidence"]),
                        CLASSIFY_MODEL,
                        now,
                        raw,
                    ),
                )
            done += 1
            if done % 50 == 0:
                print(f"classified {done}/{len(rows)}")
    print(f"done. classified {done}/{len(rows)}")


if __name__ == "__main__":
    main()
