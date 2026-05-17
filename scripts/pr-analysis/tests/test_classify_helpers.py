from scripts.pr_analysis.lib.classify_helpers import prompt_hash


def test_prompt_hash_is_stable():
    h1 = prompt_hash("system", "tax")
    h2 = prompt_hash("system", "tax")
    assert h1 == h2


def test_prompt_hash_differs_with_system_change():
    h1 = prompt_hash("system A", "tax")
    h2 = prompt_hash("system B", "tax")
    assert h1 != h2


def test_prompt_hash_differs_with_taxonomy_change():
    h1 = prompt_hash("system", "tax A")
    h2 = prompt_hash("system", "tax B")
    assert h1 != h2


def test_prompt_hash_returns_hex_sha256():
    h = prompt_hash("a", "b")
    assert len(h) == 64
    int(h, 16)  # raises if not hex
