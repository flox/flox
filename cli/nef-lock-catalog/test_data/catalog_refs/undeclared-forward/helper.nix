# Helper receiving the namespace forwarded by entry.nix.
{ catalogs }:
{
  result = catalogs.myorg.toolkit.readVersion;
}
