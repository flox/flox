# `makeScopeValue` coincides with a name the nested `setMakeScope`
# attrset already has upstream -- the stub mechanism warns and falls
# back to the upstream value instead of this repository's own
# `pkgs/setMakeScope/makeScopeValue.nix`.
{ makeScopeValue, ... }: makeScopeValue
