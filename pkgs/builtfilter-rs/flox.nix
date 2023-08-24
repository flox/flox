{
  context,
  system,
  ...
}: {
  packages.nixpkgs-flox.rustfmt = {};
  environmentVariables = {
    RUST_SRC_PATH = context.nixpkgs.legacyPackages.${system}.rustPlatform.rustLibSrc.outPath;
  };
}
