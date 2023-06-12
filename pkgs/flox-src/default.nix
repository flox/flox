{self}:
builtins.path {
  name = "flox-src";
  path = self;
  filter = path: type:
    ! builtins.elem path [
      (self.outPath + "/flake.nix")
      (self.outPath + "/flake.lock")
      (self.outPath + "/pkgs")
      (self.outPath + "/checks")
      (self.outPath + "/tests")
      (self.outPath + "/shells")
      (self.outPath + "/.github")
    ];
}
