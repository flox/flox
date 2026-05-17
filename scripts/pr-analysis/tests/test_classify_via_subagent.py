"""Tests for the subagent-orchestrated classifier.

Uses a tempfile SQLite DB seeded from ``schema.sql`` and monkeypatches
``lib.db.DEFAULT_DB_PATH`` so the script under test connects to it.
"""
from __future__ import annotations

import json
import sqlite3
from pathlib import Path

import pytest

from scripts.pr_analysis.classify_via_subagent import (
    CLASSIFIER_MODEL,
    ingest_results,
    normalize_result,
    write_batches,
)
from scripts.pr_analysis.lib import db as db_module
from scripts.pr_analysis.lib.db import apply_schema

REPO_ROOT = Path(__file__).resolve().parents[3]
SCHEMA_PATH = REPO_ROOT / "scripts" / "pr-analysis" / "schema.sql"


@pytest.fixture
def temp_db(tmp_path, monkeypatch):
    db_path = tmp_path / "test_pr_analysis.db"
    monkeypatch.setattr(db_module, "DEFAULT_DB_PATH", db_path)
    conn = sqlite3.connect(db_path)
    conn.row_factory = sqlite3.Row
    conn.execute("PRAGMA foreign_keys = ON")
    apply_schema(conn, SCHEMA_PATH)
    yield conn
    conn.close()


def _insert_pr(conn: sqlite3.Connection, number: int) -> None:
    conn.execute(
        """INSERT INTO pr (number, title, author, author_type, state, merged_at, url, fetched_at)
           VALUES (?,?,?,?,?,?,?,?)""",
        (number, "t", "a", "User", "merged", "2026-01-01T00:00:00Z",
         f"https://x/{number}", "2026-01-01T00:00:00Z"),
    )


def _insert_comment(
    conn: sqlite3.Connection,
    *,
    cid: int,
    pr_number: int,
    body: str = "body",
    diff_hunk: str = "@@ hunk",
    area: str = "commands",
    reviewer_tier: int = 2,
) -> None:
    conn.execute(
        """INSERT INTO line_comment
           (id, pr_number, author, author_type, created_at, path, body, area, reviewer_tier)
           VALUES (?,?,?,?,?,?,?,?,?)""",
        (cid, pr_number, "reviewer", "User", "2026-01-01T00:00:00Z",
         "src/x.rs", body, area, reviewer_tier),
    )
    if diff_hunk is not None:
        conn.execute(
            "UPDATE line_comment SET diff_hunk = ? WHERE id = ?",
            (diff_hunk, cid),
        )


def test_prepare_writes_batches_and_manifest(temp_db, tmp_path):
    _insert_pr(temp_db, 1)
    # Five comments; two have tier 4 (must be skipped); one is already classified.
    _insert_comment(temp_db, cid=10, pr_number=1, body="b10")
    _insert_comment(temp_db, cid=11, pr_number=1, body="b11")
    _insert_comment(temp_db, cid=12, pr_number=1, body="b12")
    _insert_comment(temp_db, cid=13, pr_number=1, body="b13", reviewer_tier=4)
    _insert_comment(temp_db, cid=14, pr_number=1, body="b14")
    temp_db.execute(
        """INSERT INTO classification
           (comment_id, taxonomy, was_addressed, rule_statement, confidence,
            classifier_model, classified_at)
           VALUES (?,?,?,?,?,?,?)""",
        (12, "naming", 1, "x", 0.5, "pre-existing", "2026-01-01T00:00:00Z"),
    )
    temp_db.commit()

    out_dir = tmp_path / "batches"
    from scripts.pr_analysis.classify_via_subagent import fetch_unclassified
    comments = fetch_unclassified(temp_db)
    # 10, 11, 14 are eligible (12 already classified, 13 is tier 4).
    assert [c["id"] for c in comments] == [10, 11, 14]

    written = write_batches(comments, batch_size=2, out_dir=out_dir)
    assert [p.name for p, _ in written] == ["batch_1.json", "batch_2.json"]
    assert [n for _, n in written] == [2, 1]

    payload = json.loads((out_dir / "batch_1.json").read_text())
    assert payload["batch_id"] == 1
    assert isinstance(payload["system_prompt"], str) and "JSON" in payload["system_prompt"]
    assert "error-handling" in payload["taxonomy_block"]
    assert len(payload["comments"]) == 2
    expected_first = {
        "id": 10,
        "body": "b10",
        "diff_hunk": "@@ hunk",
        "final_code_snippet": "",
        "area": "commands",
    }
    assert payload["comments"][0] == expected_first


