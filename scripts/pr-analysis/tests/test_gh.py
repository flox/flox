import json
from pathlib import Path

from scripts.pr_analysis.lib.gh import GhError, run_json

FIXTURES = Path(__file__).parent / "fixtures"


def test_fixture_pr_list_is_a_well_formed_list_of_dicts():
    data = json.loads((FIXTURES / "pr_list_sample.json").read_text())
    assert isinstance(data, list)
    for pr in data:
        assert {"number", "title", "author", "mergedAt", "url", "files"} <= set(pr.keys())
        assert isinstance(pr["files"], list)


def test_gh_error_is_a_subclass_of_runtime_error():
    assert issubclass(GhError, RuntimeError)


def test_run_json_propagates_gh_failures(monkeypatch):
    import subprocess as sp
    class FakeProc:
        returncode = 1
        stdout = ""
        stderr = "boom"
    monkeypatch.setattr(sp, "run", lambda *a, **kw: FakeProc())
    try:
        run_json(["bogus"])
    except GhError as exc:
        assert "boom" in str(exc)
        return
    raise AssertionError("expected GhError")


def test_pr_4231_comments_fixture_has_expected_fields():
    raw = (FIXTURES / "pr_4231_comments.json").read_text()
    # gh --paginate concatenates JSON arrays back-to-back; split and parse.
    arrays = []
    decoder = json.JSONDecoder()
    idx = 0
    while idx < len(raw):
        while idx < len(raw) and raw[idx].isspace():
            idx += 1
        if idx >= len(raw):
            break
        val, end = decoder.raw_decode(raw, idx)
        arrays.append(val)
        idx = end
    comments = [c for arr in arrays for c in arr]
    assert len(comments) >= 30  # PR #4231 has ~63
    for c in comments[:5]:
        assert {"id", "user", "path", "body", "diff_hunk", "created_at"} <= set(c.keys())
