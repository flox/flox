# The real package behind the alias.
# Lives as a member of the `python3Packages` set at `<set>/<member>/default.nix`,
# the on-disk layout NEF package sets use (a member directory, no set-root
# `default.nix`). It pulls a catalog build input, which the alias that points at
# it must transitively surface.
{ catalogs }: catalogs.myorg.toolkit.readVersion
