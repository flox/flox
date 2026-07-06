# A nested package that is itself the scan target (`foo/bar.nix`).
# It references a catalog input directly and depends on `top`, a package at the
# package-set root. Even though this file lives in `foo/`, its dependency
# arguments resolve against the root scope, so `top` must be reachable.
{ catalogs, top }: catalogs.myorg.bar-own
