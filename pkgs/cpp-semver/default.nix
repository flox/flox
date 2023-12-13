{ lib
, stdenvNoCC
, fetchFromGitHub
}:

stdenvNoCC.mkDerivation rec {
  pname = "cpp-semver";
  version = "unstabble-2021-12-10";

  src = fetchFromGitHub {
    owner = "easz";
    repo = "cpp-semver";
    rev = "7b9141d99044e4d363eb3b0a81cfb1546a33f9dd";
    sha256 = "sha256-v0Ou3z9loiwmeYTOqSGrQNbM05OlaWOgH+F9q+/FhkI=";
  };

  # Header-only library.
  dontBuild = true;

  installPhase = ''
    mkdir "$out"
    cp -r include "$out"
  '';

  meta = with lib; {
    description = "semver in c++";
    homepage = "https://github.com/easz/cpp-semver";
    maintainers = with maintainers; [ tomberek ];
    license = licenses.mit;
  };
}

