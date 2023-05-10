# Invoke with:
#   nix search --json "flake:floxpkgs#catalog.$system" "$packageregexp" | \
#       jq -r -f <this file>.jq | column --keep-empty-lines -t -s "|"

# Start by parsing and enhancing data into fields
to_entries | map(
  # Discard anything for which version = "latest".
  select(.key | endswith(".latest") | not) |
  (.key | split(".")) as $key |
  .value.version as $_version |
  .value.catalog = $key[0] |
  .value.system = $key[1] |
  .value.stability = $key[2] |
  .value.channel = $key[3] |
  .value.attrPath = ($key[4:] | join(".") | rtrimstr(".\($key[-1])")) |
  .value.alias = (
    if .value.stability == "stable" then (
      if .value.channel == "nixpkgs-flox" then [] else [.value.channel] end
    ) else
      [.value.stability,.value.channel]
    end + $key[4:] | join(".") | rtrimstr(".\($key[-1])")
  ) |
  .value
)
