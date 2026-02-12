{
  nixpkgs-url ? "nixpkgs",
  nixpkgs-flake ? builtins.getFlake nixpkgs-url,
  test-fixtures ? (
    builtins.warn ''
      No 'test-fixtures' provided, will skip 'instantiateTests'
      'test-fixtures' is expected to point to a directory resembling
       the files in `./instantiateTests/testData`, **including** `nix-build.lock`.

      Build fixtures by running

        just build build-nef-test-fixtures

      and pass the result as an argument:

        --argstr test-fixtures <path to locked fixtures>
    '' null
  ),
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
// (lib.optionalAttrs (test-fixtures != null) {
  instantiateTests = import ./instantiateTests {
    inherit lib nixpkgs;
    fixtures = test-fixtures;
  };
})
