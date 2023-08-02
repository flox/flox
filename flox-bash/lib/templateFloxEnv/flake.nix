{
  description = "flox environment";
  inputs.flox-floxpkgs.url = "github:flox/floxpkgs?ref=pure-store-path";

  outputs = args @ {flox-floxpkgs, ...}:
    flox-floxpkgs.project args ({self, ...}: {});
}
