# `stdenv` is supplied by nixpkgs, not by a file in the package set, so it
# resolves to nothing on disk. That is not a failure: the argument is satisfied
# outside the set.
{ catalogs, stdenv }:
catalogs.myorg.toolkit.readVersion
