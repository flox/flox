{
  self,
  stdenv,
  sqlite,
  doxygen,
  pkg-config,
  nlohmann_json,
  boost,
  argparse,
  flox-pkgdb,
  sqlite3pp,
  runCommand,
  # sql-builder,
}: let
  profile_d_scripts = runCommand "profile-d-scripts" {} ''
    mkdir -p $out/etc/profile.d
    cp -r ${../../assets/mkEnv/profile.d}/* $out/etc/profile.d/
  '';

  envs = {
    boost_CPPFLAGS = "-I" + boost.dev.outPath + "/include";
    # FIXME: There's way more flags than this, reference the `pkgdb.pc'
    #        `pkg-config' file to get the complete list.
    pkgdb_CLFAGS =
      if flox-pkgdb == null
      then ""
      else "-I" + flox-pkgdb.outPath + "/include";
    pkgdb_LIBDIR =
      if flox-pkgdb == null
      then ""
      else flox-pkgdb.outPath + "/lib";

    PKGDB_DIR = ../../pkgdb;

    PROFILE_D_SCRIPT_DIR = profile_d_scripts;
    SET_PROMPT_BASH_SH = "${../../assets/mkEnv/set-prompt-bash.sh}";

    libExt = stdenv.hostPlatform.extensions.sharedLibrary;
  };
in
  stdenv.mkDerivation ({
      pname = "flox-env-builder";
      version = builtins.replaceStrings ["\n"] [""] (builtins.readFile ./../../env-builder/version);

      src = builtins.path {
        path = "${self}/env-builder";
        filter = name: type: let
          bname = baseNameOf name;
          # Tests require internet so there's no point in including them
          ignores = ["out" "bin" "lib" "tests"];
          ext = let
            m = builtins.match ".*\\.([^.]+)" name;
          in
            if m == null
            then ""
            else builtins.head m;
          ignoredExts = ["o" "so" "dylib" "nix"];
          notIgnored =
            (! (builtins.elem bname ignores))
            && (! (builtins.elem ext ignoredExts));
          notResult = (builtins.match "result(-*)?" bname) == null;
        in
          notIgnored && notResult;
      };

      BUILD_AUX_DIR = ./../../build-aux;

      nativeBuildInputs = [pkg-config];

      buildInputs =
        [
          sqlite.dev
          nlohmann_json
          argparse
          sqlite3pp
          boost.dev
          # We allow `flox-pkgdb' to be null so that we can use the `devShell'
          # without having to build `flox-pkgdb' first.
        ]
        ++ (
          if flox-pkgdb != null
          then [flox-pkgdb]
          else []
        );

      configurePhase = ''
        runHook preConfigure;
        export PREFIX="$out";
        if [[ "''${enableParallelBuilding:-1}" = 1 ]]; then
          makeFlagsArray+=( '-j4' );
        fi
        runHook postConfigure;
      '';

      # Checks require internet
      doCheck = false;
      doInstallCheck = false;

      meta.mainProgram = "env-builder";

      passthru = {
        inherit
          envs
          flox-pkgdb
          ;

        ciPackages = [
          # For docs
          doxygen
        ];

        devPackages = [
        ];

        devEnvs =
          envs
          // {
            PKGDB_DIR = "";
          };

        devShellHook = ''
          #  # Find the project root and add the `bin' directory to `PATH'.
          if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
            PROJECT_ROOT_PATH=$( git rev-parse --show-toplevel; );
            PATH="$PROJECT_ROOT_PATH/env-builder/bin:$PATH";
            # TODO: if not in nix store we need to add this to the nix store in flox-env-builder
            #export PROFILE_D_SCRIPT_DIR="$PROJECT_ROOT_PATH/assets/mkEnv/profile.d";
            #export SET_PROMPT_BASH_SH="$PROJECT_ROOT_PATH/assets/mkEnv/set-prompt-bash.sh";
          fi
        '';
      };
    }
    // envs)
