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
    compare_batch_and_result,
    ingest_results,
    normalize_result,
    prepare_missing_batch,
    write_batches,
    write_missing_json,
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
        "thread_resolved": 0,
        "thread_resolved_by": None,
    }
    assert payload["comments"][0] == expected_first
    # prompt_hash is a stable 64-char hex SHA256 of system_prompt + taxonomy.
    assert isinstance(payload["prompt_hash"], str)
    assert len(payload["prompt_hash"]) == 64
    int(payload["prompt_hash"], 16)  # raises if not hex
    # Both batches should pin to the same hash.
    payload2 = json.loads((out_dir / "batch_2.json").read_text())
    assert payload2["prompt_hash"] == payload["prompt_hash"]


def test_prepare_batch_includes_thread_resolution_fields(temp_db, tmp_path):
    """Each comment in a batch file must include thread_resolved and
    thread_resolved_by so the subagent can use them as a hint."""
    _insert_pr(temp_db, 7)
    _insert_comment(temp_db, cid=70, pr_number=7, body="b70")
    _insert_comment(temp_db, cid=71, pr_number=7, body="b71")
    # Mark 70 as resolved-by-reviewer; 71 unresolved.
    temp_db.execute(
        "UPDATE line_comment SET thread_resolved = 1, thread_resolved_by = ? WHERE id = ?",
        ("reviewer", 70),
    )
    temp_db.commit()

    out_dir = tmp_path / "batches"
    from scripts.pr_analysis.classify_via_subagent import fetch_unclassified
    comments = fetch_unclassified(temp_db)
    write_batches(comments, batch_size=10, out_dir=out_dir)

    payload = json.loads((out_dir / "batch_1.json").read_text())
    by_id = {c["id"]: c for c in payload["comments"]}
    assert "thread_resolved" in by_id[70]
    assert "thread_resolved_by" in by_id[70]
    assert by_id[70]["thread_resolved"] == 1
    assert by_id[70]["thread_resolved_by"] == "reviewer"
    assert by_id[71]["thread_resolved"] == 0
    assert by_id[71]["thread_resolved_by"] is None


