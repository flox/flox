{
  help2man,
  lib,
  stdenv,
  fetchFromGitHub,
}:

stdenv.mkDerivation rec {
  pname = "t3";
  version = "1.0.0";

  src = fetchFromGitHub {
    owner = "flox";
    repo = pname;
    rev = "v${version}";
    hash = "sha256-70qEPC5V0Vq1g3xEyeungOYUEmP/SwxXnMXiTsVEXSs=";
  };

  installFlags = [ "PREFIX=$(out)" ];
  nativeBuildInputs = [ help2man ];

  meta = with lib; {
    homepage = "https://github.com/flox/t3";
    description = "Next generation tee with colorized output streams and precise time stamping";
    maintainers = [ maintainers.limeytexan ];
    license = licenses.mit;
  };
}
