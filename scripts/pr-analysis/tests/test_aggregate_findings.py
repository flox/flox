"""Tests for the pure functions in aggregate_findings (clustering and scoring).
The DB-touching driver is exercised by the real run."""
from scripts.pr_analysis.aggregate_findings import (
    cluster_rule_statements,
    confidence_score,
    determine_scope,
)


def test_cluster_groups_semantically_similar_rules():
    statements = [
        "Extend error enums rather than parsing strings at call sites.",
        "Add new error variants instead of string-matching downstream.",
        "Use formatdoc! for multi-line strings.",
    ]
    # MiniLM rates the two error-handling sentences ~0.58 cosine, the third
    # ~0.23/0.30 against either. Pass a lower threshold in the test so that
    # the production constant (0.65, deliberately conservative) is preserved.
    clusters = cluster_rule_statements(statements, threshold=0.5)
    assert len(clusters) == 2  # the two error-handling rules merge
    # Find the multi-member cluster
    multi = [c for c in clusters if len(c) > 1]
    assert len(multi) == 1
    assert sorted(multi[0]) == [0, 1]


def test_confidence_combines_tier_evidence_cross_area_and_acceptance():
    score = confidence_score(
        tier1_count=2, tier2_count=1,
        total_evidence=5, cross_area_count=2,
        acceptance_rate=0.8,
    )
    assert 0.7 <= score <= 1.0


def test_confidence_for_lone_taste_comment_is_low():
    score = confidence_score(
        tier1_count=0, tier2_count=0,
        total_evidence=1, cross_area_count=1,
        acceptance_rate=0.0,
    )
    assert score < 0.3


def test_scope_cross_cutting_requires_tier1_and_multi_area():
    assert determine_scope(tier1_count=1, cross_area_count=2) == "cross-cutting"
    assert determine_scope(tier1_count=0, cross_area_count=3) == "area-specific"
    assert determine_scope(tier1_count=2, cross_area_count=1) == "area-specific"


def test_agents_md_coverage_matches_substantive_rule_in_section():
    from scripts.pr_analysis.aggregate_findings import agents_md_coverage
    agents_text = (
        "# AGENTS.md\n\n"
        "## Rust style\n\n"
        "Use early returns from functions; avoid nested conditionals.\n"
        "\n## Error handling architecture\n\n"
        "Extend error enums with new variants rather than parsing strings "
        "at call sites.\n"
    )
    in_md, section = agents_md_coverage(
        "Extend error enums rather than parsing strings at call sites.",
        agents_text,
    )
    assert in_md == 1
    assert section is not None
    assert "error" in section.lower()


def test_agents_md_coverage_returns_zero_for_unrelated_rule():
    from scripts.pr_analysis.aggregate_findings import agents_md_coverage
    agents_text = (
        "## Rust style\n\nUse early returns from functions.\n"
    )
    in_md, _ = agents_md_coverage(
        "Always feed cats at 6am sharp.",
        agents_text,
    )
    assert in_md == 0


def test_is_generic_placeholder_drops_short_rules():
    from scripts.pr_analysis.aggregate_findings import _is_generic_placeholder
    assert _is_generic_placeholder("Review comment addressing code change.")
    assert _is_generic_placeholder("Fix logic errors.")
    assert not _is_generic_placeholder(
        "Extend error enums rather than parsing strings at call sites for"
        " classification."
    )


def test_agents_md_coverage_returns_zero_when_too_few_distinctive_tokens():
    """A rule with fewer than min_overlap distinctive tokens (>= 4 chars,
    non-stopword) is not eligible for matching."""
    from scripts.pr_analysis.aggregate_findings import agents_md_coverage
    agents_text = (
        "## Rust style\n\nUse early returns from functions; avoid nested conditionals.\n"
    )
    # "do it now" — only 'now' has >= 4 chars; below min_overlap=3.
    in_md, section = agents_md_coverage("do it now", agents_text)
    assert in_md == 0
    assert section is None
