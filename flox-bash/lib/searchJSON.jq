# Invoke with:
#   nix search --json "flake:floxpkgs#catalog.$system" "$packageregexp" | \
#       jq -r -f <this file>.jq | column --keep-empty-lines -t -s "|"
include "catalog-search";
# Convert `nix search' results into extended `flox search' entries.
to_entries|map( catalogPkgToSearchEntry|del( .floxref ) )
