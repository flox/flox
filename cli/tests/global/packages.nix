{ rev, system }:
let
  pkgs = (builtins.getFlake "github:nixos/nixpkgs/${rev}").legacyPackages.${system};
  runtime = pkgs.runCommand "global-private-runtime" { } ''
    mkdir -p $out/libexec
    printf 'private runtime\n' > $out/libexec/message
    cat > $out/libexec/private-helper <<'EOF'
    #!${pkgs.bash}/bin/bash
    echo private-helper
    EOF
    chmod +x $out/libexec/private-helper
  '';
  shared = pkgs.writeShellScriptBin "shared" "echo shared";
  package =
    name:
    pkgs.runCommand "global-${name}" { } ''
      mkdir -p $out/bin $out/sbin
      ln -s ${shared}/bin/shared $out/bin/shared
      cat > $out/bin/${name} <<'EOF'
      #!${pkgs.bash}/bin/bash
      IFS= read -r message < ${runtime}/libexec/message
      printf '%s\n' "$message"
      EOF
      chmod +x $out/bin/${name}
      ln -s $out/bin/${name} $out/sbin/${name}
    '';
in
{
  first = package "first";
  second = package "second";
}
