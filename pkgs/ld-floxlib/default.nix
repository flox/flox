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
  postPatch = ''
    substituteInPlace closure.c --replace '@@out@@' "$out"
  '';
  makeFlags = ["PREFIX=$(out)"];
  doCheck = true;
}
