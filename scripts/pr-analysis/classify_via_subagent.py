#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = []
# ///
"""Subagent-orchestrated classifier.

Alternative to ``classify_comments.py`` that does NOT call the Anthropic
SDK directly. Instead it splits the un-classified line_comment rows into
JSON batch files on disk; the parent Claude session then dispatches one
Haiku subagent per batch (each writes a ``result_<N>.json``), and the
``ingest`` mode reads those result files into the ``classification``
table.

This script must remain importable without the ``anthropic`` package.
"""
from __future__ import annotations

import argparse
import datetime as dt
import json
import sqlite3
import sys
from collections import Counter
from pathlib import Path

from lib.classify_helpers import SYSTEM_PROMPT, parse, prompt_hash, taxonomy_block
from lib.db import connect, transaction
from lib.taxonomy import TAXONOMY_IDS

CLASSIFIER_MODEL = "claude-haiku-4-5-via-subagent"


# -----------------------------------------------------------------------------
# prepare mode
# -----------------------------------------------------------------------------

UNCLASSIFIED_SQL = """
    SELECT lc.id, lc.body, lc.diff_hunk, lc.area, cfc.final_code_snippet
    FROM line_comment lc
    LEFT JOIN comment_final_code cfc ON cfc.comment_id = lc.id
    WHERE lc.id NOT IN (SELECT comment_id FROM classification)
      AND lc.reviewer_tier != 4
      AND lc.is_noise = 0
    ORDER BY lc.id
"""


def fetch_unclassified(conn: sqlite3.Connection) -> list[dict]:
    rows = conn.execute(UNCLASSIFIED_SQL).fetchall()
    return [
        {
            "id": r["id"],
            "body": r["body"],
            "diff_hunk": r["diff_hunk"] or "",
            "final_code_snippet": r["final_code_snippet"] or "",
            "area": r["area"],
        }
        for r in rows
    ]


def chunked(items: list[dict], size: int) -> list[list[dict]]:
    if size <= 0:
        raise ValueError("batch size must be positive")
    return [items[i:i + size] for i in range(0, len(items), size)]


def write_batches(
    comments: list[dict],
    *,
    batch_size: int,
    out_dir: Path,
) -> list[tuple[Path, int]]:
    out_dir.mkdir(parents=True, exist_ok=True)
    tax_block = taxonomy_block()
    ph = prompt_hash(SYSTEM_PROMPT, tax_block)
    batches = chunked(comments, batch_size)
    written: list[tuple[Path, int]] = []
    for idx, batch in enumerate(batches, start=1):
        path = out_dir / f"batch_{idx}.json"
        payload = {
            "batch_id": idx,
            "prompt_hash": ph,
            "system_prompt": SYSTEM_PROMPT,
            "taxonomy_block": tax_block,
            "comments": batch,
        }
        path.write_text(json.dumps(payload, indent=2))
        written.append((path, len(batch)))
    return written


def prepare_cmd(args: argparse.Namespace) -> int:
    conn = connect()
    comments = fetch_unclassified(conn)
    if not comments:
        print("nothing to classify: no un-classified line_comment rows with reviewer_tier != 4")
        return 0
    out_dir = Path(args.out_dir).resolve()
    written = write_batches(comments, batch_size=args.batch_size, out_dir=out_dir)
    ph = prompt_hash(SYSTEM_PROMPT, taxonomy_block())
    print(f"Prepared {len(written)} batches in {out_dir}/ (prompt_hash={ph})")
    for path, count in written:
        print(f"{path.name}: {count} comments")
    print("Controller next steps:")
    print(f"  1. Dispatch {len(written)} parallel Haiku subagents, one per batch file.")
    print("     Each subagent prompt should: Read the batch file, classify each comment")
    print("     per system_prompt + taxonomy_block, write a JSON array of result objects")
    print(f"     to {out_dir}/result_<N>.json")
    print("  2. After all result files exist, run:")
    print(f"     uv run classify_via_subagent.py ingest --in-dir {out_dir}")
    return 0


# -----------------------------------------------------------------------------
# ingest mode
# -----------------------------------------------------------------------------


def _coerce_was_addressed(value: object) -> int | None:
    if value is True:
        return 1
    if value is False:
        return 0
    return None


def normalize_result(item: dict) -> dict:
    """Apply parse()-style defaults and clamping to one result object.

    Does NOT validate that ``comment_id`` is present; the caller handles
    that so it can warn-and-skip.
    """
    taxonomy = item.get("taxonomy")
    if taxonomy not in TAXONOMY_IDS:
        taxonomy = "other"
    rule_statement = item.get("rule_statement")
    if rule_statement is None:
        rule_statement = ""
    try:
        confidence = float(item.get("confidence", 0.0))
    except (TypeError, ValueError):
        confidence = 0.0
    if confidence < 0.0:
        confidence = 0.0
    elif confidence > 1.0:
        confidence = 1.0
    return {
        "taxonomy": taxonomy,
        "was_addressed": _coerce_was_addressed(item.get("was_addressed")),
        "rule_statement": rule_statement,
        "confidence": confidence,
    }


