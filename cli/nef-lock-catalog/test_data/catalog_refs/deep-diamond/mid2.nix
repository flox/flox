# Forwards its namespace on to the shared helper under the same local name
# as mid1.nix does.
{ cats }: import ./common.nix { ns = cats; }
