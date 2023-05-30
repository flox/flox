# Invoke with:
#   nix search --json "flake:floxpkgs#catalog.$system" "$packageregexp" | \
#       jq -r --argjson showDetail (true|false) -f <this file>.jq | \
#       column --keep-empty-lines -t -s "|"
include "catalog-search";
# Convert `nix search' results into extended `flox search' entries.
to_entries|map( nixCatalogPkgToSearchEntry )|
# Then create arrays of result lines indexed under floxref.
searchEntriesToPretty( $showDetail )
