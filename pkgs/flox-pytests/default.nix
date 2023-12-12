{
  runCommand,
  runtimeShell,
  python3,
}: let
  pyenv = python3.withPackages (ps:
    with ps; [
      click
      pytest
    ]);
in
  runCommand "flox-pytests"
  {
    passthru = {inherit python3 pyenv;};
    preferLocalBuild = true;
    text = ''
      #!${runtimeShell}
      exec -a "$0" "${pyenv}/bin/python" "${./.}/run.py" "$@"
    '';
  }
  ''
    mkdir -p "$out/bin"
    echo -n "$text" > "$out/bin/flox-pytests"
    chmod +x "$out/bin/flox-pytests"
  ''
