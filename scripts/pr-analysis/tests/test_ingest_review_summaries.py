"""Shape sanity test for the review-summary ingest fixture."""
import json
from pathlib import Path

FIXTURES = Path(__file__).parent / "fixtures"


def _parse_concatenated_arrays(raw: str) -> list[dict]:
    """gh --paginate concatenates JSON arrays back-to-back; split and flatten."""
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
    return [item for arr in arrays for item in arr]


def test_pr_4231_reviews_fixture_has_expected_fields():
    raw = (FIXTURES / "pr_4231_reviews.json").read_text()
    reviews = _parse_concatenated_arrays(raw)
    assert len(reviews) >= 1
    for r in reviews:
        assert {"id", "user", "state", "submitted_at"} <= set(r.keys())
        # body may be empty/null on approval-only reviews, but the key exists
        assert "body" in r
