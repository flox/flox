#
# Merge search results from .../<channel>/stdout result files into
# single json stream with channel injected into floxpkgs tuple.
#
# Usage:
#   jq -r -f merge-search-results.jq <files> | jq -r -s add
#
( input_filename | split("/")[-3] ) as $channel
|
with_entries (
  ( .key | split(".") ) as $key |
  $key[0] as $catalog |
  $key[1] as $system |
  $key[2] as $stability |
  ( $key[3:] | join(".") ) as $attrPathVersion |
  .key = "\($catalog).\($system).\($stability).\($channel).\($attrPathVersion)"
)
