"""Make the package importable from tests when running pytest from any cwd."""
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]  # repo root
sys.path.insert(0, str(ROOT))

# Allow `from scripts.pr_analysis.lib.foo import ...` style imports
# by aliasing the hyphenated dir under an importable name.
import importlib.util
import types

ALIAS_TARGET = ROOT / "scripts" / "pr-analysis"
pkg = types.ModuleType("scripts")
pkg.__path__ = [str(ROOT / "scripts")]
sys.modules.setdefault("scripts", pkg)

inner = types.ModuleType("scripts.pr_analysis")
inner.__path__ = [str(ALIAS_TARGET)]
sys.modules.setdefault("scripts.pr_analysis", inner)
