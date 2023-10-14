{nixVersions, ...}:
nixVersions.nix_2_15.overrideAttrs (oldAttrs: {
  patches =
    (oldAttrs.patches or [])
    ++ [
      ../flox-bash/nix-patches/CmdProfileBuild.patch
      ../flox-bash/nix-patches/CmdSearchAttributes.patch
      ../flox-bash/nix-patches/update-profile-list-warning.patch
      ../flox-bash/nix-patches/multiple-github-tokens.2.13.2.patch
      ../flox-bash/nix-patches/curl_flox_version.patch
      ../flox-bash/nix-patches/no-default-prefixes-hash.2.15.1.patch
    ];
})
