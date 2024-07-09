{
  bash,
  coreutils,
  findutils,
  gnused,
  ld-floxlib,
  runCommand,
  shellcheck,
  stdenv,
  process-compose,
}: let
  ld-floxlib_so =
    if stdenv.isLinux
    then "${ld-floxlib}/lib/ld-floxlib.so"
    else "__LINUX_ONLY__";
in
  runCommand "flox-activation-scripts" {
    buildInputs = [bash coreutils gnused];
  } ''
    cp -R ${../../assets/activation-scripts} $out

    substituteInPlace $out/activate \
      --replace "@coreutils@" "${coreutils}" \
      --replace "@gnused@" "${gnused}" \
      --replace "@out@" "$out" \
      --replace "@process-compose@" "${process-compose}/bin/process-compose" \
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
      substituteInPlace $i --replace "@ld-floxlib@" "${ld-floxlib_so}"
    done

    ${shellcheck}/bin/shellcheck \
      $out/activate \
      $out/activate.d/bash \
      $out/activate.d/set-prompt.bash \
      $out/etc/profile.d/*
  ''
