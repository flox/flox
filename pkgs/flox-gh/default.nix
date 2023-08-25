{
  gh,
  gitMinimal,
  lib,
  makeWrapper,
}:
gh.overrideAttrs (oldAttrs: {
  pname = "flox-${oldAttrs.pname}";
  nativeBuildInputs = (oldAttrs.nativeBuildInputs or []) ++ [makeWrapper];
  patches = (oldAttrs.patches or []) ++ [(./flox-gh.patch + ".v${oldAttrs.version}")];
  postInstall = ''
    mv $out/bin/gh $out/bin/flox-gh
    wrapProgram $out/bin/flox-gh \
      --run '# This script should only be invoked by flox with $FLOX_*_HOME defined.' \
      --run 'set -eu' \
      --run 'export XDG_CONFIG_HOME="$FLOX_CONFIG_HOME"' \
      --run 'export XDG_STATE_HOME="$FLOX_STATE_HOME"' \
      --run 'export XDG_DATA_HOME="$FLOX_DATA_HOME"' \
      --run '# Unset gh-related environment variables.' \
      --run 'unset GITHUB_TOKEN GH_TOKEN GITHUB_ENTERPRISE_TOKEN GH_ENTERPRISE_TOKEN' \
      --run 'unset GH_CONFIG_DIR GH_HOST GH_PATH GH_REPO' \
      --prefix PATH : "${lib.makeBinPath [gitMinimal]}"
  '';
})
