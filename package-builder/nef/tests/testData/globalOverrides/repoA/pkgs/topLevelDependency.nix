# Lives in `pkgs/`, same as any ordinary published package. It is
# treated as a global override only because the test's lock entry
# names this attr_path in `global_overrides` -- a real override is
# published into the reserved `global` catalog, which is what puts
# its entries in a lock's `global_overrides` group in the first
# place; NEF's overlay only ever reads that group.
{ }: "overridden by repoA"
