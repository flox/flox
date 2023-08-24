{
  description = "flox environment";
  inputs.flox-floxpkgs.url = "github:flox/floxpkgs";

  outputs = args @ {flox-floxpkgs, ...}:
    flox-floxpkgs.project args ({self, ...}: {});
}
