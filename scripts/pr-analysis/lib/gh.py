"""Thin subprocess wrapper around the `gh` CLI returning parsed JSON."""
from __future__ import annotations

import json
import subprocess
from typing import Any


class GhError(RuntimeError):
    pass


def run_json(args: list[str]) -> Any:
    """Invoke gh with the given args; return parsed JSON. Raises GhError on failure."""
    proc = subprocess.run(
        ["gh", *args],
        capture_output=True,
        text=True,
        check=False,
    )
    if proc.returncode != 0:
        raise GhError(f"gh {' '.join(args)} failed: {proc.stderr.strip()}")
    if not proc.stdout.strip():
        return None
    try:
        return json.loads(proc.stdout)
    except json.JSONDecodeError as exc:
        raise GhError(f"gh {' '.join(args)} returned non-JSON: {exc}") from exc


def paginate_jsonl(args: list[str]) -> list[Any]:
    """Invoke gh with --paginate and parse each line as JSON.

    `gh api --paginate -q '.[]'` emits one JSON value per line per array element
    across all pages.
    """
    proc = subprocess.run(
        ["gh", *args],
        capture_output=True,
        text=True,
        check=False,
    )
    if proc.returncode != 0:
        raise GhError(f"gh {' '.join(args)} failed: {proc.stderr.strip()}")
    out = []
    for line in proc.stdout.splitlines():
        line = line.strip()
        if not line:
            continue
        out.append(json.loads(line))
    return out
