{
  writeShellScript,
  coreutils,
  gnused,
  procps,
  flox-activate-d-scripts,
}:
writeShellScript "activate" ''
  _coreutils="${coreutils}"
  _gnused="${gnused}"
  _procps="${procps}"
  _zdotdir="${flox-activate-d-scripts}/zdotdir"

  ${builtins.readFile ../../pkgdb/src/buildenv/assets/activate.sh}
''
