{ lib }:
# A minimal stand-in for nixpkgs: `lib.makeExtensible` gives it the same
# `.extend` support real nixpkgs has (confirmed via `nix eval --impure
# --expr '(import <nixpkgs> {}) ? extend'`), without importing all of
# nixpkgs into these tests.
lib.makeExtensible (final: {
  greeting = "hello";

  # Stands in for a nested package set nixpkgs already has (e.g.
  # `python3Packages`), so the nested-override test merges into an
  # existing scope rather than exercising `newScope` creation, which
  # this minimal stand-in doesn't provide.
  python3Packages = lib.makeExtensible (finalPy: {
    foo = "unoverridden foo";
  });
})
