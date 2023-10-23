{rustPlatform}: let
  src = builtins.fetchGit {
    url = "https://github.com/vlinkz/nix-editor.git";
    rev = "ee45ac30a6e8bf1cbf40a5c1518eedd39a51fec1";
    ref = "main";
  };
in
  rustPlatform.buildRustPackage {
    pname = "nix-editor";
    version = "0.3.0";
    inherit src;
    cargoLock = {
      lockFile = src + "/Cargo.lock";
    };
  }
