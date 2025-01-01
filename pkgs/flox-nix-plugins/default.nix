{
  stdenv,
  lib,
  doxygen,
  boost,
  clang-tools_16,
  lcov,
  nix,
  pkg-config,
  meson,
  ninja,
  # For testing
  gdb ? throw "`gdb' is required for debugging with `g++'",
  lldb ? throw "`lldb' is required for debugging with `clang++'",
}:
let
  envs = { };
in
stdenv.mkDerivation (
  {
    pname = "flox-nix-plugins";
    version = lib.fileContents ./../../VERSION;
    src = builtins.path {
      path = ./../../nix-plugins;
    };

    nativeBuildInputs = [
      meson
      ninja
      pkg-config
    ];

    buildInputs = [
      boost
      nix.dev
    ];

    # Checks require internet
    doCheck = false;
    doInstallCheck = false;

    passthru = {
      inherit envs nix;

      ciPackages = [
        # For tests

        # For docs
        doxygen
      ];

      devPackages = [
        # For profiling
        lcov
        # For IDEs
        # ccls
        # bear
        # For lints/fmt
        clang-tools_16
        # include-what-you-use
        # llvm # for `llvm-symbolizer'
        # For debugging
        (if stdenv.cc.isGNU or false then gdb else lldb)
      ];
      # Uncomment if you need to do memory profiling/sanitization.
      #++ (lib.optionals stdenv.isLinux [
      #  valgrind
      #  massif-visualizer
      #]);
    };
  }
  // envs
)
