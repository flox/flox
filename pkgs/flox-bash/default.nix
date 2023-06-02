{
  self,
  flox-src,
  inputs,
  stdenv,
  ansifilter,
  bashInteractive,
  coreutils,
  curl,
  dasel,
  diffutils,
  entr,
  expect,
  findutils,
  gawk,
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
  nixStable,
  pandoc,
  pkgs,
  shellcheck,
  shfmt,
  substituteAll,
  util-linuxMinimal,
  which,
  semver,
}: let
  # The getent package can be found in pkgs.unixtools.
  inherit (pkgs.unixtools) getent;

  # Choose a smaller version of git.
  git = pkgs.gitMinimal;

  nixPatched = nixStable.overrideAttrs (oldAttrs: {
    patches =
      (oldAttrs.patches or [])
      ++ [
        ./nix-patches/CmdProfileBuild.patch
        ./nix-patches/CmdSearchAttributes.patch
        ./nix-patches/update-profile-list-warning.patch
        ./nix-patches/multiple-github-tokens.2.13.2.patch
        ./nix-patches/curl_flox_version.patch
        ./nix-patches/no-default-prefixes-hash.patch
        ./nix-patches/subflake-outPaths.patch
      ];
  });

  # TODO: floxActivateFish, etc.
  floxActivateBashDarwin = substituteAll {
    src = builtins.toFile "activate.bash" (
      (builtins.readFile ./activate.bash)
      + (builtins.readFile ./activate.darwin.bash)
    );
    inherit (pkgs) cacert;
    inherit (pkgs.darwin) locale;
    coreFoundation = pkgs.darwin.CF;
  };
  floxActivateBashLinux = substituteAll {
    src = builtins.toFile "activate.bash" (
      (builtins.readFile ./activate.bash)
      + (builtins.readFile ./activate.linux.bash)
    );
    inherit (pkgs) cacert glibcLocales;
  };
  floxActivateBash =
    if hostPlatform.isLinux
    then floxActivateBashLinux
    else if hostPlatform.isDarwin
    then floxActivateBashDarwin
    else throw "unsupported system variant";

  bats = pkgs.bats.withLibraries (p: [p.bats-support p.bats-assert]);

  # read commitizen config file as the single source of version
  czToml = lib.importTOML (flox-src + "/.cz.toml");

  progDecls = out: let
    drvProgs = {
      ansifilter = ["ansifilter"];
      gawk = ["awk"];
      bashInteractive = ["bash" "sh"];
      curl = ["curl"];
      dasel = ["dasel"];
      gh = ["gh"];
      git = ["git"];
      gnugrep = ["grep"];
      gum = ["gum"];
      jq = ["jq"];
      nix-editor = ["nix-editor"];
      nix = ["nix" "nix-store"];
      gnused = ["sed"];
      diffutils = ["cmp"];
      getent = ["getent"];
      libossp_uuid = ["uuid"];
      gnutar = ["tar"];
      man = ["man"];
      findutils = ["xargs"];
      gzip = ["zgrep"];
      semver = ["semver"];
      util-linuxMinimal = ["column"];
      coreutils = [
        "date"
        "dirname"
        "stat"
        "tail"
        "tee"
        "touch"
        "tr"
        "uname"
        "pwd"
        "readlink"
        "realpath"
        "rm"
        "rmdir"
        "sleep"
        "sort"
        "id"
        "ln"
        "mkdir"
        "mktemp"
        "mv"
        "basename"
        "cat"
        "chmod"
        "cp"
        "cut"
      ];
    };
    drvs = {
      inherit
        ansifilter
        gawk
        bashInteractive
        curl
        dasel
        git
        gnugrep
        gum
        jq
        nix-editor
        gnused
        diffutils
        getent
        libossp_uuid
        gnutar
        man
        findutils
        gzip
        semver
        coreutils
        util-linuxMinimal
        ;
      # Wrapped progs
      gh = out + "/libexec/flox";
      nix = out + "/libexec/flox";
    };
    nixVars = builtins.concatStringsSep " " [
      "NIX_REMOTE"
      "NIX_SSL_CERT_FILE"
      "NIX_USER_CONF_FILES"
      "GIT_CONFIG_SYSTEM"
    ];
    genProg = drvOrPath: name: let
      binPath =
        if builtins.isString drvOrPath
        then drvOrPath
        else drvOrPath.outPath + "/bin";
      vname = builtins.replaceStrings ["-"] ["_"] name;
      vars =
        if ! (builtins.elem name ["nix" "nix-store"])
        then ""
        else ''
          exported_variables["${binPath}/${name}"]='${nixVars}';
        '';
    in
      ''
        _${vname}='${binPath}/${name}';
        invoke_${vname}='invoke ${binPath}/${name}';
      ''
      + vars;
    proc = acc: drvName: let
      drv = builtins.getAttr drvName drvs;
      progs = builtins.getAttr drvName drvProgs;
      decls = map (genProg drv) progs;
    in
      acc ++ decls;
    init = ''
      declare -Ax exported_variables;
      _PROGS_INJECTED=:;
    '';
    allDecls = builtins.foldl' proc [init] (builtins.attrNames drvProgs);
  in
    builtins.concatStringsSep "\n" allDecls;
in
  stdenv.mkDerivation rec {
    pname = "flox-bash";
    version = "${czToml.tool.commitizen.version}-${inputs.flox-floxpkgs.lib.getRev self}";
    src = flox-src + "/flox-bash";
    nativeBuildInputs =
      [bats entr makeWrapper pandoc shellcheck shfmt which]
      # nix-provided expect not working on Darwin (#441)
      ++ lib.optionals hostPlatform.isLinux [expect];
    propagatedBuildInputs = [
      ansifilter
      bashInteractive
      coreutils
      curl
      dasel
      diffutils
      findutils
      gawk
      getent
      git
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
      util-linuxMinimal
      semver
    ];
    makeFlags =
      [
        "PREFIX=${builtins.placeholder "out"}"
        "VERSION=${version}"
        "FLOXPATH=${builtins.placeholder "out"}/libexec/flox:${
          lib.makeBinPath propagatedBuildInputs
        }"
        "NIXPKGS_CACERT_BUNDLE_CRT=${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt"
        "FLOX_ACTIVATE_BASH=${floxActivateBash}"
      ]
      ++ lib.optionals hostPlatform.isLinux [
        "LOCALE_ARCHIVE=${pkgs.glibcLocales}/lib/locale/locale-archive"
      ]
      ++ lib.optionals hostPlatform.isDarwin [
        "NIX_COREFOUNDATION_RPATH=${pkgs.darwin.CF}/Library/Frameworks"
        "PATH_LOCALE=${pkgs.darwin.locale}/share/locale"
      ];

    progs_sh = progDecls (builtins.placeholder "out");

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
      mkdir -p "$out/libexec"

      makeWrapper ${nixPatched}/bin/nix "$out/libexec/flox/nix" --argv0 '$0' \
        --prefix PATH : "${lib.makeBinPath [git]}"

      ln -s ${nixPatched}/bin/nix-store "$out/libexec/flox/nix-store";

      makeWrapper ${gh}/bin/gh "$out/libexec/flox/gh" --argv0 '$0' \
        --prefix PATH : "${lib.makeBinPath [git]}"

      # Rewrite /usr/bin/env bash to the full path of bashInteractive.
      # Use --host to resolve using the runtime path.
      patchShebangs --host "$out/libexec/flox/flox"               \
                           "$out/libexec/flox/darwin-path-fixer"

      echo "$progs_sh" > "$out/lib/progs.sh";
      chmod +x "$out/lib/progs.sh";
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
