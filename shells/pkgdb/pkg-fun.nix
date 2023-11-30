# ============================================================================ #
#
#
#
# ---------------------------------------------------------------------------- #

{ stdenv
, mkShell
, bats
, jq
, yj
, doxygen
, lcov
, remake
, ccls
, bear
, clang-tools_16
, llvm  # for `llvm-symbolizer'
, include-what-you-use
, gdb        ? throw "`gdb' is required for debugging with `g++'"
, lldb       ? throw "`lldb' is required for debugging with `clang++'"
, valgrind   ? throw "`valgrind' is required for memory sanitization on Linux"
, flox-pkgdb

# Limit packages to only those required for building and testing `pkgdb' in CI.
# This includes Doxygen for generating docs, but excludes linting tools like
# `clang-tidy' and `include-what-you-use' which are only provided in the _full_
# interactive shell ( `ci = false' produces the _full_ interactive shell ).
, ci ? false
}: let

# ---------------------------------------------------------------------------- #

  # For use in GitHub Actions and local development.
  ciPkgs = let
    batsWith = bats.withLibraries ( libs: [
      libs.bats-assert
      libs.bats-file
      libs.bats-support
    ] );
  in [
    # For tests
    batsWith
    jq
    yj
    # For doc
    doxygen
  ];

  # For use in local development.
  interactivePkgs = [
    # For profiling
    lcov
    remake
    # For IDEs
    ccls
    bear
    # For lints/fmt
    clang-tools_16
    include-what-you-use
    llvm  # for `llvm-symbolizer'
    # For debugging
    ( if stdenv.cc.isGNU or false then gdb else lldb )
  ] ++ ( if stdenv.isLinux or false then [valgrind] else [] );

in mkShell {
  name = if ci then "ci" else "pkgdb";
  packages = ciPkgs ++ ( if ci then [] else interactivePkgs );
  inputsFrom = [flox-pkgdb];
  inherit (flox-pkgdb)
    nix_INCDIR boost_CFLAGS toml_CFLAGS yaml_PREFIX libExt SEMVER_PATH
  ;
  shellHook = ''
    shopt -s autocd;

    alias gs='git status';
    alias ga='git add';
    alias gc='git commit -am';
    alias gl='git pull';
    alias gp='git push';

    # For running `pkgdb' interactively with inputs from the test suite.
    NIXPKGS_TEST_REV="e8039594435c68eb4f780f3e9bf3972a7399c4b1";
    NIXPKGS_TEST_REF="github:NixOS/nixpkgs/$NIXPKGS_TEST_REV";

    # Find the project root and add the `bin' directory to `PATH'.
    if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
      PATH="$PATH:$( git rev-parse --show-toplevel; )/pkgdb/bin";
    fi

    if [ -z "''${NO_WELCOME:-}" ]; then
      {
        echo "First enter the \`pkgdb' subdirectory with \`cd pkgdb;'";
        echo "";
        echo "Build with \`make' ( or \`make -j' to go fast )";
        echo "";
        echo "Run with \`./bin/pkgdb --help'";
        echo "";
        echo "Test with \`make check'";
        echo "";
        echo "Read docs with: \`make docs && firefox ./docs/index.hml'";
        echo "";
        echo "See more tips in \`CONTRIBUTING.md'";
      } >&2;
    fi
  '';
}


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
