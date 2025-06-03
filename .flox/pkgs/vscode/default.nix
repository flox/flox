{ vscode-extensions, vscode-with-extensions }:

let
  vscodeExtensions = with pkgs.vscode-extensions; [
    aaronduino.nix-lsp
    eamodio.gitlens
    editorconfig.editorconfig
    eliverlara.andromeda
    flox.flox
    github.copilot
    github.copilot-chat
    github.github-vscode-theme
    github.vscode-github-actions
    golang.go
    mads-hartmann.bash-ide-vscode
    mariorodeghiero.vue-theme
    mkhl.shfmt
    ms-python.black-formatter
    ms-python.debugpy
    ms-python.isort
    ms-python.python
    ms-python.vscode-pylance
    ms-toolsai.jupyter
    ms-toolsai.jupyter-keymap
    ms-toolsai.jupyter-renderers
    ms-toolsai.vscode-jupyter-cell-tags
    ms-toolsai.vscode-jupyter-slideshow
    ms-vscode-remote.remote-ssh
    ms-vscode-remote.remote-ssh-edit
    ms-vscode.cpptools
    ms-vscode.cpptools-themes
    ms-vscode.makefile-tools
    ms-vscode.remote-explorer
    redhat.vscode-xml
    rust-lang.rust-analyzer
    vadimcn.vscode-lldb
    vscodevim.vim
  ];
in
pkgs.vscode-with-extensions.override {
  inherit vscodeExtensions;
}
