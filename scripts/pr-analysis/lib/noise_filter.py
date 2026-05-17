"""Recognize PR review comments that teach no rule and should skip classification.

A comment is noise when its body is dominated by content that cannot encode a
review rule: a bare GitHub commit URL, a `suggestion` code block with no prose,
or a single emoji/praise prefix with no substance.

False positives here cost analysis recall; false negatives waste classifier
calls. Bias toward false negatives — only filter what is unambiguously noise.
"""
from __future__ import annotations

import re

# A bare commit/file URL with optional whitespace.
_URL_ONLY = re.compile(
    r"^\s*https?://github\.com/[\w.-]+/[\w.-]+/(?:commit|pull)/[^\s]+\s*$"
)

# Only a fenced ```suggestion ... ``` block, optionally surrounded by whitespace
# and the GitHub-suggested-changes UI fragment.
_SUGGESTION_ONLY = re.compile(
    r"^\s*```suggestion\b.*?```\s*$",
    re.DOTALL,
)

# Praise/nit prefix with no substantive body.
_PRAISE_NIT_ONLY = re.compile(
    r"^\s*\(?(?:praise|nit|question|chore|thought|minor|style)\)?\s*[:.\-]?\s*"
    r"[\w\s.,!👍🏻👏🎉✨🚀✅❌⚠️🤖ℹ️✅⚠️-]{0,40}\s*$",
    re.IGNORECASE,
)

# Emoji-only / single-word approval ("lgtm", "thanks", "👍🏻").
_LGTM_ONLY = re.compile(
    r"^\s*(?:lgtm|thanks|thx|ty|sgtm|done|fixed|👍|👍🏻|👏|✅|🎉|💯|🚀|"
    r"looks?\s+good|great|nice|cool)[!.]?\s*$",
    re.IGNORECASE,
)


def is_noise(body: str) -> bool:
    """Return True if the comment body cannot plausibly teach a review rule."""
    if not body or not body.strip():
        return True
    if _URL_ONLY.match(body):
        return True
    if _SUGGESTION_ONLY.match(body):
        return True
    if _LGTM_ONLY.match(body):
        return True
    if _PRAISE_NIT_ONLY.match(body):
        return True
    return False
