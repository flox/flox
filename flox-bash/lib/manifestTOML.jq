#
# jq functions used by flox in the processing of manifest.json
#
# Usage:
#   cat manifest.toml | jq -e -r -s -f <this file> \
#     --args <function> <funcargs>
#

# Start by defining some constants.
$ARGS.positional[0] as $function
|
$ARGS.positional[1:] as $funcargs
|

.[] as $manifest |

# Helper method to validate number of arguments to function call.
def expectedArgs(count; args):
  (args | length) as $argc |
  if $argc < count then
    error("too few arguments \($argc) - was expecting \(count)")
  elif $argc > count then
    error("too many arguments \($argc) - was expecting \(count)")
  else . end;

#
# Functions for parsing manifest.toml.
#

def bashEnv:
  $manifest | if .environment then [
    if ($verbose == 1) then
      "echo \"flox: setting \($environmentOwner)/\($environmentName) environment variables\" 1>&2"
    else empty end,
    ( .environment | to_entries | map(
      if ($verbose == 1) then
        "echo \"+ export \(.key)=\\\"\(.value)\\\"\" 1>&2"
      else empty end,
      "export \(.key)=\"\(.value)\""
    ) | .[] )
  ] else empty end | .[];

def bashAliases:
  $manifest | if .aliases then [
    if ($verbose == 1) then
      "echo \"flox: setting \($environmentOwner)/\($environmentName) aliases\" 1>&2"
    else empty end,
    ( .aliases | to_entries | map(
      if ($verbose == 1) then
        "echo \"+ alias \(.key)=\\\"\(.value)\\\"\" 1>&2"
      else empty end,
      "alias \(.key)=\"\(.value)\""
    ) | .[] )
  ] else empty end | .[];

def bashHooks:
  $manifest | if .hooks then (
    .hooks | to_entries | map(
      if ($verbose == 1) then
        "echo \"flox: invoking \($environmentOwner)/\($environmentName) \\\"\(.key)\\\" hook\" 1>&2",
        "\(.key)_posthook=:",
        "if [[ $- =~ *x* ]]; then",
        "  \(.key)_posthook=:",
        "else",
        "  \(.key)_posthook=\"set +x\"",
        "  set -x",
        "fi",
        "\(.value)",
        "$\(.key)_posthook"
      else
        "\(.value)"
      end
    )
  ) else empty end | .[];

def bashInit(args): expectedArgs(0; args) |
  [ bashEnv, bashAliases, bashHooks ] | .[];

def installables(args): expectedArgs(0; args) |
  $manifest | if .packages then .packages else empty end | to_entries | map(
    if .value.storePaths then
      .value.storePaths[]
    elif .value.originalUrl then
      "\(.value.originalUrl)#\(.value.attrPath)"
    else (
      (if .value.channel then .value.channel else "nixpkgs-flox" end) as $channel |
      (if .value.stability then .value.stability else "stable" end) as $stability |
      "\($stability).\($channel).\(.key)"
    ) end
  ) | .[];

# For debugging.
def dump(args): expectedArgs(0; args) | $manifest;

#
# Call requested function with provided args.
# Think of this as this script's public API specification.
#
# XXX Convert to some better way using "jq -L"?
#
     if $function == "bashInit"     then bashInit($funcargs)
else if $function == "installables" then installables($funcargs)
else if $function == "dump"         then dump($funcargs)
else error("unknown function: \"\($function)\"")
end end end
