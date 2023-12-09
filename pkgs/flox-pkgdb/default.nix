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
  llvm, # for `llvm-symbolizer'
  gdb ? throw "`gdb' is required for debugging with `g++'",
  lldb ? throw "`lldb' is required for debugging with `clang++'",
  valgrind ? throw "`valgrind' is required for memory sanitization on Linux",
  ci ? false,
}: let
  envs = {
    nix_INCDIR = nix.dev.outPath + "/include";
    boost_CFLAGS = "-isystem " + boost.dev.outPath + "/include";
    toml_CFLAGS = "-isystem " + toml11.outPath + "/include";
    yaml_PREFIX = yaml-cpp.outPath;
    libExt = stdenv.hostPlatform.extensions.sharedLibrary;
    SEMVER_PATH = semver.outPath + "/bin/semver";
  };
in
  stdenv.mkDerivation ({
      pname = "flox-pkgdb";
      version = builtins.replaceStrings ["\n"] [""] (builtins.readFile ./../../pkgdb/version);

      src = builtins.path {
        path = ./../..;
        filter = name: type: let
          bname = baseNameOf name;
          ignores = [
            "default.nix"
            "pkg-fun.nix"
            "flake.nix"
            "flake.lock"
            ".ccls"
            ".ccls-cache"
            "compile_commands.json"
            ".git"
            ".gitignore"
            "out"
            "bin"
            "pkgs"
            "shells"
            "bear.d"
            ".direnv"
            ".envrc"
            ".clang-tidy"
            ".clang-format"
            ".envrc"
            ".github"
            "LICENSE"
            "tests"
            "autom4te.cache"
            "assets"
            "cli"
            "env-builder"
            "img"
            "resolver"
            "target"
            "Justfile"
            "Procfile"
          ];
          ext = let
            m = builtins.match ".*\\.([^.]+)" name;
          in
            if m == null
            then ""
            else builtins.head m;
          ignoredExts = ["o" "so" "dylib" "log"];
          notResult = (builtins.match "result(-*)?" bname) == null;
          notTmp = (builtins.match ".*~" bname) == null;
          notIgnored =
            (! (builtins.elem bname ignores))
            && (! (builtins.elem ext ignoredExts));
        in
          notIgnored && notResult && notTmp;
      };

      propagatedBuildInputs = [semver nix];

      nativeBuildInputs = [pkg-config];

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

      preBuild = "cd pkgdb;";

      # Checks require internet
      doCheck = false;
      doInstallCheck = false;

      outputs = ["out" "dev" "test"];

      postInstall = ''
        mkdir -p "$test/bin" "$test/lib"

        cp ${../../pkgdb/tests/is_sqlite3.cc} ./tests/is_sqlite3.cc
        cp ${../../pkgdb/tests/search-params.cc} ./tests/search-params.cc
        make tests/is_sqlite3
        make tests/search-params

        for i in tests/*; do
          if (! [[ -d "$i" ]]) && [[ -x "$i" ]]; then
            cp "$i" "$test/bin/"
          fi
        done

        for i in "$out/lib/"*; do
          ln -s "$i" "$test/lib/"
        done
      '';

      meta.mainProgram = "pkgdb";

      passthru = {
        inherit
          envs
          nix
          semver
          ;

        ciPackages = [
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
