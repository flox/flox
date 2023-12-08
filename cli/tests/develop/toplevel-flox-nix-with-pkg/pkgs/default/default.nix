{
  mkShell,
  ripgrep,
}:
mkShell {
  packages = [ripgrep];
  shellHook = ''
    echo "developing package"
  '';
}
