{
  help2man,
  lib,
  stdenv,
  fetchFromGitHub,
}:

stdenv.mkDerivation rec {
  pname = "t3";
  version = "1.0.1";

  src = fetchFromGitHub {
    owner = "flox";
    repo = pname;
    rev = "v${version}";
    hash = "sha256-Xmju4H6tpORjRUw9TJ2OEBJZBJ1vsZ3x/GAt+Yqhkmc=";
  };

  installFlags = [ "PREFIX=$(out)" ];
  nativeBuildInputs = [ help2man ];
  doCheck = true;

  meta = with lib; {
    homepage = "https://github.com/flox/t3";
    description = "Next generation tee with colorized output streams and precise time stamping";
    maintainers = [ maintainers.limeytexan ];
    license = licenses.mit;
  };
}
