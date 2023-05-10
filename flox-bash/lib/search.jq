# Invoke with:
#   nix search --json "flake:floxpkgs#catalog.$system" "$packageregexp" | \
#       jq -r --argjson showDetail (true|false) -f <this file>.jq | \
#       column --keep-empty-lines -t -s "|"

# Start by parsing and enhancing data into fields
with_entries(
  # Discard anything for which version = "latest".
  select(.key | endswith(".latest") | not) |
  (.key | split(".")) as $key |
  .value.version as $_version |
  .value.catalog = $key[0] |
  .value.system = $key[1] |
  .value.stability = $key[2] |
  .value.channel = $key[3] |
  .value.attrPath = ($key[4:] | join(".") | rtrimstr(".\($key[-1])")) |
  .value.floxref = "\(.value.channel).\(.value.attrPath)" |
  .value.alias = (
    if .value.stability == "stable" then (
      if .value.channel == "nixpkgs-flox" then [] else [.value.channel] end
    ) else
      [.value.stability,.value.channel]
    end + $key[4:] | join(".") | rtrimstr(".\($key[-1])")
  )
) |

# Then create arrays of result lines indexed under floxref.
reduce .[] as $x (
  {};
  "  " as $indent |
  "\($x.floxref)" as $f |
  (
    if $x.description == null or $x.description == ""
    then "\($x.alias)"
    else "\($x.alias) - \($x.description)"
    end
  ) as $header |
  "\($x.stability).\($x.floxref)@\($x.version)" as $line |
  # The first time seeing a floxref construct an array containing a
  # header as the previous value, otherwise use the previous array.
  ( if .[$f] then .[$f] else [$header] end ) as $prev |
  ( if $showDetail then ($prev + [($indent + $line)]) else $prev end ) as $result |
  . * { "\($f)": $result }
) |

# Sort by key.
to_entries | sort_by(.key) |
# Join floxref arrays by newline.
map(.value | join("\n")) |
# Our desire is to separate groupings of output with a newline but
# unfortunately the Linux version of `column` which supports the
# `--keep-empty-lines` option is not available on Darwin, so we
# instead place a line with "---" between groupings and then use
# `sed` to remove that on the flox.sh end.
( if $showDetail then "\n---\n" else "\n" end ) as $joinString |
join($joinString)
