# Dependency package resolved as `foo/default.nix`.
# It pulls a catalog ref from a helper it imports with a path relative to its
# own directory (`./helper.nix` -> `foo/helper.nix`). Following that import
# requires resolving the path against `foo/`, not the package-set root.
{ catalogs }: (import ./helper.nix { inherit catalogs; }).result
