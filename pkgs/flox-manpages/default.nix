# compiled manpages
{
  writeShellScript,
  runCommand,
  pandoc,
  fd,
  installShellFiles,
}: let
  compileManPageBin = writeShellScript "compile" ''
    source="$1"
    shift
    dest="$1"
    shift

    # tools
    pandoc=${pandoc}/bin/pandoc

    # Compile manpage
    #
    # Produe a standalone manpage with header and footer
    # Apply custom filters:
    # * `include-files.lua` to include other markdown files
    # * `filter-links.lua` to mitigate against <https://github.com/jgm/pandoc/issues/9458>
    $pandoc                       \
      -L ${./pandoc-filters/include-files.lua} \
      -L ${./pandoc-filters/filter-links.lua}  \
      --standalone                             \
      --strip-comments                         \
      --from markdown                          \
      --to man                                 \
       $source                                 \
    > "$dest"
  '';
in
  runCommand "flox-manpages" {
    src = builtins.path {
      name = "flox-manpage-src";
      path = "${./../../cli/flox/doc}";
    };
    buildInputs = [fd installShellFiles];
  } ''
    buildDir=$(pwd)/__build

    mkdir "$out"
    mkdir "$buildDir"
    pushd "$src"


    fd ".*\.md" -d 1 ./ -x ${compileManPageBin} {} $buildDir/{/.}.1
    rm -f $buildDir/manifest.toml.1
    ${compileManPageBin} manifest.toml.md $buildDir/manifest.toml.5

    ls $buildDir

    installManPage $buildDir/*;
  ''
