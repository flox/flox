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
  sentry-native,
  sqlite,
  sqlite3pp,
  toml11,
  yaml-cpp,
  cpp-semver,
  bash,
  # For testing
  yj,
  jq,
  gnugrep,
  bats,
  git,
  coreutils,
  parallel,
  llvm, # for `llvm-symbolizer'
  gdb ? throw "`gdb' is required for debugging with `g++'",
  lldb ? throw "`lldb' is required for debugging with `clang++'",
  valgrind ? throw "`valgrind' is required for memory sanitization on Linux",
  massif-visualizer ? throw "`massif-visualizer' is required for memory profiling on Linux",
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
    sentry_PREFIX = sentry-native.outPath;
    yaml_PREFIX = yaml-cpp.outPath;
    semver_PREFIX = cpp-semver.outPath;
    libExt = stdenv.hostPlatform.extensions.sharedLibrary;
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
      name = "set-prompt.bash.sh";
      path = ../../pkgdb/src/buildenv/assets/set-prompt.bash.sh;
    };

    # Used by `buildenv' to set shell prompts on activation.
    SET_PROMPT_ZSH_SH = builtins.path {
      name = "set-prompt.zsh.sh";
      path = ../../pkgdb/src/buildenv/assets/set-prompt.zsh.sh;
    };

    # Used by `buildenv --container' to create a container builder script.
    CONTAINER_BUILDER_PATH = builtins.path {
      name = "mkContainer.nix";
      path = ../../pkgdb/src/buildenv/assets/mkContainer.nix;
    };

    # The Bash executable to use for `hook.on-activate`
    FLOX_BASH_BIN = "${bash}/bin/bash";

    # Used by `buildenv --container' to access `dockerTools` at a known version
    # When utilities from nixpkgs are used by flox at runtime,
    # they should be
    # a) bundled at buildtime if possible (binaries/packages)
    # b) use this version of nixpkgs i.e. (nix library utils such as `dockerTools`)
    COMMON_NIXPKGS_URL = let
      lockfile = builtins.fromJSON (builtins.readFile ./../../flake.lock);
      root = lockfile.nodes.${lockfile.root};
      nixpkgs = lockfile.nodes.${root.inputs.nixpkgs}.locked;
    in
      # todo: use `builtins.flakerefToString` once flox ships with nix 2.18+
      "github:NixOS/nixpkgs/${nixpkgs.rev}";
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

      propagatedBuildInputs = [cpp-semver];

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
        cpp-semver
        bash
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
          cpp-semver
          ;

        ciPackages = [
          # For tests
          batsWith
          yj
          jq
          git
          sqlite
          parallel
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
            massif-visualizer
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
            LD_FLOXLIB="$REPO_ROOT/pkgdb/lib/ld-floxlib.so";
            PKGDB_SEARCH_PARAMS_BIN="$REPO_ROOT/pkgdb/tests/search-params";
            PKGDB_IS_SQLITE3_BIN="$REPO_ROOT/pkgdb/tests/is_sqlite3";
          fi
        '';
      };
    }
    // envs)
