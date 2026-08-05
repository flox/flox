# Pattern: a dependency-argument cycle. `pkg-a` depends on `pkg-b` and
# `pkg-b` depends back on `pkg-a`; the closure must terminate and union both
# packages' refs.
{ catalogs, pkg-b }: catalogs.myorg.toolkit.readVersion
