#
# Merge search results from .../<channel>/stdout result files into
# single json stream with channel injected into floxpkgs tuple.
#
# Usage:
#   jq -r -f merge-search-results.jq <files> | jq -r -s add
#
include "catalog-search";
( input_filename | split("/")[-3] ) as $channel|
with_entries( nixPkgToCatalogPkg( $channel ) )
