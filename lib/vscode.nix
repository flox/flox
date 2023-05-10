{}: rec {
  # check pkgs.vscode-extensions ? extension
  isNixpkgsExtension = pkgs: extension:
    pkgs.lib.attrsets.hasAttrByPath [extension.publisher extension.name]
    pkgs.vscode-extensions;

  nixpkgsExtensions = pkgs: extensions:
    builtins.map (extension:
      pkgs.lib.attrsets.getAttrFromPath [extension.publisher extension.name]
      pkgs.vscode-extensions)
    extensions;

  # generate a list of full attribute paths for each extension string
  configuredVscode = pkgs: vscodeConfig: extensions:
    if vscodeConfig ? extensions
    then
      pkgs.vscode-with-extensions.override {
        vscodeExtensions = let
          partitioned = builtins.partition (x: isNixpkgsExtension pkgs x) extensions;
        in
          (nixpkgsExtensions pkgs partitioned.right)
          ++ (pkgs.vscode-utils.extensionsFromVscodeMarketplace partitioned.wrong);
      }
    else pkgs.vscode;
}
