# Pattern: importing a directory. Nix resolves `import ./import-dir` to
# `./import-dir/default.nix`; the scanner must do the same.
{ catalogs }:
import ./import-dir { inherit catalogs; }
