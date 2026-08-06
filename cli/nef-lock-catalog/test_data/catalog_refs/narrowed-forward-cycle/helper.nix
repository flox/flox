# Closes the cycle by passing its own narrowed namespace back as `catalogs`.
# Nix never forces `back`, so this evaluates fine; the scanner follows the
# import anyway and has to terminate on its own.
{ myorg }:
{
  fromHelper = myorg.pkg;
  back = (import ./entry.nix { catalogs = myorg; }).viaHelper;
}
