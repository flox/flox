{
  description = "flox environment";
  inputs.flox-floxpkgs.url = "github:flox/floxpkgs";

  outputs = args @ {flox-floxpkgs, ...}:
    flox-floxpkgs.project args ({self, ...}: {
      devShells.default = {
        mkShell,
        ripgrep,
      }:
        mkShell {
          packages = [ripgrep];
          shellHook = ''
            echo "developing package"
          '';
        };
    });
}
