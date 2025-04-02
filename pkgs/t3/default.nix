{
  help2man,
  lib,
  stdenv,
  fetchFromGitHub,
  src,
  version,
}:

stdenv.mkDerivation rec {
  pname = "t3";
  inherit version src;

  installFlags = [
    "PREFIX=$(out)"
    "VERSION=${version}"
  ];
  nativeBuildInputs = [ help2man ];
  doCheck = false;

  meta = with lib; {
    homepage = "https://github.com/flox/t3";
    description = "Next generation tee with colorized output streams and precise time stamping";
    maintainers = [ maintainers.limeytexan ];
    license = licenses.mit;
  };
}
