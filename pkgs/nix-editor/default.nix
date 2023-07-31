{
  rustPlatform,
  fetchFromGitHub,
}:
rustPlatform.buildRustPackage rec {
  pname = "nix-editor";
  version = "0.3.0-beta.1";
  src = fetchFromGitHub {
    owner = "vlinkz";
    repo = pname;
    rev = version;
    sha256 = "sha256-yjiYbBJNzsG6kAS6aRbqmMyiwEimT1/wzg4MUWwzNco=";
  };

  cargoLock = {
    lockFile = src + "/Cargo.lock";
  };
}
