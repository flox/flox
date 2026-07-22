# Pattern: transitive closure under a non-default root (`inputs`). `main`
# pulls in the `dep-pkg` argument; the sibling's `inputs.*` refs must join the
# closure.
{ inputs, dep-pkg }: inputs.nixpkgs.lib
