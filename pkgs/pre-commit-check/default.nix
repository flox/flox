{
  lib,
  pre-commit-hooks,
  symlinkJoin,
  system,
  rustfmt,
  fenix,
  rust-toolchain,
  clang-tools_16,
  makeWrapper,
}:
pre-commit-hooks.lib.${system}.run {
  src = builtins.path {path = ./.;};
  default_stages = [
    "manual"
    "push"
  ];
  hooks = {
    nixfmt-rfc-style = {
      enable = true;
    };
    clang-format = {
      enable = true;
      types_or = lib.mkForce [
        "c"
        "c++"
      ];
    };
    rustfmt = let
      wrapper = symlinkJoin {
        name = "rustfmt-wrapped";
        paths = [rustfmt];
        nativeBuildInputs = [makeWrapper];
        postBuild = let
          # Use nightly rustfmt
          PATH = lib.makeBinPath [
            fenix.stable.cargo
            rustfmt
          ];
        in ''
          wrapProgram $out/bin/cargo-fmt --prefix PATH : ${PATH};
        '';
      };
    in {
      enable = true;
      entry = lib.mkForce "${wrapper}/bin/cargo-fmt fmt --all --manifest-path 'cli/Cargo.toml' -- --color always";
    };
    clippy.enable = true;
    clippy.settings.denyWarnings = true;
    commitizen.enable = true;
    shfmt.enable = false;
    # shellcheck.enable = true; # disabled until we have time to fix all the warnings
  };
  settings = {
    rust.cargoManifestPath = "cli/Cargo.toml";
  };
  tools = {
    # use fenix provided clippy
    clippy = rust-toolchain.clippy;
    cargo = rust-toolchain.cargo;
    clang-tools = clang-tools_16;
  };
}
