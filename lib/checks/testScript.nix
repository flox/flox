# mkFakeDerivation transforms data in catalog format into a fake derivation with a store path that
# can be substituted
{lib}: {
  pkgs,
  name,
  runtimeInputs,
  checkOn ? [],
}: text: let
  script = pkgs.writeShellApplication {
    inherit name text runtimeInputs;
  };
in
  script // {meta = script.meta // {inherit checkOn;};}
