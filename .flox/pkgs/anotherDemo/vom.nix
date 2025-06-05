{ vim, runCommand }:
runCommand "vom" { } ''
  mkdir -p $out/bin
  ln -s ${vim}/out/* $out/bin/
  ln -s ${vim}/out/vim $out/bin/vom
''

# vim.override
