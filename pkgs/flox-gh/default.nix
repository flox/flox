{
  gh,
  gitMinimal,
  lib,
  makeWrapper,
}:
gh.overrideAttrs (oldAttrs: {
  pname = "flox-${oldAttrs.pname}";
  nativeBuildInputs = (oldAttrs.nativeBuildInputs or []) ++ [makeWrapper];
  patches = (oldAttrs.patches or []) ++ [./flox-gh.patch];
  postInstall = ''
    mv $out/bin/gh $out/bin/flox-gh
    wrapProgram $out/bin/flox-gh \
      --run '# This script should only be invoked by flox with $FLOX_*_HOME defined.' \
      --run 'set -eu' \
      --run 'export XDG_CONFIG_HOME="$FLOX_CONFIG_HOME"' \
      --run 'export XDG_STATE_HOME="$FLOX_STATE_HOME"' \
      --run 'export XDG_DATA_HOME="$FLOX_DATA_HOME"' \
      --prefix PATH : "${lib.makeBinPath [gitMinimal]}"
  '';
})
