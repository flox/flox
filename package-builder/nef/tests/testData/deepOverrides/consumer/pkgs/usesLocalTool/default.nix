# Proves a deep override cannot see its own repository's `pkgs/` tree:
# `localTool` only exists in this repo's `pkgs/`, never in the deep
# overlay's own scope, so the calling scope binds `localTool` to a
# throw stub instead of leaving it unbound -- the default below is
# never reached.
{
  localTool ? "not visible to deep overrides",
}:
localTool
