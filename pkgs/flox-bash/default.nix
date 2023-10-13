{
  flox-src,
  inputs,
  stdenv,
  ansifilter,
  bashInteractive,
  coreutils,
  curl,
  dasel,
  diffutils,
  expect,
  findutils,
  flox-gh,
  gawk,
  gitMinimal,
  gh,
  gnugrep,
  gnused,
  gnutar,
  gum,
  gzip,
  hostPlatform,
  jq,
  less, # Required by man, believe it or not  :-(
  lib,
  libossp_uuid,
  makeWrapper,
  man,
  nix-editor,
  nixVersions,
  openssh,
  pandoc,
  parser-util,
  ps,
  shellcheck,
  shfmt,
  substituteAll,
  util-linuxMinimal,
  which,
  semver,
  builtfilter-rs,
  floxVersion,
  flox-pkgdb,
  unixtools,
  cacert,
  glibcLocales,
  darwin,
}: let
  pkgdb = inputs.pkgdb.packages.flox-pkgdb;

  nixPatched = nixVersions.nix_2_15.overrideAttrs (oldAttrs: {
    patches =
      (oldAttrs.patches or [])
      ++ [
        ./nix-patches/CmdProfileBuild.patch
        ./nix-patches/CmdSearchAttributes.patch
        ./nix-patches/update-profile-list-warning.patch
        ./nix-patches/multiple-github-tokens.2.13.2.patch
        ./nix-patches/curl_flox_version.patch
        ./nix-patches/no-default-prefixes-hash.2.15.1.patch
      ];
  });

  # TODO: floxActivateFish, etc.
  floxActivateBashDarwin = substituteAll {
    src = builtins.toFile "activate.bash" (
      (builtins.readFile ./activate.bash)
      + (builtins.readFile ./activate.darwin.bash)
    );
    inherit cacert;
    inherit (darwin) locale;
    coreFoundation = darwin.CF;
  };
  floxActivateBashLinux = substituteAll {
    src = builtins.toFile "activate.bash" (
      (builtins.readFile ./activate.bash)
      + (builtins.readFile ./activate.linux.bash)
    );
    inherit cacert glibcLocales;
  };
  floxActivateBash =
    if hostPlatform.isLinux
    then floxActivateBashLinux
    else if hostPlatform.isDarwin
    then floxActivateBashDarwin
    else throw "unsupported system variant";

  # read commitizen config file as the single source of version
  czToml = lib.importTOML (flox-src + "/.cz.toml");
in
  stdenv.mkDerivation rec {
    pname = "flox-bash";
    version = floxVersion;
    src = builtins.path {path = flox-src + "/flox-bash";};
    nativeBuildInputs =
      [makeWrapper pandoc shellcheck shfmt which]
      # nix-provided expect not working on Darwin (#441)
      ++ lib.optionals hostPlatform.isLinux [expect];
    buildInputs = [
      ansifilter
      bashInteractive
      coreutils
      curl
      dasel
      diffutils
      findutils
      flox-gh
      gawk
      unixtools.getent
      gitMinimal
      gh
      gnugrep
      gnused
      gnutar
      gum
      gzip
      jq
      less
      libossp_uuid
      man
      nixPatched
      nix-editor
      openssh
      ps
      util-linuxMinimal
      semver
      builtfilter-rs
      parser-util
      flox-pkgdb
    ];
    makeFlags =
      [
        "PREFIX=$(out)"
        "VERSION=${version}"
        "FLOXPATH=$(out)/libexec/flox:${lib.makeBinPath buildInputs}"
        "NIXPKGS_CACERT_BUNDLE_CRT=${cacert}/etc/ssl/certs/ca-bundle.crt"
        "FLOX_ACTIVATE_BASH=${floxActivateBash}"
      ]
      ++ lib.optionals hostPlatform.isLinux [
        "LOCALE_ARCHIVE=${glibcLocales}/lib/locale/locale-archive"
      ]
      ++ lib.optionals hostPlatform.isDarwin [
        "NIX_COREFOUNDATION_RPATH=${darwin.CF}/Library/Frameworks"
        "PATH_LOCALE=${darwin.locale}/share/locale"
      ];

    postInstall = ''
      # Some programs cannot function without git, ssh, and other
      # programs in their PATH. We have gone gone to great lengths
      # to avoid leaking /nix/store paths into PATH, so in order
      # to correct for these shortcomings we need to arrange for
      # flox to invoke our wrapped versions of these programs in
      # preference to the ones straight from nixpkgs.
      #
      # Note that we must prefix the path to avoid prompting the
      # user to download XCode at runtime on MacOS.
      #
      # TODO: replace "--argv0 '$0'" with "--inherit-argv0" once Nix
      #       version advances to the version that supports it.
      #
      mkdir -p $out/libexec
      makeWrapper ${nixPatched}/bin/nix $out/libexec/flox/nix --argv0 '$0' \
        --prefix PATH : "${gitMinimal}/bin"
      makeWrapper ${gh}/bin/gh $out/libexec/flox/gh --argv0 '$0' \
        --prefix PATH : "${gitMinimal}/bin"

      # Rewrite /usr/bin/env bash to the full path of bashInteractive.
      # Use --host to resolve using the runtime path.
      patchShebangs --host $out/libexec/flox/flox $out/libexec/flox/darwin-path-fixer
    '';

    doInstallCheck = true;
    postInstallCheck = ''
      # Quick unit test to ensure that we are not using any "naked"
      # commands within our scripts. Doesn't hit all codepaths but
      # catches most of them.
      env -i USER=`id -un` HOME=$PWD $out/bin/flox help > /dev/null
    '';

    passthru.nixPatched = nixPatched;
  }
