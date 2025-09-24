# compiled manpages
{
  writeShellScript,
  runCommand,
  pandoc,
  findutils,
  installShellFiles,
  outputFormat ? "man"
}:
let
  outputFileString = if outputFormat == "man" then
      "$destdir/$(basename $source .md).$section"
    else
      "$destdir/$(basename $source)";
  compileManPageBin = writeShellScript "compile" ''
    source="$1"
    shift
    destdir="$1"
    shift
    section="$1"
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
      --to ${outputFormat}                     \
       $source                                 \
    > "${outputFileString}"
  '';
in
runCommand "flox-manpages"
  {
    src = builtins.path {
      name = "flox-manpage-src";
      path = "${./../../cli/flox/doc}";
    };
    buildInputs = [
      findutils
      installShellFiles
    ];
  }
  ''
    buildDir=$(pwd)/__build

    mkdir "$out"
    mkdir "$buildDir"
    pushd "$src"

    find . -name "*.md" -exec ${compileManPageBin} {} $buildDir 1 \;
    ${if outputFileString == "man" then
        "mv $buildDir/manifest.toml.1 $buildDir/manifest.toml.5"
      else ''
        # Remove the partials that were in the 'include' directory
        find . -wholename "*include/*.md" | while read -r included_file; do
          bname="$(basename "$included_file")"
          rm "$buildDir/$bname"
        done
      ''
    }

    ls $buildDir

    installManPage $buildDir/*;
  ''
