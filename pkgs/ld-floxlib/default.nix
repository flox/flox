{
  self,
  lib,
  stdenv,
}:
stdenv.mkDerivation {
  pname = "ld-floxlib";
  version = "1.0.0";
  src = builtins.path {
    name = "ld-floxlib-src";
    path = "${./../../ld-floxlib}";
  };
  makeFlags = [ "PREFIX=$(out)" ];
  doCheck = true;
}
