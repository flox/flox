# Helper imported by foo/default.nix, living beside it in foo/.
# Its catalog ref must be reached when the import is resolved relative to foo/.
{ catalogs }:
{
  result = catalogs.myorg.helper-ref;
}
