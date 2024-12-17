{pkgs, flox-src, flox, flox-pkgdb, flox-buildenv, flox-activation-scripts, closureInfo }:
let
  flox-src = ../..;
  bats = pkgs.bats.withLibraries (p: [
    p.bats-support
    p.bats-assert
    p.bats-file
    p.bats-detik
  ]);
  batsLibraryPaths = "${flox-src}/cli/tests/setup_suite.bash:${flox-src}/cli/tests/test_support.bash";
in
pkgs.testers.runCommand
  {
    name = "flox-test-install-hello";
    # __impure = true;
    # requiredSystemFeatures = [ "recursive-nix" ];
    buildInputs = [
      flox
      flox-buildenv
      flox-activation-scripts.build_wrapper
      flox-activation-scripts
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
      pkgs.time
    ];
    FLOX_DISABLE_METRICS = "true";
    SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
    NIX_SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
    BATS_LIB_PATH = "${batsLibraryPaths}";
    FLOX_BIN = "${flox}/bin/flox";
    PKGDB_BIN = "${flox-pkgdb}/bin/pkgdb";
    NIX_BIN = "${pkgs.nix}/bin/nix";
    TESTS_DIR = "${flox-src}/cli/tests";
    GENERATED_DATA = "${flox-src}/test_data/generated";
    INPUT_DATA = "${flox-src}/test_data/input_data";
    __FT_RAN_XDG_REALS_SETUP = "1";

    __FLOX_NO_EUID_WARNING = "1";
  script =
  ''
    mkdir t
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

    export TEST_ROOT=$(pwd)/test-tmp
    export HOME=$(mktemp -d)
    export NIX_CONFIG="experimental-features = flakes nix-command"

    closureInfo=${closureInfo {
      rootPaths = [
        #inputs.nixpkgs.sourceInfo.outPath
        #inputs.nixpkgs.legacyPackages.${system}.hello
        flox-activation-scripts.build_wrapper
        flox-activation-scripts
	flox-pkgdb
	flox-buildenv
      ];
    }}
    mkdir -p $TEST_ROOT/nix/store/
    # command time xargs -I % cp -a -t $TEST_ROOT/nix/store/ % < $closureInfo/store-paths
    command time xargs -I % ln -s -t $TEST_ROOT/nix/store/ % < $closureInfo/store-paths
    ls -alh $TEST_ROOT/nix/store/*/bin/mkdir -alh

    export NIX_CONFIG="experimental-features = flakes nix-command
    store = $TEST_ROOT"
    nix-store --load-db < $closureInfo/registration

    flox init --debug
    echo created
    flox install hello --debug -vvv
    if ! grep "installed to environment" output ; then
        echo "did not install"
        exit 1
    fi
    touch $out
  '';
  }

# 1. Long time to setup store
# 2. impure - testers.runCommand




