"""Pure-logic tests for snippet extraction. Network-touching code is exercised
via the real run, not unit-mocked."""
from scripts.pr_analysis.ingest_final_code import extract_window


def test_extract_window_returns_n_lines_around_anchor():
    lines = [f"line {i}" for i in range(1, 101)]
    snippet = extract_window(lines, anchor_line=50, radius=5)
    out = snippet.splitlines()
    assert out[0].startswith("45:")
    assert out[-1].startswith("55:")
    assert len(out) == 11


def test_extract_window_clamps_at_file_start():
    lines = [f"line {i}" for i in range(1, 11)]
    snippet = extract_window(lines, anchor_line=2, radius=5)
    out = snippet.splitlines()
    assert out[0].startswith("1:")
    assert out[-1].startswith("7:")


def test_extract_window_clamps_at_file_end():
    lines = [f"line {i}" for i in range(1, 11)]
    snippet = extract_window(lines, anchor_line=9, radius=5)
    out = snippet.splitlines()
    assert out[0].startswith("4:")
    assert out[-1].startswith("10:")


def test_extract_window_returns_empty_for_none_anchor():
    assert extract_window(["a", "b"], anchor_line=None, radius=5) == ""
