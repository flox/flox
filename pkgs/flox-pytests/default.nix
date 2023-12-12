{
  writeShellScriptBin,
  python3,
}: let
  pytestWith = python3.withPackages (ps:
    with ps; [
      click
      pytest
    ]);
in
  writeShellScriptBin "flox-pytests" ''
    exec -a "$0" "${pytestWith}/bin/python" "${./.}/run.py" "$@"
  ''
