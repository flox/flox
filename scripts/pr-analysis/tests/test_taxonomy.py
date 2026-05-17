from scripts.pr_analysis.lib.taxonomy import TAXONOMY, TAXONOMY_IDS, TAXONOMY_BY_ID


def taxonomy_ids_are_unique():
    assert len(TAXONOMY_IDS) == len(set(TAXONOMY_IDS))


def taxonomy_has_open_bucket():
    assert "other" in TAXONOMY_IDS


def taxonomy_has_core_agents_md_categories():
    required = {"error-handling", "type-safety", "user-facing-messages", "naming"}
    assert required <= set(TAXONOMY_IDS)


def lookup_by_id_returns_entry():
    entry = TAXONOMY_BY_ID["error-handling"]
    assert "Error type hierarchy" in entry.description