def test_ingest_handles_unknown_taxonomy(temp_db, tmp_path):
    _insert_pr(temp_db, 2)
    _insert_comment(temp_db, cid=20, pr_number=2)
    temp_db.commit()

    result_path = tmp_path / "result_1.json"
    result_path.write_text(json.dumps([
        {
            "comment_id": 20,
            "taxonomy": "made-up",
            "was_addressed": True,
            "rule_statement": "do a thing",
            "confidence": 0.5,
        }
    ]))

    rows, files = ingest_results(temp_db, tmp_path)
    assert (rows, files) == (1, 1)
    row = temp_db.execute(
        "SELECT taxonomy, was_addressed, classifier_model FROM classification WHERE comment_id = ?",
        (20,),
    ).fetchone()
    assert dict(row) == {
        "taxonomy": "other",
        "was_addressed": 1,
        "classifier_model": CLASSIFIER_MODEL,
    }


def test_ingest_clamps_confidence(temp_db, tmp_path):
    _insert_pr(temp_db, 3)
    _insert_comment(temp_db, cid=30, pr_number=3)
    _insert_comment(temp_db, cid=31, pr_number=3)
    temp_db.commit()

    result_path = tmp_path / "result_1.json"
    result_path.write_text(json.dumps([
        {
            "comment_id": 30,
            "taxonomy": "naming",
            "was_addressed": False,
            "rule_statement": "x",
            "confidence": 1.5,
        },
        {
            "comment_id": 31,
            "taxonomy": "naming",
            "was_addressed": None,
            "rule_statement": "y",
            "confidence": -0.5,
        },
    ]))

    rows, _ = ingest_results(temp_db, tmp_path)
    assert rows == 2
    confidences = {
        r["comment_id"]: r["confidence"]
        for r in temp_db.execute(
            "SELECT comment_id, confidence FROM classification ORDER BY comment_id"
        ).fetchall()
    }
    assert confidences == {30: 1.0, 31: 0.0}


def test_ingest_is_idempotent(temp_db, tmp_path):
    _insert_pr(temp_db, 4)
    _insert_comment(temp_db, cid=40, pr_number=4)
    _insert_comment(temp_db, cid=41, pr_number=4)
    temp_db.commit()

    result_path = tmp_path / "result_1.json"
    result_path.write_text(json.dumps([
        {
            "comment_id": 40,
            "taxonomy": "naming",
            "was_addressed": True,
            "rule_statement": "x",
            "confidence": 0.8,
        },
        {
            "comment_id": 41,
            "taxonomy": "testing",
            "was_addressed": False,
            "rule_statement": "y",
            "confidence": 0.7,
        },
    ]))

    rows_first, files_first = ingest_results(temp_db, tmp_path)
    rows_second, files_second = ingest_results(temp_db, tmp_path)

    assert (rows_first, files_first) == (2, 1)
    assert (rows_second, files_second) == (2, 1)

    total = temp_db.execute("SELECT COUNT(*) FROM classification").fetchone()[0]
    assert total == 2


def test_normalize_result_defaults_missing_fields():
    out = normalize_result({"comment_id": 1})
    assert out == {
        "taxonomy": "other",
        "was_addressed": None,
        "rule_statement": "",
        "confidence": 0.0,
    }
