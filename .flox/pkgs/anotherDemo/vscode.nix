{ vscode-with-extensions, vscode-extensions }:
vscode-with-extensions.override {
  vscodeExtensions = [
    vscode-extensions.rust-lang.rust-analyzer
  ];
}
