# Target package that depends on the `foo` sibling.
# `foo` resolves as `foo/default.nix` (a package directory, not a flat
# `foo.nix`), which is the branch of `load_dep` that mishandles the import
# directory.
{ catalogs, foo }: catalogs.myorg.direct
