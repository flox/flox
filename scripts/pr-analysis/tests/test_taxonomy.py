from scripts.pr_analysis.lib.taxonomy import TAXONOMY, TAXONOMY_IDS, TAXONOMY_BY_ID


def test_taxonomy_ids_are_unique():
    assert len(TAXONOMY_IDS) == len(set(TAXONOMY_IDS))


def test_taxonomy_has_open_bucket():
    assert "other" in TAXONOMY_IDS


def test_taxonomy_has_core_agents_md_categories():
    required = {"error-handling", "type-safety", "user-facing-messages", "naming"}
    assert required <= set(TAXONOMY_IDS)


def test_lookup_by_id_returns_entry():
    entry = TAXONOMY_BY_ID["error-handling"]
    assert "Error type hierarchy" in entry.description


def test_imports_taxonomy_covers_organization_concerns():
    from scripts.pr_analysis.lib.taxonomy import TAXONOMY_BY_ID
    entry = TAXONOMY_BY_ID["imports"]
    desc = entry.description.lower()
    # Should cover multiple import-related concerns, not just ::-qualification.
    assert "use" in desc
    assert "module" in desc or "placement" in desc or "where" in desc
    assert "re-export" in desc or "grouping" in desc or "organization" in desc
