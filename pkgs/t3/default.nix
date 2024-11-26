{
  help2man,
  lib,
  stdenv,
  fetchFromGitHub,
  t3-src,
}:

let
  pname = "t3";
  version = "1.0.3";
  src = t3-src;

in
stdenv.mkDerivation rec {
  inherit pname version src;

  installFlags = [
    "PREFIX=$(out)"
    "VERSION=${version}"
  ];
  nativeBuildInputs = [ help2man ];

  meta = with lib; {
    homepage = "https://github.com/flox/t3";
    description = "Next generation tee with colorized output streams and precise time stamping";
    maintainers = [ maintainers.limeytexan ];
    license = licenses.mit;
  };
}
