"""Tests for the pure functions in aggregate_findings (clustering and scoring).
The DB-touching driver is exercised by the real run."""
from scripts.pr_analysis.aggregate_findings import (
    cluster_rule_statements,
    confidence_score,
    determine_scope,
)


def test_cluster_groups_near_duplicates_via_normalized_prefix():
    statements = [
        "Extend error enums rather than parsing strings at call sites.",
        "Extend error enums rather than string-matching at call sites.",  # near-dup
        "Use formatdoc! for multi-line strings.",
    ]
    clusters = cluster_rule_statements(statements, threshold=0.6)
    assert len(clusters) == 2


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
