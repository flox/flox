let
  inherit (pkgs) linkFarm;
  pkgs = import <nixpkgs> {
    overlays = [ ];
    config = { };
  };
  flox-src = (builtins.getFlake "git+file:/home/bryan/flox?ref=bryanhonof.impure-tests");
  flox = flox-src.packages."${pkgs.system}".default;
  pkgdb = flox-src.packages."${pkgs.system}".flox-pkgdb;
  bats = pkgs.bats.withLibraries (p: [
    p.bats-support
    p.bats-assert
    p.bats-file
    p.bats-detik
  ]);
  batsLibraryPaths = "${flox-src}/cli/tests/setup_suite.bash:${flox-src}/cli/tests/test_support.bash";
in
pkgs.runCommand "flox-activate"
  {
    __impure = true;
    requiredSystemFeatures = [ "recursive-nix" ];
    buildInputs = [
      flox
      bats
      pkgs.nix
      pkgs.jq
      pkgs.expect
      pkgs.which
      pkgs.openssh
      pkgs.zsh
      pkgs.tcsh
      pkgs.fish
      pkgs.dash
      pkgs.bashInteractive
      pkgs.coreutils
      pkgs.curl
      pkgs.diffutils
      pkgs.entr
      pkgs.findutils
      pkgs.gawk
      pkgs.git
      pkgs.gnugrep
      pkgs.gnupg
      pkgs.gnused
      pkgs.gnutar
      pkgs.man
      pkgs.parallel
      pkgs.pstree
      pkgs.unixtools.util-linux
      pkgs.which
      pkgs.yq
      pkgs.process-compose
      pkgs.procps
    ];
    FLOX_DISABLE_METRICS = "true";
    SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
    NIX_SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
    BATS_LIB_PATH = "${batsLibraryPaths}";
    FLOX_BIN = "${flox}/bin/flox";
    PKGDB_BIN = "${pkgdb}/bin/pkgdb";
    NIX_BIN = "${pkgs.nix}/bin/nix";
    TESTS_DIR = "${flox-src}/cli/tests";
    GENERATED_DATA = "${flox-src}/test_data/generated";
    INPUT_DATA = "${flox-src}/test_data/input_data";
    __FT_RAN_XDG_REALS_SETUP = "1";
  }
  ''
    mkdir $out t
    cd t
    export LD_PRELOAD="${pkgs.sssd}/lib/libnss_sss.so.2"
    export HOME="$PWD"
    export USER="floxfan"
    export FLOX_CONFIG_DIR="$PWD/flox-config"
    export FLOX_DATA_DIR="$PWD/flox-data"
    export FLOX_STATE_DIR="$PWD/flox-state"
    FLOX_CACHE_DIR="$(mktemp -d -p "$PWD")"
    export FLOX_CACHE_DIR
    export _FLOX_NIX_STORE_URL="$NIX_REMOTE"

    export XDG_CONFIG_HOME="$HOME/.config"
    export XDG_CACHE_HOME="$HOME/.cache"
    export XDG_DATA_HOME="$HOME/.local/share"
    export XDG_STATE_HOME="$HOME/.local/state"

    export REAL_HOME="$HOME"
    export REAL_XDG_CONFIG_HOME="$XDG_CONFIG_HOME"
    export REAL_XDG_CACHE_HOME="$XDG_CACHE_HOME"
    export REAL_XDG_DATA_HOME="$XDG_DATA_HOME"
    export REAL_XDG_STATE_HOME="$XDG_STATE_HOME"
    unset HOME XDG_CONFIG_HOME XDG_CACHE_HOME XDG_DATA_HOME XDG_STATE_HOME XDG_DATA_DIRS
    export __FT_RAN_XDG_REALS_SETUP=:

    cat <<EOF > fake-ssh-key
    -----BEGIN OPENSSH PRIVATE KEY-----
    b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
    QyNTUxOQAAACAWB/0/BjnFoyVW3TKHMOf52GC4zebVU10laVtONbLXUwAAAJhxxor+ccaK
    /gAAAAtzc2gtZWQyNTUxOQAAACAWB/0/BjnFoyVW3TKHMOf52GC4zebVU10laVtONbLXUw
    AAAEBGIICsrI73axapAWifW5dBvP5axxp0ekZn0teFQEs0MhYH/T8GOcWjJVbdMocw5/nY
    YLjN5tVTXSVpW041stdTAAAAFWJyeWFuaG9ub2ZAQnJ5YW5zLU1CUA==
    -----END OPENSSH PRIVATE KEY-----
    EOF
    chmod 600 fake-ssh-key

    cat <<EOF > fake-ssh-key.pub
    ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBYH/T8GOcWjJVbdMocw5/nYYLjN5tVTXSVpW041stdT bryanhonof@Bryans-MBP
    EOF

    export FLOX_TEST_SSH_KEY="$PWD/fake-ssh-key"

    bats \
      --print-output-on-failure \
      --show-output-of-passing-tests \
      --gather-test-outputs-in "$out/test-outputs" \
      --timing \
      --verbose-run \
      "${flox-src}/cli/tests/activate.bats"
  ''
