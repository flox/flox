# ============================================================================ #
#
#
#
# ---------------------------------------------------------------------------- #
{
  stdenv,
  sqlite,
  pkg-config,
  nlohmann_json,
  nix,
  boost,
  argparse,
  semver,
  sqlite3pp,
  toml11,
  yaml-cpp,
}:
stdenv.mkDerivation {
  pname = "flox-pkgdb";
  version = builtins.replaceStrings ["\n"] [""] (builtins.readFile ./../../pkgdb/version);
  src = builtins.path {
    path = ./../../pkgdb;
    filter = name: type: let
      bname = baseNameOf name;
      ignores = [
        "default.nix"
        "pkg-fun.nix"
        "flake.nix"
        "flake.lock"
        ".ccls"
        ".ccls-cache"
        "compile_commands.json"
        ".git"
        ".gitignore"
        "out"
        "bin"
        "pkgs"
        "bear.d"
        ".direnv"
        ".clang-tidy"
        ".clang-format"
        ".envrc"
        ".github"
        "LICENSE"
      ];
      ext = let
        m = builtins.match ".*\\.([^.]+)" name;
      in
        if m == null
        then ""
        else builtins.head m;
      ignoredExts = ["o" "so" "dylib" "log"];
      notResult = (builtins.match "result(-*)?" bname) == null;
      notIgnored =
        (! (builtins.elem bname ignores))
        && (! (builtins.elem ext ignoredExts));
    in
      notIgnored && notResult;
  };
  propagatedBuildInputs = [semver nix];
  nativeBuildInputs = [pkg-config];
  buildInputs = [
    sqlite.dev
    nlohmann_json
    argparse
    sqlite3pp
    toml11
    yaml-cpp
    boost
    nix
  ];
  nix_INCDIR = nix.dev.outPath + "/include";
  boost_CFLAGS = "-isystem " + boost.dev.outPath + "/include";
  toml_CFLAGS = "-isystem " + toml11.outPath + "/include";
  yaml_PREFIX = yaml-cpp.outPath;
  libExt = stdenv.hostPlatform.extensions.sharedLibrary;
  SEMVER_PATH = semver.outPath + "/bin/semver";
  configurePhase = ''
    runHook preConfigure;
    export PREFIX="$out";
    if [[ "''${enableParallelBuilding:-1}" = 1 ]]; then
      makeFlagsArray+=( "-j''${NIX_BUILD_CORES:?}" );
    fi
    runHook postConfigure;
  '';
  # Checks require internet
  doCheck = false;
  doInstallCheck = false;
  outputs = ["out" "dev" "test"];
  meta.mainProgram = "pkgdb";
  postInstall = ''
    mkdir -p "$test/bin" "$test/lib"

    make tests

    for i in tests/*; do
      if (! [[ -d "$i" ]]) && [[ -x "$i" ]]; then
        cp "$i" "$test/bin/"
      fi
    done

    for i in "$out/lib/"*; do
      ln -s "$i" "$test/lib/"
    done
  '';
}
# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #

