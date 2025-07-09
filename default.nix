{
  system ? builtins.currentSystem,
}:
let
  src = ./.;
  flake-compat = builtins.fetchTarball {
    url = "https://github.com/edolstra/flake-compat/archive/baa7aa7bd0a570b3b9edd0b8da859fee3ffaa4d4.tar.gz";
    sha256 = "sha256:002mjvf08z3vm1djzgb2b95d89kn526fas0lagjwr38jmmf7ign6";
  };
  flake = import flake-compat { inherit src; };
in
flake.defaultNix.packages.${system}.flox
