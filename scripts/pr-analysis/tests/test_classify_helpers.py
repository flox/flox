from scripts.pr_analysis.lib.classify_helpers import build_user_prompt, prompt_hash


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


def test_build_user_prompt_mentions_thread_resolution_state():
    prompt = build_user_prompt(
        body="please rename foo to bar",
        diff_hunk="@@ hunk",
        final_snippet="let bar = ...;",
        taxonomy_block="- naming: ...",
        thread_resolved=True,
        thread_resolved_by="alice",
    )
    assert "Review-thread resolution state" in prompt
    assert "resolved" in prompt
    assert "alice" in prompt


def test_build_user_prompt_unresolved_thread():
    prompt = build_user_prompt(
        body="b", diff_hunk="d", final_snippet="s", taxonomy_block="t",
        thread_resolved=False,
    )
    assert "Review-thread resolution state: unresolved" in prompt


def test_build_user_prompt_unknown_thread_state_when_none():
    prompt = build_user_prompt(
        body="b", diff_hunk="d", final_snippet="s", taxonomy_block="t",
    )
    assert "Review-thread resolution state: unknown" in prompt
