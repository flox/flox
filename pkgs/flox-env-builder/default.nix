{
  self,
  stdenv,
  sqlite,
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
    pkgdb_CLFAGS = "-I" + flox-pkgdb.outPath + "/include";

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

      nativeBuildInputs = [pkg-config];

      buildInputs = [
        sqlite.dev
        nlohmann_json
        argparse
        flox-pkgdb
        sqlite3pp
        boost.dev
      ];

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

      passthru = {
        inherit
          envs
          flox-pkgdb
          ;

        devPackages = [
        ];

        devEnvs =
          envs
          // {
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
