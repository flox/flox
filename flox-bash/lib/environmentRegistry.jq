# FIXME FIXME FIXME
#
# This is a copy of lib/registry.jq. It should instead be
# a module that first includes that file and then adds the
# environment-specific functions.
#
# FIXME FIXME FIXME
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
def get(args):
  $registry | getpath(args) // empty;

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
  $registry | .version;

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
  $registry | .generations | to_entries |
    map(listGeneration) | flatten | .[];

#
# Functions which generate script snippets.
#

# The process of generating an environment package is straightforward
# but requires that all storePaths referenced by the manifest are
# present on the system before invoking `nix profile build`. Take
# this opportunity to verify all the paths are present by invoking
# the `nix build` or `nix-store -r` commands that can create them.
def _syncGeneration(args):
  args[0] as $currentGen |
  args[1] as $ageDays |
  .key as $generation |
  .value.path as $path |
  .value.created as $created |
  .value.lastActive as $lastActive |
  # v1 floxEnvs do not contain a version
  (if .value.version then .value.version else 1 end) as $version |
  (($now-$lastActive)/(24*60*60)) as $daysSinceActive |
  "\($environmentParentDir)/\($environmentName)-\($generation)-link" as $targetLink |
  # Cannot embed newlines so best we can do is return array and flatten later.
  if .value.path != null then (
    # Don't bother building an old generation *unless* it's the current one.
    if (($currentGen == $generation) or ($daysSinceActive <= $ageDays)) then [
      # Don't rebuild links/environments for generations that already exist.
      "if [ -L \($targetLink) -a -d \($targetLink)/. ]; then " +
        ": verified existence of \($targetLink); " +
      "else " +
      # Do not spend time building/linking anything but the current generation.
      ( if ($currentGen == $generation) then
        if ($version == 1) then
          # Temporary XXX: Identify schema version in use, <=006 or >=007
          "environmentManifestFile=$( [ -e \($environmentMetaDir)/\($generation).json ] && echo \($environmentMetaDir)/\($generation).json || echo \($environmentMetaDir)/\($generation)/manifest.json ) && " +
          # Ensure all flakes referenced in environment are built.
          "manifest $environmentManifestFile listFlakesInEnvironment | " +
          " $_xargs --no-run-if-empty $( [ $verbose -eq 0 ] || echo '--verbose' ) -- $_nix build --impure --no-link && " +
          # Ensure all anonymous store paths referenced in environment are copied.
          "manifest $environmentManifestFile listStorePaths | " +
          " $_xargs --no-run-if-empty -n 1 -- $_sh -c '[ -d $0 ] || echo $0' | " +
          " $_xargs --no-run-if-empty --verbose -- $_nix_store -r && " +
          # Now we can attempt to build the environment and store in the bash $environmentPath variable.
          "environmentPath=$($_nix profile build $environmentManifestFile) && "
        else
          "environmentPath=$($invoke_nix build --impure --no-link --print-out-paths \($environmentMetaDir)/\($generation)#.floxEnvs.\($environmentSystem).default) && "
        end +
        # Now create the generation link using nix-store so that it creates a
        # GC root in the process. N.B. this command will silently overwrite a
        # symlink in situ.
        "$_nix_store --add-root \($targetLink) -r $environmentPath >/dev/null && " +
        # And set the symbolic link's date.
        "$_touch -h --date=@\($created) \($targetLink); "
      else
        ": not rendering non-current generation \($generation) at \($targetLink); "
      end ) +
      "fi"
    ] else [
      # Remove old generation symlinks to allow package GC.
      "if [ -L \($targetLink) ]; then " +
        "$_rm -f \($targetLink); " +
      "fi"
    ] end
  ) else [] end;

def syncGenerations(args):
  ( $registry | .currentGen ) as $currentGen |
  ( $registry | (if has("ageDays") then .ageDays else 90 end) ) as $ageDays |
  ( $registry | .generations | to_entries ) | map(_syncGeneration([$currentGen, $ageDays])) + [
    # Set the current generation symlink. Let its timestamp be now.
    "$_rm -f \($environmentParentDir)/\($environmentName)",
    "$_ln --force -s \($environmentName)-\($currentGen)-link \($environmentParentDir)/\($environmentName)"
  ] | flatten | .[];

# JSON does not permit integer keys so the generation keys are strings.
def curGeneration(args):
  $registry.currentGen | tonumber;

# JSON does not permit integer keys so the generation keys are strings.
# To find the max generation we must therefore convert to number first.
def nextGeneration(args):
  ( $registry.generations | keys | map(tonumber) | max ) as $maxGen |
  ( $maxGen + 1);

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
else if $function == "listGenerations" then listGenerations($funcargs)
else if $function == "syncGenerations" then syncGenerations($funcargs)
else if $function == "curGeneration"   then curGeneration($funcargs)
else if $function == "nextGeneration"  then nextGeneration($funcargs)
else error("unknown function: \"\($function)\"")
end end end end end end end end end end end end end end end end end
