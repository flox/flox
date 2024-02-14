{
  fetchFromGitHub,
  buildGoModule,
  installShellFiles,
  stdenv,
  gitMinimal,
  makeWrapper,
}: let
  version = "2.32.1";
in
  buildGoModule {
    pname = "flox-gh";

    inherit version;

    src = fetchFromGitHub {
      owner = "cli";
      repo = "cli";
      rev = "v${version}";
      hash = "sha256-DfcafkgauO0mlMEJTfR7hjnkY1QJ4dUyrWv/bqJlVAo=";
    };

    vendorHash = "sha256-7Izhqma/zukH9M7EvV9I4axefVaTDoNVXQmLx+GjAt0=";

    nativeBuildInputs = [installShellFiles makeWrapper];

    patches = [./flox-gh.patch.v2.32.1];

    buildPhase = let
      maybeManpages =
        if stdenv.buildPlatform.canExecute stdenv.hostPlatform
        then "manpages"
        else "";
    in ''
      runHook preBuild;
      make GO_LDFLAGS='-s -w' GH_VERSION=${version} bin/gh ${maybeManpages};
      runHook postBuild;
    '';

    installPhase = ''
      runHook preInstall;
      install -Dm755 bin/gh -t "$out/bin";
      runHook postInstall;
    '';

    # most tests require network access
    doCheck = false;

    postInstall = ''
      mv "$out/bin/gh" "$out/bin/flox-gh";
      wrapProgram "$out/bin/flox-gh"                                             \
        --run '# This should only be invoked by flox with $FLOX_*_HOME defined.' \
        --run 'set -eu'                                                          \
        --run 'export XDG_CONFIG_HOME="$FLOX_CONFIG_DIR"'                       \
        --run 'export XDG_STATE_HOME="$FLOX_STATE_HOME"'                         \
        --run 'export XDG_DATA_HOME="$FLOX_DATA_HOME"'                           \
        --run '# Unset gh-related environment variables.'                        \
        --run 'unset GITHUB_TOKEN GH_TOKEN GITHUB_ENTERPRISE_TOKEN'              \
        --run 'unset GH_ENTERPRISE_TOKEN GH_CONFIG_DIR GH_HOST GH_PATH GH_REPO'  \
        --prefix PATH : "${gitMinimal}/bin"                                      \
      ;
    '';
  }
