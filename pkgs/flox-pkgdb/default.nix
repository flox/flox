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
  substituteAll,
  symlinkJoin,
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
    PROFILE_D_SCRIPTS_DIR = let
      path = builtins.path {
        name = "etc-profile.d";
        path = ../../pkgdb/src/buildenv/assets/etc/profile.d;
      };

      dependencies = {
        realpath = coreutils + "/bin/realpath";
      };

      scripts = lib.mapAttrs (name: type:
        substituteAll ({
            src = path + "/${name}";
            dir = "etc/profile.d";
            isExecutable = true;
          }
          // dependencies)) (builtins.readDir path);

      joined = symlinkJoin {
        name = "profile-d-scripts";
        paths = lib.attrValues scripts;
      };
    in
      joined;
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
        contents = builtins.readFile ./../../pkgdb/.version;
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
        echo "PROFILE_D_SCRIPTS_DIR: $PROFILE_D_SCRIPTS_DIR" >&2;
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
            bear
            valgrind
          ]);

        devEnvs =
          envs
          // {
            # For running `pkgdb' interactively with inputs from the test suite.
            NIXPKGS_TEST_REV = "ab5fd150146dcfe41fda501134e6503932cc8dfd";
            NIXPKGS_TEST_REF =
              "github:NixOS/nixpkgs/"
              + "ab5fd150146dcfe41fda501134e6503932cc8dfd";
          };

        devShellHook = ''
          #  # Find the project root and add the `bin' directory to `PATH'.
          if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
            REPO_ROOT="$( git rev-parse --show-toplevel; )";
            PATH="$REPO_ROOT/pkgdb/bin:$PATH";
            PKGDB_BIN="$REPO_ROOT/pkgdb/bin/pkgdb";
            PKGDB_SEARCH_PARAMS_BIN="$REPO_ROOT/pkgdb/tests/search-params";
            PKGDB_IS_SQLITE3_BIN="$REPO_ROOT/pkgdb/tests/is_sqlite3";
          fi
        '';
      };
    }
    // envs)