def test_ingest_handles_unknown_taxonomy(temp_db, tmp_path):
    _insert_pr(temp_db, 2)
    _insert_comment(temp_db, cid=20, pr_number=2)
    temp_db.commit()

    batch_path = tmp_path / "batch_1.json"
    batch_path.write_text(json.dumps({
        "batch_id": 1,
        "prompt_hash": "a" * 64,
        "system_prompt": "sp",
        "taxonomy_block": "tb",
        "comments": [],
    }))
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

    rows, files, missing = ingest_results(temp_db, tmp_path)
    assert (rows, files, missing) == (1, 1, {})
    row = temp_db.execute(
        "SELECT taxonomy, was_addressed, classifier_model, prompt_hash FROM classification WHERE comment_id = ?",
        (20,),
    ).fetchone()
    assert dict(row) == {
        "taxonomy": "other",
        "was_addressed": 1,
        "classifier_model": CLASSIFIER_MODEL,
        "prompt_hash": "a" * 64,
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

    rows, _, _ = ingest_results(temp_db, tmp_path)
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

    rows_first, files_first, missing_first = ingest_results(temp_db, tmp_path)
    rows_second, files_second, missing_second = ingest_results(temp_db, tmp_path)

    assert (rows_first, files_first, missing_first) == (2, 1, {})
    assert (rows_second, files_second, missing_second) == (2, 1, {})

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


def test_ingest_persists_prompt_hash_from_result_item(temp_db, tmp_path):
    """If the subagent passes prompt_hash through on the result item, that
    value wins over the batch file's value."""
    _insert_pr(temp_db, 5)
    _insert_comment(temp_db, cid=50, pr_number=5)
    temp_db.commit()

    batch_path = tmp_path / "batch_1.json"
    batch_path.write_text(json.dumps({
        "batch_id": 1,
        "prompt_hash": "b" * 64,
        "system_prompt": "sp",
        "taxonomy_block": "tb",
        "comments": [],
    }))
    result_path = tmp_path / "result_1.json"
    result_path.write_text(json.dumps([
        {
            "comment_id": 50,
            "taxonomy": "naming",
            "was_addressed": True,
            "rule_statement": "r",
            "confidence": 0.9,
            "prompt_hash": "c" * 64,
        }
    ]))

    rows, _, _ = ingest_results(temp_db, tmp_path)
    assert rows == 1
    ph = temp_db.execute(
        "SELECT prompt_hash FROM classification WHERE comment_id = ?", (50,),
    ).fetchone()["prompt_hash"]
    assert ph == "c" * 64


def test_ingest_persists_null_prompt_hash_when_batch_missing(temp_db, tmp_path):
    """If no batch file is present and the result lacks prompt_hash, the
    column is written as NULL rather than blowing up."""
    _insert_pr(temp_db, 6)
    _insert_comment(temp_db, cid=60, pr_number=6)
    temp_db.commit()

    result_path = tmp_path / "result_1.json"
    result_path.write_text(json.dumps([
        {
            "comment_id": 60,
            "taxonomy": "naming",
            "was_addressed": True,
            "rule_statement": "r",
            "confidence": 0.9,
        }
    ]))

    rows, _, _ = ingest_results(temp_db, tmp_path)
    assert rows == 1
    ph = temp_db.execute(
        "SELECT prompt_hash FROM classification WHERE comment_id = ?", (60,),
    ).fetchone()["prompt_hash"]
    assert ph is None


def test_compare_batch_and_result_detects_missing_and_extra(tmp_path):
    """compare_batch_and_result reports IDs in batch but not result (missing)
    and IDs in result but not batch (extra)."""
    batch = {
        "batch_id": 1,
        "prompt_hash": "abc",
        "system_prompt": "sp",
        "taxonomy_block": "tb",
        "comments": [
            {"id": 1, "body": "b1", "diff_hunk": "", "final_code_snippet": "", "area": "commands"},
            {"id": 2, "body": "b2", "diff_hunk": "", "final_code_snippet": "", "area": "commands"},
            {"id": 3, "body": "b3", "diff_hunk": "", "final_code_snippet": "", "area": "commands"},
        ],
    }
    (tmp_path / "batch_1.json").write_text(json.dumps(batch))
    # Result has id 1, 2, 99 — id 3 missing, id 99 is extra.
    result = [
        {"comment_id": 1, "taxonomy": "naming", "was_addressed": True, "rule_statement": "r1", "confidence": 0.7},
        {"comment_id": 2, "taxonomy": "testing", "was_addressed": True, "rule_statement": "r2", "confidence": 0.7},
        {"comment_id": 99, "taxonomy": "naming", "was_addressed": True, "rule_statement": "r99", "confidence": 0.7},
    ]
    (tmp_path / "result_1.json").write_text(json.dumps(result))

    diff = compare_batch_and_result(tmp_path / "batch_1.json", tmp_path / "result_1.json")
    assert diff == {
        "batch_ids": [1, 2, 3],
        "result_ids": [1, 2, 99],
        "missing": [3],
        "extra": [99],
    }


def test_ingest_detects_missing_comments(temp_db, tmp_path):
    """When a result file is missing a comment_id from the batch, ingest
    writes a missing.json with the gap and returns it in missing_by_batch."""
    _insert_pr(temp_db, 8)
    _insert_comment(temp_db, cid=1, pr_number=8, body="b1")
    _insert_comment(temp_db, cid=2, pr_number=8, body="b2")
    _insert_comment(temp_db, cid=3, pr_number=8, body="b3")
    temp_db.commit()

    batch = {
        "batch_id": 1,
        "prompt_hash": "a" * 64,
        "system_prompt": "sp",
        "taxonomy_block": "tb",
        "comments": [
            {"id": 1, "body": "b1", "diff_hunk": "", "final_code_snippet": "", "area": "commands"},
            {"id": 2, "body": "b2", "diff_hunk": "", "final_code_snippet": "", "area": "commands"},
            {"id": 3, "body": "b3", "diff_hunk": "", "final_code_snippet": "", "area": "commands"},
        ],
    }
    (tmp_path / "batch_1.json").write_text(json.dumps(batch))
    # Result has only ids 1, 2 — id 3 dropped
    result = [
        {"comment_id": 1, "taxonomy": "naming", "was_addressed": True, "rule_statement": "r1", "confidence": 0.7},
        {"comment_id": 2, "taxonomy": "testing", "was_addressed": True, "rule_statement": "r2", "confidence": 0.7},
    ]
    (tmp_path / "result_1.json").write_text(json.dumps(result))

    rows, files, missing = ingest_results(temp_db, tmp_path)
    assert rows == 2
    assert files == 1
    assert missing == {"1": [3]}

    # Now simulate ingest_cmd's missing.json writeout.
    path = write_missing_json(tmp_path, missing)
    payload = json.loads(path.read_text())
    assert payload["missing_comment_ids"] == [3]
    assert payload["by_batch"] == {"1": [3]}
    assert "Re-prepare" in payload["instructions"]


def test_ingest_globs_retry_results(temp_db, tmp_path):
    """ingest must pick up result_retry.json alongside numeric result files."""
    _insert_pr(temp_db, 9)
    _insert_comment(temp_db, cid=100, pr_number=9, body="b100")
    _insert_comment(temp_db, cid=101, pr_number=9, body="b101")
    temp_db.commit()

    # Original batch with id 100 only; result has 100 only.
    (tmp_path / "batch_1.json").write_text(json.dumps({
        "batch_id": 1,
        "prompt_hash": "a" * 64,
        "system_prompt": "sp",
        "taxonomy_block": "tb",
        "comments": [
            {"id": 100, "body": "b100", "diff_hunk": "", "final_code_snippet": "", "area": "commands"},
        ],
    }))
    (tmp_path / "result_1.json").write_text(json.dumps([
        {"comment_id": 100, "taxonomy": "naming", "was_addressed": True, "rule_statement": "r", "confidence": 0.7},
    ]))
    # Retry batch + result for id 101.
    (tmp_path / "batch_retry.json").write_text(json.dumps({
        "batch_id": "retry",
        "prompt_hash": "a" * 64,
        "system_prompt": "sp",
        "taxonomy_block": "tb",
        "comments": [
            {"id": 101, "body": "b101", "diff_hunk": "", "final_code_snippet": "", "area": "commands"},
        ],
    }))
    (tmp_path / "result_retry.json").write_text(json.dumps([
        {"comment_id": 101, "taxonomy": "testing", "was_addressed": True, "rule_statement": "r", "confidence": 0.7},
    ]))

    rows, files, missing = ingest_results(temp_db, tmp_path)
    assert rows == 2
    assert files == 2
    assert missing == {}


def test_prepare_missing_batch_pulls_full_context(temp_db):
    """prepare_missing_batch reads each missing comment's full context from
    the DB and emits a batch payload in the original shape."""
    _insert_pr(temp_db, 11)
    _insert_comment(
        temp_db, cid=210, pr_number=11, body="body-210",
        diff_hunk="@@ hunk 210", area="commands",
    )
    _insert_comment(
        temp_db, cid=211, pr_number=11, body="body-211",
        diff_hunk="@@ hunk 211", area="sdk",
    )
    # Mark 210 as resolved-by-author.
    temp_db.execute(
        "UPDATE line_comment SET thread_resolved = 1, thread_resolved_by = ? WHERE id = ?",
        ("author", 210),
    )
    # Attach a final_code_snippet for 211.
    temp_db.execute(
        "INSERT INTO comment_final_code (comment_id, final_code_snippet, snippet_available) VALUES (?, ?, ?)",
        (211, "fn final_211() {}", 1),
    )
    temp_db.commit()

    batch = prepare_missing_batch(temp_db, [210, 211])
    assert batch["batch_id"] == "retry"
    assert isinstance(batch["prompt_hash"], str) and len(batch["prompt_hash"]) == 64
    assert isinstance(batch["system_prompt"], str)
    assert isinstance(batch["taxonomy_block"], str)
    by_id = {c["id"]: c for c in batch["comments"]}
    assert by_id[210] == {
        "id": 210,
        "body": "body-210",
        "diff_hunk": "@@ hunk 210",
        "final_code_snippet": "",
        "area": "commands",
        "thread_resolved": 1,
        "thread_resolved_by": "author",
    }
    assert by_id[211] == {
        "id": 211,
        "body": "body-211",
        "diff_hunk": "@@ hunk 211",
        "final_code_snippet": "fn final_211() {}",
        "area": "sdk",
        "thread_resolved": 0,
        "thread_resolved_by": None,
    }
