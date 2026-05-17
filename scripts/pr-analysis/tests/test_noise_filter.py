from scripts.pr_analysis.lib.noise_filter import is_noise


def test_bare_commit_url_is_noise():
    assert is_noise("https://github.com/flox/flox/pull/4231/commits/abc123def")


def test_url_with_surrounding_whitespace_is_noise():
    assert is_noise("\n  https://github.com/flox/flox/commit/abc123  \n")


def test_suggestion_only_is_noise():
    assert is_noise("```suggestion\nfoo bar\n```")
    assert is_noise("```suggestion\n# multi-line\nlet x = 1;\n```")


def test_lgtm_is_noise():
    for body in ("LGTM", "lgtm!", "thanks", "👍🏻", "✅", "looks good", "nice"):
        assert is_noise(body), f"expected noise: {body!r}"


def test_praise_prefix_only_is_noise():
    assert is_noise("(praise) This approach is much better 👍🏻")
    assert is_noise("nit: typo")


def test_real_rule_comment_is_not_noise():
    # A substantive comment that the classifier should see.
    body = (
        "Consider using `formatdoc!` here instead of string concatenation; "
        "it preserves indentation and matches the project convention in "
        "AGENTS.md."
    )
    assert not is_noise(body)


def test_suggestion_block_with_prose_is_not_noise():
    # Suggestion + accompanying explanation = real rule.
    body = (
        "We should extend the error enum rather than parsing here.\n\n"
        "```suggestion\nGitRemoteCommandError::AccessDenied\n```"
    )
    assert not is_noise(body)


def test_empty_body_is_noise():
    assert is_noise("")
    assert is_noise("   \n   ")
