"""Anthropic API wrapper used by classify and synthesize stages."""
from __future__ import annotations

import os
import time

from anthropic import Anthropic, APIError

CLASSIFY_MODEL = "claude-haiku-4-5-20251001"
SYNTH_MODEL = "claude-sonnet-4-6"


def client() -> Anthropic:
    api_key = os.environ.get("ANTHROPIC_API_KEY")
    if not api_key:
        raise RuntimeError("ANTHROPIC_API_KEY not set")
    return Anthropic(api_key=api_key)


def call_with_retry(
    anthropic: Anthropic,
    *,
    model: str,
    system: str,
    user: str,
    max_tokens: int = 800,
    attempts: int = 4,
) -> str:
    last_exc: Exception | None = None
    for i in range(attempts):
        try:
            msg = anthropic.messages.create(
                model=model,
                max_tokens=max_tokens,
                system=system,
                messages=[{"role": "user", "content": user}],
            )
            return msg.content[0].text
        except APIError as exc:
            last_exc = exc
            time.sleep(2 ** i)
    assert last_exc is not None
    raise last_exc
