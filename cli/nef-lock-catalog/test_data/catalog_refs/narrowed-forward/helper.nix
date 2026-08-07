# Scanned with `myorg` as its root, so `pkg` is one component here but already
# package-deep once rewritten under `catalogs.myorg`.
{ myorg }:
{
  deep = myorg.toolkit.readVersion;
  shallow = myorg.pkg;
}
