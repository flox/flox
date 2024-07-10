{
  inputs.nixpkgs.url = "github:nixos/nixpkgs?rev=ab5fd150146dcfe41fda501134e6503932cc8dfd";
  outputs = {
    nixpkgs,
    self,
  }: let
    eachDefaultSystemMap = let
      defaultSystems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
    in
      fn: let
        proc = system: {
          name = system;
          value = fn system;
        };
      in
        builtins.listToAttrs (map proc defaultSystems);
  in {
    packages = eachDefaultSystemMap (
      system: let
        pkgs = import nixpkgs {inherit system;};
      in {
        default = self.packages.${system}.hello;

        # a simple derivation without any special attributes
        hello = pkgs.runCommand "hello" {} ''
          echo "Hello, world!" > $out
        '';

        licenseString =
          pkgs.runCommand "licenseString" {
            meta.license = "unlicense";
          } ''
            unlicense >> $out
          '';

        licenseAttrs =
          pkgs.runCommand "licenseAttrs" {
            meta.license = pkgs.lib.licenses.unlicense;
          } ''
            unlicense >> $out
          '';

        licenseListOfAttrs =
          pkgs.runCommand "licenseListOfAttrs" {
            meta.license = [pkgs.lib.licenses.unlicense pkgs.lib.licenses.mit];
          } ''
            unlicense >> $out
          '';

        multipleOutputs =
          pkgs.runCommand "multipleOutputs" {
            outputs = ["out" "man" "dev"];
            meta.outputsToInstall = ["out" "man"];
          } ''
            touch $out
            touch $man
            touch $dev
          '';
      }
    );
  };
}
