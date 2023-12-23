{
  stdenv,
  lib,
  argparse,
  doxygen,
  bear,
  boost,
  ccls,
  clang-tools_16,
  include-what-you-use,
  lcov,
  nix,
  nlohmann_json,
  pkg-config,
  remake,
  semver,
  sqlite,
  sqlite3pp,
  toml11,
  yaml-cpp,
  # For testing
  bash,
  yj,
  jq,
  gnugrep,
  bats,
  git,
  coreutils,
  llvm, # for `llvm-symbolizer'
  gdb ? throw "`gdb' is required for debugging with `g++'",
  lldb ? throw "`lldb' is required for debugging with `clang++'",
  valgrind ? throw "`valgrind' is required for memory sanitization on Linux",
  ci ? false,
}: let
  batsWith = bats.withLibraries (p: [
    p.bats-assert
    p.bats-file
    p.bats-support
  ]);
  envs = {
    nix_INCDIR = nix.dev.outPath + "/include";
    boost_CFLAGS = "-isystem " + boost.dev.outPath + "/include";
    toml_CFLAGS = "-isystem " + toml11.outPath + "/include";
    yaml_PREFIX = yaml-cpp.outPath;
    libExt = stdenv.hostPlatform.extensions.sharedLibrary;
    SEMVER_PATH = semver.outPath + "/bin/semver";
    # Used by `buildenv' to provide activation hook extensions.
    PROFILE_D_SCRIPT_DIR = builtins.path {
      name = "etc-profile.d";
      path = ../../pkgdb/src/buildenv/assets;
    };
    # Used by `buildenv' to set shell prompts on activation.
    SET_PROMPT_BASH_SH = builtins.path {
      name = "set-prompt-bash.sh";
      path = ../../pkgdb/src/buildenv/set-prompt-bash.sh;
    };
  };
in
  stdenv.mkDerivation ({
      pname = "flox-pkgdb";
      version = let
        contents = builtins.readFile ../../pkgdb/.version;
      in
        builtins.replaceStrings ["\n"] [""] contents;

      src = builtins.path {
        path = ./../../pkgdb;
        filter = name: type: let
          bname = baseNameOf name;
          ignores = [
            ".ccls"
            ".ccls-cache"
            "compile_commands.json"
            ".git"
            ".gitignore"
            "bin"
            "build"
            "pkgs"
            "bear.d"
            ".direnv"
            ".clang-tidy"
            ".clang-format"
            ".envrc"
            "LICENSE"
          ];
          ext = let
            m = builtins.match ".*\\.([^.]+)" name;
          in
            if m == null
            then ""
            else builtins.head m;
          ignoredExts = ["o" "so" "dylib" "log"];
          notResult = (builtins.match "result(-*)?" bname) == null;
          notIgnored =
            (! (builtins.elem bname ignores))
            && (! (builtins.elem ext ignoredExts));
        in
          notIgnored && notResult;
      };

      propagatedBuildInputs = [semver];

      nativeBuildInputs = [pkg-config coreutils gnugrep];

      buildInputs = [
        sqlite.dev
        nlohmann_json
        argparse
        sqlite3pp
        toml11
        yaml-cpp
        boost
        nix
      ];

      configurePhase = ''
        runHook preConfigure;
        export PREFIX="$out";
        echo "PROFILE_D_SCRIPT_DIR: $PROFILE_D_SCRIPT_DIR" >&2;
        echo "SET_PROMPT_BASH_SH: $SET_PROMPT_BASH_SH" >&2;
        if [[ "''${enableParallelBuilding:-1}" = 1 ]]; then
          makeFlagsArray+=( "-j''${NIX_BUILD_CORES:?}" );
        fi
        runHook postConfigure;
      '';

      # Checks require internet
      doCheck = false;
      doInstallCheck = false;

      meta.mainProgram = "pkgdb";

      passthru = {
        inherit
          envs
          nix
          semver
          ;

        ciPackages = [
          # For tests
          batsWith
          yj
          jq
          bash
          git
          sqlite
          # For docs
          doxygen
        ];

        devPackages =
          [
            # For profiling
            lcov
            remake
            # For IDEs
            ccls
            bear
            # For lints/fmt
            clang-tools_16
            include-what-you-use
            llvm # for `llvm-symbolizer'
            # For debugging
            (
              if stdenv.cc.isGNU or false
              then gdb
              else lldb
            )
          ]
          ++ (lib.optionals stdenv.isLinux [
            valgrind
          ]);

        devEnvs =
          envs
          // {
            # For running `pkgdb' interactively with inputs from the test suite.
            NIXPKGS_TEST_REV = "e8039594435c68eb4f780f3e9bf3972a7399c4b1";
            NIXPKGS_TEST_REF = "github:NixOS/nixpkgs/$NIXPKGS_TEST_REV";
          };

        devShellHook = ''
          #  # Find the project root and add the `bin' directory to `PATH'.
          if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
            PATH="$( git rev-parse --show-toplevel; )/pkgdb/bin":$PATH;
          fi
        '';
      };
    }
    // envs)