def load_result_file(path: Path) -> list[dict]:
    data = json.loads(path.read_text())
    if not isinstance(data, list):
        raise ValueError(f"{path}: expected a JSON array, got {type(data).__name__}")
    return data


def _batch_path_for_result(result_path: Path) -> Path:
    """Sibling batch file for a result_<N>.json — same dir, batch_<N>.json."""
    name = result_path.name
    # result_<N>.json -> batch_<N>.json
    suffix = name[len("result_"):]
    return result_path.parent / f"batch_{suffix}"


def _read_prompt_hash(batch_path: Path) -> str | None:
    """Return the prompt_hash field from the batch file, or None if unavailable."""
    if not batch_path.is_file():
        return None
    try:
        payload = json.loads(batch_path.read_text())
    except (ValueError, json.JSONDecodeError):
        return None
    ph = payload.get("prompt_hash")
    return ph if isinstance(ph, str) else None


def ingest_results(
    conn: sqlite3.Connection,
    in_dir: Path,
    *,
    now_iso: str | None = None,
) -> tuple[int, int]:
    """Ingest every ``result_*.json`` file in ``in_dir``.

    Returns ``(rows_inserted, files_processed)``.
    """
    now_iso = now_iso or dt.datetime.now(dt.UTC).isoformat()
    result_files = sorted(in_dir.glob("result_*.json"))
    if not result_files:
        print(f"no result_*.json files found in {in_dir}", file=sys.stderr)
        return 0, 0

    # Pre-fetch the set of valid comment ids so we can warn on bad IDs.
    valid_ids = {row[0] for row in conn.execute("SELECT id FROM line_comment").fetchall()}

    rows_inserted = 0
    for path in result_files:
        try:
            items = load_result_file(path)
        except (ValueError, json.JSONDecodeError) as exc:
            print(f"warning: skipping {path.name}: {exc}", file=sys.stderr)
            continue
        batch_ph = _read_prompt_hash(_batch_path_for_result(path))
        for raw_item in items:
            if not isinstance(raw_item, dict):
                print(f"warning: {path.name}: non-object entry skipped", file=sys.stderr)
                continue
            comment_id = raw_item.get("comment_id")
            if not isinstance(comment_id, int):
                print(
                    f"warning: {path.name}: missing or non-integer comment_id, skipping",
                    file=sys.stderr,
                )
                continue
            if comment_id not in valid_ids:
                print(
                    f"warning: {path.name}: comment_id {comment_id} not in line_comment, skipping",
                    file=sys.stderr,
                )
                continue
            normalized = normalize_result(raw_item)
            # Prefer the prompt_hash on the result item (subagent may pass it
            # through) but fall back to the batch file as the source of truth.
            item_ph = raw_item.get("prompt_hash")
            ph = item_ph if isinstance(item_ph, str) else batch_ph
            with transaction(conn):
                conn.execute(
                    """INSERT OR REPLACE INTO classification
                       (comment_id, taxonomy, was_addressed, rule_statement,
                        confidence, classifier_model, classified_at, raw_response,
                        prompt_hash)
                       VALUES (?,?,?,?,?,?,?,?,?)""",
                    (
                        comment_id,
                        normalized["taxonomy"],
                        normalized["was_addressed"],
                        normalized["rule_statement"],
                        normalized["confidence"],
                        CLASSIFIER_MODEL,
                        now_iso,
                        json.dumps(raw_item),
                        ph,
                    ),
                )
            rows_inserted += 1
    return rows_inserted, len(result_files)


def print_taxonomy_distribution(conn: sqlite3.Connection) -> None:
    rows = conn.execute(
        "SELECT taxonomy, COUNT(*) AS n FROM classification GROUP BY taxonomy ORDER BY n DESC"
    ).fetchall()
    if not rows:
        print("classification table is empty")
        return
    print("taxonomy distribution:")
    counts = Counter()
    for row in rows:
        counts[row["taxonomy"]] = row["n"]
        print(f"  {row['taxonomy']}: {row['n']}")


def ingest_cmd(args: argparse.Namespace) -> int:
    in_dir = Path(args.in_dir).resolve()
    if not in_dir.is_dir():
        print(f"error: {in_dir} is not a directory", file=sys.stderr)
        return 2
    conn = connect()
    rows, files = ingest_results(conn, in_dir)
    print(f"ingested {rows} classifications across {files} batch files")
    print_taxonomy_distribution(conn)
    return 0


# -----------------------------------------------------------------------------
# CLI entry
# -----------------------------------------------------------------------------


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="mode", required=True)

    p_prep = sub.add_parser("prepare", help="write per-batch JSON files for subagents")
    p_prep.add_argument("--batch-size", type=int, default=15)
    p_prep.add_argument("--out-dir", required=True)
    p_prep.set_defaults(func=prepare_cmd)

    p_ing = sub.add_parser("ingest", help="ingest result_*.json files into classification")
    p_ing.add_argument("--in-dir", required=True)
    p_ing.set_defaults(func=ingest_cmd)

    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main())
