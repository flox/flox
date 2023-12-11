{
  runCommand,
  flox-cli,
  nix,
  sqlite,
  flox-pkgdb,
  cacert,
  gnutar,
  zstd,
  time,
  gawk,
  parallel,
  diffutils,
  flox-env-builder,
  inputs,
  rsync,
  closureInfo,
}: let
  system = flox-cli.system;
  cached_home =
    runCommand "pure-flox-tests" {
      nativeBuildInputs = [flox-cli nix sqlite flox-pkgdb gnutar zstd];
    } ''
      set -euo pipefail

      function setup_nix_isolated(){
      export NIX_SSL_CERT_FILE=${cacert}/etc/ssl/certs/ca-bundle.crt
      export USER=$(whoami)
      export HOME=$(mktemp -d)
      cd $HOME
      mkdir -p $HOME/.config/flox
      mkdir -p ~/.cache/nix
      export NIX_CONFIG="experimental-features = flakes nix-command
                  store = $PWD/temp"
      }

      function setup_cached_nixpkgs(){
      sqlite3 ~/.cache/nix/fetcher-cache-v1.sqlite '
      CREATE TABLE Cache (
          input     text not null,
          info      text not null,
          path      text not null,
          immutable integer not null,
          timestamp integer not null,
          primary key (input)
      );
      '
      cat <<EOF > cmd.sql
      INSERT INTO Cache VALUES (
      '{"rev":"${inputs.nixpkgs.sourceInfo.rev}","type":"git-tarball"}','{"lastModified":1701947945,"rev":"${inputs.nixpkgs.sourceInfo.rev}"}','${inputs.nixpkgs.sourceInfo.outPath}',1,1701982865);

      INSERT INTO Cache VALUES ('{"name":"source","type":"tarball","url":"https://github.com/NixOS/nixpkgs/archive/${inputs.nixpkgs.sourceInfo.rev}.tar.gz"}','{"etag":"1f1662fb764f1eb7ec3e036c81d277c67d1b5fcb098b493619b509fba35e3c42","lastModified":1701952659}','${inputs.nixpkgs.sourceInfo.outPath}','1',1702050920);
      EOF
      sqlite3 ~/.cache/nix/fetcher-cache-v1.sqlite < cmd.sql

      echo forcing path to exist in sandbox:"${"path://${inputs.nixpkgs.sourceInfo.outPath}?narHash=${inputs.nixpkgs.sourceInfo.narHash}"}"

      nix store add-path ${inputs.nixpkgs.sourceInfo.outPath} --name source

      }

      setup_flox_settings(){
      mkdir -p ~/.config/flox
      echo $HOME
      cat <<EOF > ~/.config/flox/global-manifest.toml
      [options.allow]
      # Whether non-FOSS packages should appear in search/install results.
      unfree = true

      # Whether "broken" packages should appear in search/install results.
      # These are packages with known build/runtime issues.
      broken = false

      # Limit search/install results to only packages which
      # certain SPDX Identifiers by listing them below.
      # See all valid license ids here: https://spdx.org/licenses/
      ## licenses = ["LGPL2-or-later"]

      [options.semver]
      # Whether unstable/pre-release versions of software should be
      # preferred over the latest stable release in search/install results.
      prefer-pre-releases = false
      EOF

      export FLOX_DISABLE_METRICS=true
      export _PKGDB_GA_REGISTRY_REF_OR_REV="${inputs.nixpkgs.sourceInfo.rev}"
      }

      setup_nix_isolated
      setup_cached_nixpkgs
      setup_flox_settings

      flox search hello
      tar -I zstd -cf $out --exclude=temp .
    '';

  create_home =
    runCommand "home" {
      nativeBuildInputs = [flox-cli nix sqlite flox-pkgdb gnutar zstd];
    } ''
      mkdir $out
      cd $out
      tar -I zstd -xf ${cached_home}
    '';
  search =
    runCommand "search2" {
      nativeBuildInputs = [flox-cli nix sqlite flox-pkgdb gnutar zstd time gawk parallel diffutils];
      passthru = {inherit create_home;};
    } ''
      function setup_nix_isolated(){
      export NIX_SSL_CERT_FILE=${cacert}/etc/ssl/certs/ca-bundle.crt
      export USER=$(whoami)
      export HOME=$(mktemp -d -u)
      cp -r --no-preserve=mode ${create_home} $HOME

      export NIX_CONFIG="experimental-features = flakes nix-command
                  store = $HOME/temp"

      echo forcing path to exist in sandbox:"${"path://${inputs.nixpkgs.sourceInfo.outPath}?narHash=${inputs.nixpkgs.sourceInfo.narHash}"}"

      cd $HOME
      export FLOX_DISABLE_METRICS=true
      export _PKGDB_GA_REGISTRY_REF_OR_REV="${inputs.nixpkgs.sourceInfo.rev}"

      }

      setup_nix_isolated

      mkdir $out

      parallel --will-cite -v --res $out flox search {} ::: hello hello@2.12.1 'hello@>=1' hello@2.x hello@=2.12 hello@v2 'hello@>1 <3'

      find $out -iname stderr -delete
    '';
  # TODO: can easily check output via: diff -r ${../../tests/test_output} $out
in
  runCommand "activate" {
    nativeBuildInputs = [flox-cli nix sqlite flox-pkgdb gnutar zstd time gawk parallel diffutils rsync];
    passthru = {inherit create_home;};
    # __impure = true;
  } ''
    export TIME='%E: %C'

    function setup_nix_isolated(){
    export NIX_SSL_CERT_FILE=${cacert}/etc/ssl/certs/ca-bundle.crt
    export USER=$(whoami)
    export HOME=$(mktemp -d -u)
    export NIX_CONFIG="experimental-features = flakes nix-command
                store = $HOME/temp"

    command time cp --reflink=auto -as --no-preserve=mode ${create_home} $HOME
    materialize(){
      command time cp --reflink=auto -rfL "$1"{,.bak}
      rm -rf "$1"
      mv "$1"{.bak,}
      chmod -R +w "$1"
    }
    materialize "$HOME"/.config/flox
    materialize "$HOME"/.cache/flox

    cd $HOME
    export FLOX_DISABLE_METRICS=true
    export _PKGDB_GA_REGISTRY_REF_OR_REV="${inputs.nixpkgs.sourceInfo.rev}"

    }

    setup_nix_isolated

    closureInfo=${closureInfo {
      rootPaths = [
        inputs.nixpkgs.sourceInfo.outPath
        inputs.nixpkgs.legacyPackages.${system}.hello
        flox-env-builder.PROFILE_D_SCRIPT_DIR
      ];
    }}
    mkdir -p $HOME/temp/nix/store
    command time xargs -I % cp -a -t $HOME/temp/nix/store/ % < $closureInfo/store-paths
    command time nix-store --load-db < $closureInfo/registration

    command time flox search hello
    command time flox init
    # command time flox --debug install hello
  ''
