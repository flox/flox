import pytest
from scripts.pr_analysis.classify_comments import parse


def test_well_formed_json_parses():
    raw = '{"taxonomy":"error-handling","was_addressed":true,"rule_statement":"Extend error enums rather than parsing strings at call sites.","confidence":0.85}'
    out = parse(raw)
    assert out["taxonomy"] == "error-handling"
    assert out["was_addressed"] is True
    assert "Extend error enums" in out["rule_statement"]
    assert out["confidence"] == 0.85


def test_unknown_taxonomy_falls_back_to_other():
    raw = '{"taxonomy":"made-up","was_addressed":false,"rule_statement":"x","confidence":0.4}'
    assert parse(raw)["taxonomy"] == "other"


def test_extracts_object_when_wrapped_in_prose():
    raw = 'Here is the JSON: {"taxonomy":"naming","was_addressed":null,"rule_statement":"Helpers follow the str_to_x convention in flox-catalog.","confidence":0.7} done.'
    out = parse(raw)
    assert out["taxonomy"] == "naming"
    assert out["was_addressed"] is None


def test_missing_fields_default_safely():
    raw = '{"taxonomy":"naming"}'
    out = parse(raw)
    assert out["was_addressed"] is None
    assert out["rule_statement"] == ""
    assert out["confidence"] == 0.0


def test_non_json_response_raises():
    with pytest.raises(ValueError):
        parse("no object here")
