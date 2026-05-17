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
