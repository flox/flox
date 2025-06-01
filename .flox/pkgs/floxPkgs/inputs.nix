{ pkgs, fetchgit }:
{
  self = ../../../.;
  crane.mkLib =
    pkgs:
    import
      (builtins.fetchTree {
        "narHash" = "sha256-DdWJLA+D5tcmrRSg5Y7tp/qWaD05ATI4Z7h22gd1h7Q=";
        "owner" = "ipetkov";
        "repo" = "crane";
        "rev" = "dfd9a8dfd09db9aad544c4d3b6c47b12562544a5";
        "type" = "github";
      }).outPath
      { inherit pkgs; };

  fenix = builtins.getFlake (
    builtins.flakeRefToString {
      "narHash" = "sha256-sVuLDQ2UIWfXUBbctzrZrXM2X05YjX08K7XHMztt36E=";
      "owner" = "nix-community";
      "repo" = "fenix";
      "rev" = "7d9ba794daf5e8cc7ee728859bc688d8e26d5f06";
      "type" = "github";
    }
  );
}
