{
  nixpkgs-url ? "nixpkgs",
  nixpkgs-flake ? builtins.getFlake nixpkgs-url,
}:
let
  nixpkgs = import nixpkgs-flake { };
  libOverlay = (import ../lib).overlay;
  lib = nixpkgs.lib.extend libOverlay;

in
{

  collectionTests = import ./collectionTests.nix { inherit lib; };
  extensionTests = import ./extensionTests.nix { inherit lib; };
  reflectTests = import ./reflectTests { inherit lib; };
}
