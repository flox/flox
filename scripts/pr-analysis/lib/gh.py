"""Thin subprocess wrapper around the `gh` CLI returning parsed JSON."""
from __future__ import annotations

import json
import subprocess
from typing import Any


class GhError(RuntimeError):
    pass


def _run(args: list[str]) -> str:
    """Invoke gh with the given args; return stdout as text. Raises GhError on non-zero exit."""
    proc = subprocess.run(
        ["gh", *args],
        capture_output=True,
        text=True,
        check=False,
    )
    if proc.returncode != 0:
        raise GhError(f"gh {' '.join(args)} failed: {proc.stderr.strip()}")
    return proc.stdout


def run_json(args: list[str]) -> Any:
    """Invoke gh with the given args; return parsed JSON. Raises GhError on failure."""
    stdout = _run(args)
    if not stdout.strip():
        return None
    try:
        return json.loads(stdout)
    except json.JSONDecodeError as exc:
        raise GhError(f"gh {' '.join(args)} returned non-JSON: {exc}") from exc


def paginate_jsonl(args: list[str]) -> list[Any]:
    """Invoke gh and parse each line of stdout as JSON.

    Caller is responsible for passing `--paginate` and a `-q` filter that
    yields one JSON value per line (typically `-q '.[]'`); without those,
    a single JSON document is emitted and this function will raise GhError
    when the multi-line parse fails.
    """
    stdout = _run(args)
    out: list[Any] = []
    for line in stdout.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            out.append(json.loads(line))
        except json.JSONDecodeError as exc:
            raise GhError(f"gh {' '.join(args)} emitted non-JSON line: {exc}") from exc
    return out
