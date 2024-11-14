{
  bashInteractive,
  tcsh,
  zsh,
  fish,
  symlinkJoin,
  makeBinaryWrapper,
}:
symlinkJoin {
  name = "flox-test-shells";

  paths = [
    bashInteractive
    zsh
    tcsh
    fish
  ];

  nativeBuildInputs = [ makeBinaryWrapper ];

  postBuild = ''
    wrapProgram $out/bin/bash --add-flags "--norc"
    wrapProgram $out/bin/zsh --add-flags "-o NO_GLOBAL_RCS -o NO_RCS"
    wrapProgram $out/bin/fish --add-flags "--no-config"
    wrapProgram $out/bin/tcsh --add-flags "-m"
  '';
}

