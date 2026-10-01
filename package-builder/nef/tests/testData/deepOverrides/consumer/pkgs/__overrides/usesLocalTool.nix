# Proves a deep override cannot see its own repository's `pkgs/` tree:
# `localTool` only exists in this repo's `pkgs/`, never in the deep
# overlay's own scope, so callPackage falls back to this default
# instead of resolving the real package.
{
  localTool ? "not visible to deep overrides",
}:
localTool
