"""Map a repo-relative file path to a normalized area label.

Areas are coarsened so that comments on related files group together. The
three "hot areas" (commands, models/environment, providers) keep enough
granularity to drive per-area CLAUDE.md synthesis; everything else falls
into broader buckets.
"""
from __future__ import annotations

# Order matters: longest, most-specific prefixes first.
_PREFIX_MAP: list[tuple[str, str]] = [
    ("cli/flox/src/commands/services/", "commands/services"),
    ("cli/flox/src/commands/init/", "commands/init"),
    ("cli/flox/src/commands/", "commands"),
    ("cli/flox-rust-sdk/src/models/environment/", "models/environment"),
    ("cli/flox-rust-sdk/src/models/", "models/other"),
    ("cli/flox-rust-sdk/src/providers/", "providers"),
    ("cli/flox-activations/src/", "activations"),
    ("cli/flox-core/src/", "core"),
    ("cli/flox-test-utils/", "test-utils"),
    ("cli/flox-manifest/", "manifest"),
    ("cli/flox/src/utils/", "cli/utils"),
    ("cli/flox/src/", "cli/other"),
    ("cli/", "cli/other"),
    ("nix-plugins/", "nix-plugins"),
    ("assets/activation-scripts/", "activation-scripts"),
]

HOT_AREAS = ("commands", "models/environment", "providers")


def area_for_path(path: str) -> str:
    for prefix, area in _PREFIX_MAP:
        if path.startswith(prefix):
            return area
    return "other"


def is_rust(path: str) -> bool:
    return path.endswith(".rs")
