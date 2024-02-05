# compiled manpages
{
  runCommand,
  pandoc,
  fd,
  installShellFiles,
}:
runCommand "flox-manpages" {
  src = builtins.path {
    name = "flox-manpage-src";
    path = "${./../../cli/flox/doc}";
  };
  buildInputs = [pandoc fd installShellFiles];
} ''
  buildDir=$(pwd)/__build

  mkdir "$out"
  mkdir "$buildDir"
  pushd "$src"

  fd "flox.*.md" ./ -x \
    pandoc -t man \
      -L ${./pandoc-filters/include-files.lua} \
      --standalone \
      -o "$buildDir/{/.}.1" \
      {}

    ls $buildDir

    installManPage $buildDir/*;
''
