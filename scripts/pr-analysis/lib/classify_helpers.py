"""Prompt construction and response parsing shared by classifier variants.

Kept dependency-free (no `anthropic`) so it can be imported by the
subagent-orchestrated classifier without requiring the SDK.
"""
from __future__ import annotations

import hashlib
import json

from lib.taxonomy import TAXONOMY, TAXONOMY_IDS


def prompt_hash(system_prompt: str, taxonomy_block_text: str) -> str:
    """Stable SHA256 of the prompt configuration. Lets us tell whether two
    classification runs used the same prompt + taxonomy."""
    payload = (system_prompt + "\n---\n" + taxonomy_block_text).encode("utf-8")
    return hashlib.sha256(payload).hexdigest()

SYSTEM_PROMPT = (
    "You analyze Rust PR review comments for the flox/flox repository. "
    "For each comment you receive the body, the original diff hunk it pointed at, "
    "and the code that exists at that location in the merged result. "
    "You output strict JSON with this exact shape: "
    '{"taxonomy": "<one of the allowed ids>", '
    '"was_addressed": true|false|null, '
    '"rule_statement": "<one-sentence imperative rule, present tense, <= 160 chars>", '
    '"confidence": 0.0..1.0}. '
    "If the comment is conversational only (e.g. 'nit', 'lgtm', 'thanks') and teaches no rule, "
    'set taxonomy="other", rule_statement="" and confidence < 0.3. '
    "Do not emit any text outside the JSON object."
)


def build_user_prompt(body: str, diff_hunk: str, final_snippet: str, taxonomy_block: str) -> str:
    return (
        f"Allowed taxonomy ids:\n{taxonomy_block}\n\n"
        "Review comment body:\n"
        "```\n" + body + "\n```\n\n"
        "Original diff hunk it pointed at:\n"
        "```\n" + (diff_hunk or "(no hunk)") + "\n```\n\n"
        "Code at that location in the merged result:\n"
        "```\n" + (final_snippet or "(snippet unavailable)") + "\n```\n\n"
        "Emit the JSON object now."
    )


def taxonomy_block() -> str:
    return "\n".join(f"- {t.id}: {t.description}" for t in TAXONOMY)


def parse(raw: str) -> dict:
    # Be lenient: find the first { and the matching closing }.
    start = raw.find("{")
    end = raw.rfind("}")
    if start == -1 or end == -1:
        raise ValueError(f"no JSON object in response: {raw[:200]}")
    obj = json.loads(raw[start:end + 1])
    if obj.get("taxonomy") not in TAXONOMY_IDS:
        obj["taxonomy"] = "other"
    if "was_addressed" not in obj:
        obj["was_addressed"] = None
    if "rule_statement" not in obj:
        obj["rule_statement"] = ""
    if "confidence" not in obj:
        obj["confidence"] = 0.0
    return obj
