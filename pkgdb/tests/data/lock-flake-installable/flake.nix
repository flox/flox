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
          touch $out
        '';

        licenseString =
          pkgs.runCommand "licenseString" {
            meta.license = "Unlicense";
          } ''
            touch $out
          '';

        licenseAttrs =
          pkgs.runCommand "licenseAttrs" {
            meta.license = pkgs.lib.licenses.unlicense;
          } ''
            touch $out
          '';

        licenseListOfAttrs =
          pkgs.runCommand "licenseListOfAttrs" {
            meta.license = [pkgs.lib.licenses.unlicense pkgs.lib.licenses.mit];
          } ''
            touch $out
          '';

        licenseNoLicense = pkgs.runCommand "licenseNoLicense" {} ''
          touch $out
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

        withDescription =
          pkgs.runCommand "withDescrioption" {
            meta.description = "A package with a description";
          } ''
            touch $out
          '';

        names =
          pkgs.runCommand "explicit-name" {
            pname = "hello";
          } ''
            touch $out
          '';

        versioned =
          pkgs.runCommand "explicit-name" {
            version = "1.0";
          } ''
            touch $out
          '';

        # with broken = true, the package does not even evaluate
        broken =
          pkgs.runCommand "broken" {
            meta.broken = false;
          } ''
            exit 1
          '';

        # with unfree = true, the package does not even evaluate
        unfree =
          pkgs.runCommand "unfree" {
            meta.unfree = false;
          } ''
            touch $out
          '';
      }
    );
  };
}
