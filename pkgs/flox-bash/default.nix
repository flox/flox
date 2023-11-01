{
  flox-src,
  inputs,
  stdenv,
  bashInteractive,
  gitMinimal,
  gh,
  hostPlatform,
  lib,
  makeWrapper,
  nixVersions,
  pandoc,
  floxVersion,
  cacert,
  glibcLocalesUtf8,
  darwin,
}: let
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
in
  stdenv.mkDerivation rec {
    pname = "flox-bash";
    version = floxVersion;
    src = builtins.path {path = flox-src + "/flox-bash";};
    nativeBuildInputs = [makeWrapper pandoc];
    buildInputs = [
    ];
    makeFlags =
      [
        "PREFIX=$(out)"
        "VERSION=${version}"
        "FLOXPATH=$(out)/libexec/flox:${lib.makeBinPath buildInputs}"
        "NIXPKGS_CACERT_BUNDLE_CRT=${cacert}/etc/ssl/certs/ca-bundle.crt"
      ]
      ++ lib.optionals hostPlatform.isLinux [
        "LOCALE_ARCHIVE=${glibcLocalesUtf8}/lib/locale/locale-archive"
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

    passthru.nixPatched = nixPatched;
  }
