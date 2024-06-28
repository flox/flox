{
  bash,
  coreutils,
  findutils,
  gnused,
  runCommand,
  shellcheck,
}:
runCommand "flox-activate" {
  buildInputs = [bash coreutils gnused];
} ''
  cp -R ${../../pkgdb/src/buildenv/assets} $out

  substituteInPlace $out/activate \
    --replace "@coreutils@" "${coreutils}" \
    --replace "@gnused@" "${gnused}" \
    --replace "@out@" "$out" \
    --replace "/usr/bin/env bash" "${bash}/bin/bash"

  substituteInPlace $out/activate.d/bash \
    --replace "@gnused@" "${gnused}"
  substituteInPlace $out/activate.d/fish \
    --replace "@gnused@" "${gnused}"
  substituteInPlace $out/activate.d/tcsh \
    --replace "@gnused@" "${gnused}"
  substituteInPlace $out/activate.d/zsh \
    --replace "@gnused@" "${gnused}"

  for i in $out/etc/profile.d/*; do
    substituteInPlace $i --replace "@coreutils@" "${coreutils}"
    substituteInPlace $i --replace "@gnused@" "${gnused}"
    substituteInPlace $i --replace "@findutils@" "${findutils}"
  done

  ${shellcheck}/bin/shellcheck \
    $out/activate \
    $out/activate.d/bash \
    $out/activate.d/set-prompt.bash \
    $out/etc/profile.d/*
''
