#
# jq functions for managing the flox registry.
#
# Analogous to ~/.cache/nix/flake-registry.json, the flox registry
# contains configuration data managed imperatively by flox CLI
# subcommands.
#
# Usage:
#   jq -e -n -r -s -f <this file> \
#     --arg version <version> \
#     --slurpfile registry <path/to/registry.json> \
#     --args <function> <funcargs>
#
$ARGS.positional[0] as $function
|
$ARGS.positional[1:] as $funcargs
|
($registry | .[]) as $registry
|

# Verify we're talking to the expected schema version.
if $registry.version != ($version | tonumber) then
  error(
    "unsupported registry schema version: " +
    ( $registry.version | tostring )
  )
else . end
|

# Helper method to validate number of arguments to function call.
def expectedArgs(count; args):
  (args | length) as $argc |
  if $argc < count then
    error("too few arguments \($argc) - was expecting \(count)")
  elif $argc > count then
    error("too many arguments \($argc) - was expecting \(count)")
  else . end;

#
# Accessor methods.
#

# XXX temporary(?) hack: don't let "get" return an empty string.
def get(args):
  ($registry | getpath(args) // empty) as $retval |
  if $retval == "" then empty else $retval end;

def setNumber(args):
  $registry | setpath(args[0:-1]; (args[-1]|tonumber));

def setString(args):
  $registry | setpath(args[0:-1]; (args[-1]|tostring));

def set(args):
  setString(args);

def delete(args):
  $registry | delpaths([args]);

def addArrayNumber(args):
  $registry | setpath(
    args[0:-1];
    ( getpath(args[0:-1])? ) + [ (args[-1]|tonumber) ]
  );

def addArrayString(args):
  $registry | setpath(
    args[0:-1];
    ( getpath(args[0:-1])? ) + [ (args[-1]|tostring) ]
  );

def addArray(args):
  addArrayString(args);

def delArrayNumber(args):
  ( $registry | getpath(args[0:-1]) ) as $origarray |
  ( $origarray | index(args[-1]|tonumber) ) as $delindex |
  ( $origarray | del(.[$delindex]) ) as $newarray |
  $registry | setpath((args[0:-1]); $newarray);

def delArrayString(args):
  ( $registry | getpath(args[0:-1]) ) as $origarray |
  ( $origarray | index(args[-1]|tostring) ) as $delindex |
  ( $origarray | del(.[$delindex]) ) as $newarray |
  $registry | setpath((args[0:-1]); $newarray);

def delArray(args):
  delArrayString(args);

def dump(args): expectedArgs(0; args) |
  $registry;

def version(args): expectedArgs(0; args) |
  $registry.version;

# Environment-specific functions.

# JSON does not permit integer keys so the generation keys are strings.
def currentGen(args):
  if $registry.currentGen then (
    $registry.currentGen | tonumber
  ) else 0 end;

# JSON does not permit integer keys so the generation keys are strings.
# To find the max generation we must therefore convert to number first.
def nextGen(args):
  if $registry.generations then (
    ( $registry.generations | keys | map(tonumber) | max ) + 1
  ) else 1 end;

#
# Functions which present output directly to users.
#
def listGeneration:
  select(.value.created != null and .value.lastActive != null) |
  .key as $generation |
  (.value.created | todate) as $created |
  (.value.lastActive | todate) as $lastActive |
  # Cannot embed newlines so best we can do is return array and flatten later.
  [ "Generation \($generation):",
    "  Path:        \(.value.path)",
    "  Created:     \($created)",
    "  Last active: \($lastActive)" ] +
  if .value.logMessage != null then [
    "  Log entries:", (.value.logMessage | map("    \(.)"))
  ] else [] end;

def listGenerations(args):
  (args | length) as $argc |
  if $argc == 0 then
    $registry | .generations | to_entries |
      map(listGeneration) | flatten | .[]
  elif args[0] == "--json" then
    $registry | .generations
  else
    error("unknown arg/option: " + args[0])
  end;

#
# Call requested function with provided args.
# Think of this as this script's public API specification.
#
# XXX Convert to some better way using "jq -L"?
#
     if $function == "get"             then get($funcargs)
else if $function == "set"             then set($funcargs)
else if $function == "setNumber"       then setNumber($funcargs)
else if $function == "setString"       then setString($funcargs)
else if $function == "delete"          then delete($funcargs)
else if $function == "addArrayNumber"  then addArrayNumber($funcargs)
else if $function == "addArrayString"  then addArrayString($funcargs)
else if $function == "addArray"        then addArray($funcargs)
else if $function == "delArrayNumber"  then delArrayNumber($funcargs)
else if $function == "delArrayString"  then delArrayString($funcargs)
else if $function == "delArray"        then delArray($funcargs)
else if $function == "dump"            then dump($funcargs)
else if $function == "version"         then version($funcargs)
else if $function == "currentGen"      then currentGen($funcargs)
else if $function == "nextGen"         then nextGen($funcargs)
else if $function == "listGenerations" then listGenerations($funcargs)
else error("unknown function: \"\($function)\"")
end end end end end end end end end end end end end end end end
