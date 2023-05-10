#
# jq functions used by flox in the processing of manifest.json
#
# Usage:
#   jq -e -n -r -s -f <this file> \
#     --slurpfile manifest <path/to/manifest.json>
#     --args <function> <funcargs>
#

# Start by defining some constants.
$ARGS.positional[0] as $function
|
$ARGS.positional[1:] as $funcargs
|

# Verify we're talking to the expected schema version.
if $manifest[].version != 1 and $manifest[].version != 2 then
  error(
    "unsupported manifest schema version: " +
    ( $manifest[].version | tostring )
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
# Functions which convert between flakeref and floxpkg tuple elements.
#
# floxpkg: <stability>.<channel>.<pkgname> (fully-qualified)
# flake:<channel>#evalCatalog.<system>.<stability>.<pkgname>
#
# Sample element:
# {
#   "active": true,
#   "attrPath": "evalCatalog.$system.stable.vim",
#   "originalUrl": "flake:nixpkgs-flox",
#   "storePaths": [
#     "/nix/store/ivwgm9bdsvhnx8y7ac169cx2z82rwcla-vim-8.2.4350"
#   ],
#   "url": "github:flox/nixpkgs-flox/ef23087ad88d59f0c0bc0f05de65577009c0c676",
#   "position": 3
# }
#
#

def floxpkgToFlakeref(args): expectedArgs(1; args) |
  args[0] as $floxpkg |
  ( $floxpkg | split(".") ) as $floxpkgArray |
  $floxpkgArray[0] as $stability |
  $floxpkgArray[1] as $channel |
  ( $floxpkgArray[2:] | join(".") ) as $attrPath |
  "flake:\($channel)#evalCatalog.\($system).\($stability).\($attrPath)";

def flakerefToFloxpkg(args): expectedArgs(1; args) |
  args[0] as $flakeref |
  ( $flakeref | split("#") | .[0] ) as $flakeOriginalUrl |
  ( $flakeref | split("#") | .[1] ) as $flakeAttrPath |
  ( $flakeAttrPath | split(".") ) as $flakeAttrPathArray |
  ( $flakeOriginalUrl | ltrimstr("flake:") ) as $channel |
  if ($channel == "floxpkgs") then
    # legacy "one flake" access to catalog retired 9/17/22.
    if ($flakeAttrPath | startswith("legacyPackages.\($system).catalog.")) then (
      # legacyPackages.<system>.catalog.<channel>.<stability>.<name> format retired 6/30/22
      $flakeAttrPathArray[3] as $channel |
      $flakeAttrPathArray[4] as $stability |
      ( $flakeAttrPathArray[5:] | join(".") ) as $attrPath |
      "\($stability).\($channel).\($attrPath)"
    ) elif ($flakeAttrPath | startswith("legacyPackages.\($system).")) then (
      # legacyPackages.<system>.<stability>.<channel>.<name> format retired 8/30/22
      $flakeAttrPathArray[2] as $stability |
      $flakeAttrPathArray[3] as $channel |
      ( $flakeAttrPathArray[4:] | join(".") ) as $attrPath |
      "\($stability).\($channel).\($attrPath)"
    ) elif ($flakeAttrPath | startswith("evalCatalog.\($system).")) then (
      # evalCatalog.<system>.<stability>.<channel>.<name> format retired 9/17/22
      $flakeAttrPathArray[2] as $stability |
      $flakeAttrPathArray[3] as $channel |
      ( $flakeAttrPathArray[4:] | join(".") ) as $attrPath |
      "\($stability).\($channel).\($attrPath)"
    ) else $flakeref end
  else
    # Current format starting 9/17/22: flake:<channel>#evalCatalog.<system>.<stability>.<name>
    $flakeAttrPathArray[0] as $flakeType |
    if ($flakeType == "evalCatalog") then (
      $flakeAttrPathArray[2] as $stability |
      ( $flakeAttrPathArray[3:] | join(".") ) as $attrPath |
      "\($stability).\($channel).\($attrPath)"
    ) elif ($flakeType == "legacyPackages") then (
      ( $flakeAttrPathArray[2:] | join(".") ) as $attrPath |
      "\($channel)#\($attrPath)"
    ) else $flakeref end
  end;

def elementToFloxpkg:
  . as $element |
  ( $element["storePaths"] // [] ) as $storePaths |
  ( $element["attrPath"] | split(".") ) as $attrPathArray |
  ( $element["originalUrl"] | ltrimstr("flake:") ) as $channel |
  # Current format starting 9/17/22: flake:<channel>#evalCatalog.<system>.<stability>.<name>
  $attrPathArray[0] as $flakeType |
  if ($flakeType == "evalCatalog") then (
    $attrPathArray[2] as $stability |
    $attrPathArray[-1] as $maybeVersion |
    ($maybeVersion | split("_") | join(".")) as $maybeVersionWithDots |
    # If any of the storePaths end with the dotted version then assume
    # that is in fact a version string.
    if (($storePaths | map(select(endswith("-\($maybeVersionWithDots)"))) | length) > 0) then (
      ( $attrPathArray[3:-1] | join(".") ) as $attrPath |
      "\($stability).\($channel).\($attrPath)@\($maybeVersionWithDots)"
    ) else (
      ( $attrPathArray[3:] | join(".") ) as $attrPath |
      "\($stability).\($channel).\($attrPath)"
    ) end
  ) elif ($flakeType == "legacyPackages") then (
    ( $attrPathArray[2:] | join(".") ) as $attrPath |
    "\($channel)#\($attrPath)"
  ) else (
    # Punt? Just print the flake reference.
    [ $element["originalUrl"], $element["attrPath"] ] | join("#")
  ) end;

def flakerefToCatalogPath(args): expectedArgs(1; args) |
  args[0] as $flakeref |
  ( $flakeref | split("#") | .[0] ) as $flakeOriginalUrl |
  ( $flakeref | split("#") | .[1] ) as $flakeAttrPath |
  ( $flakeAttrPath | split(".") ) as $flakeAttrPathArray |
  ( $flakeOriginalUrl | ltrimstr("flake:") ) as $channel |
  # Current format starting 9/17/22: flake:<channel>#evalCatalog.<system>.<stability>.<name>
  $flakeAttrPathArray[0] as $flakeType |
  if ($flakeType == "evalCatalog") then (
    $flakeAttrPathArray[2] as $stability |
    ( $flakeAttrPathArray[3:] | join(".") ) as $attrPath |
    ".[\"\($channel)\"][\"\($system)\"][\"\($stability)\"][\"\($attrPath)\"]"
  ) else empty end;

# Pull pname attribute from flakeref (for sorting).
def flakerefToPname(args): expectedArgs(1; args) |
  flakerefToFloxpkg(args) |
  split(".") | .[2:] | join(".");

# Add position and package{Name,PName,Version} as we define $elements.
( $manifest[].elements | to_entries | map(
  ( .value.storePaths[0] | .[44:] ) as $packageName |
  ( if ( .value | has("attrPath") ) then (
    ( .value | elementToFloxpkg | split(".")[2:] | join(".") | split("@")[0] ) as $packagePName |
    ( $packageName | ltrimstr("\($packagePName)-") ) as $packageVersion |
    [ $packagePName, $packageVersion ]
  ) else (
    # When installing by store path we don't have the provenance to
    # precisely know what part of the "name" is the "pname" as opposed
    # to the "version", so don't guess and instead present the beginning
    # characters of the path checksum as the version.
    ( .value.storePaths[0] | .[11:19] ) as $packageVersion |
    [ $packageName, $packageVersion ]
  ) end ) as $packagePNameVersion |
  .value * {
    position:.key,
    packageName:$packageName,
    packagePName:$packagePNameVersion[0],
    packageVersion:$packagePNameVersion[1],
    packageIdentifier: (
      if .value.attrPath then
        flakerefToPname(["\(.value.originalUrl)#\(.value.attrPath)"])
      else
        $packageName
      end
    )
  }
) ) as $elements
|

def evalCatalogFlakerefToTOML(arg):
  flakerefToFloxpkg([arg]) | split(".") |
  .[0] as $stability |
  .[1] as $channel |
  (.[2:] | join(".")) as $nameAttrPath |
  "  [packages.\"\($nameAttrPath)\"]
  channel = \"\($channel)\"
  stability = \"\($stability)\"
";

def legacyPackagesFlakerefToTOML(arg):
  arg | split("#") |
  .[0] as $originalUrl |
  .[1] as $attrPath |
  "  [packages.\"\($attrPath)\"]
  originalUrl = \"\($originalUrl)\"
  attrPath = \"\($attrPath)\"
";

def flakerefToTOML(arg):
  if (arg | contains("#evalCatalog.\($system).")) then
    evalCatalogFlakerefToTOML(arg)
  else
    legacyPackagesFlakerefToTOML(arg)
  end;

def storePathsToTOML(storePaths):
  ( "\"" + ( storePaths | join("\",\n      \"") ) + "\"" ) as $storePaths |
  ( storePaths[0] | .[44:] ) as $pkgname |
  "  [packages.\"\($pkgname)\"]
  storePaths = [
    \($storePaths)
  ]
";

def floxpkgToAttrPath(args): expectedArgs(1; args) |
  ["evalCatalog", $system, args[0]] | join(".");

def floxpkgFromElementV1:
  if .attrPath then
    flakerefToFloxpkg(["\(.originalUri)#\(.attrPath)"])
  else
    .storePaths[]
  end;
def floxpkgFromElementV2:
  if .attrPath then
    elementToFloxpkg
  else
    .storePaths[]
  end;
def floxpkgFromElement:
  if $manifest[].version == 2 then
    floxpkgFromElementV2
  else
    floxpkgFromElementV1
  end;

def catalogPathFromElement:
  if .attrPath then
    flakerefToCatalogPath(["\(.originalUrl)#\(.attrPath)"])
  else
    .storePaths[]
  end;

def TOMLFromElement:
  if .attrPath then
    flakerefToTOML("\(.originalUrl)#\(.attrPath)")
  else
    storePathsToTOML(.storePaths)
  end;

def flakerefFromElementV1:
  "\(.originalUri)#\(.attrPath)";
def flakerefFromElementV2:
  "\(.originalUrl)#\(.attrPath)";
def flakerefFromElement:
  if $manifest[].version == 2 then
    flakerefFromElementV2(args)
  else
    flakerefFromElementV1(args)
  end;

def lockedFlakerefFromElementV1:
  "\(.uri)#\(.attrPath)";
def lockedFlakerefFromElementV2:
  "\(.url)#\(.attrPath)";
def lockedFlakerefFromElement:
  if $manifest[].version == 2 then
    lockedFlakerefFromElementV2
  else
    lockedFlakerefFromElementV1
  end;

#
# Functions to look up element and return data in requested format.
#
def flakerefToElementV1(args): expectedArgs(2; args) |
  $elements | map(select(
    (.originalUri == args[0]) and (.attrPath == args[1])
  )) | .[0];
def flakerefToElementV2(args): expectedArgs(2; args) |
  # Look for exact match.
  $elements | map(select(
    (.originalUrl == args[0]) and (.attrPath == args[1])
  )) | .[0] as $fullMatch |
  # Look for partial match of the attrPath part,
  # e.g. "legacyPackages.x86_64-linux.hello".
  $elements | map(select(
    has("attrPath") and (
      .attrPath as $attrPath |
      args[1] | endswith($attrPath)
    )
  )) | .[0] as $partialMatch |
  # Look to see if user provided some string that exactly matches
  # the final part of the flake attrPath, e.g. "hello".
  $elements | map(select(
    (.originalUrl == "flake:\(args[0])") and
    (.attrPath | endswith(args[1]))
  )) | .[0] as $weakestMatch |
  # Prefer full match over partial over weakest match if any exist.
  ($fullMatch // $partialMatch // $weakestMatch);
def flakerefToElement(args): expectedArgs(1; args) |
  ( args[0] | split("#") ) as $_args |
  if $manifest[].version == 2 then
    flakerefToElementV2($_args)
  else
    flakerefToElementV1($_args)
  end;

def flakerefToPosition(args): expectedArgs(1; args) |
  flakerefToElement(args) | .position;

def floxpkgToPosition(args): expectedArgs(1; args) |
  floxpkgToFlakeref([ args[0] ]) as $flakeref |
  flakerefToPosition([ $flakeref ]);

def storepathToElement(args): expectedArgs(1; args) |
  $elements | map(select(.storePaths | contains([args[0]]))) | .[0];

def storepathToPosition(args): expectedArgs(1; args) |
  storepathToElement(args) | .position;

def positionToFloxpkg(args): expectedArgs(1; args) |
  $elements[args[0] | tonumber] | floxpkgFromElement;

def positionToCatalogPath(args): expectedArgs(1; args) |
  $elements[args[0] | tonumber] | catalogPathFromElement;

#
# Functions which present output directly to users.
#
def listEnvironment(args):
  (args | length) as $argc |
  if $argc == 0 then
    $elements | map(
      . as $datum |
      ($datum | floxpkgFromElement) as $floxpkgArg |
      ($datum | .packageVersion) as $floxpkgVersion |
      ($datum | .position) as $position |
      "\($position | tostring) \($floxpkgArg) \($floxpkgVersion)"
    ) | join("\n")
  elif $argc == 2 then
    error("excess argument: " + args[1])
  elif $argc > 1 then
    error("excess arguments: " + (args[1:] | join(" ")))
  elif args[0] == "--out-path" then
    $elements | map(
      . as $datum |
      ($datum | floxpkgFromElement) as $floxpkgArg |
      ($datum | .storePaths | join(",")) as $storePaths |
      ($datum | .position) as $position |
      "\($position | tostring) \($floxpkgArg) \($storePaths)"
    ) | join("\n")
  elif args[0] == "--json" then (
    $elements | sort_by(.packageIdentifier) | unique_by(.packageIdentifier) | map(
      . as $datum |
      ($datum | floxpkgFromElement) as $floxpkgArg |
      {"floxpkgArg": $floxpkgArg} * $datum
    ) as $elementData |
    {
      "elements": $elementData,
      "version": $manifest[].version
    }
  ) else
    error("unknown option: " + args[0])
  end;

def listEnvironmentTOML(args): expectedArgs(0; args) |
  $elements | sort_by(.packageIdentifier) | unique_by(.packageIdentifier) |
    map(TOMLFromElement) as $TOMLelements |
  (["[packages]"] + $TOMLelements) | join("\n");

def listFlakesInEnvironment(args): expectedArgs(0; args) |
  ( $elements | map(
    if .attrPath then lockedFlakerefFromElement else empty end
  ) ) as $flakesInEnvironment |
  if ($flakesInEnvironment | length) == 0 then " " else ($flakesInEnvironment | .[]) end;

def listStorePaths(args): expectedArgs(0; args) |
  ( $elements | map(.storePaths) | flatten ) as $anonStorePaths |
  if ($anonStorePaths | length) == 0 then " " else ($anonStorePaths | .[]) end;

# return an attrPath and any other args to pass for installation. Only the attrPath is used for removals
def flakerefToNixEditorArgs(args): expectedArgs(1; args) |
  args[0] as $flakeref |
  ( $flakeref | split("#") | .[0] ) as $flakeOriginalUrl |
  ( $flakeref | split("#") | .[1] ) as $flakeAttrPathWithVersion |
  ( $flakeAttrPathWithVersion | split("@") | .[0] ) as $flakeAttrPath |
  ( $flakeAttrPathWithVersion | split("@") | .[1] ) as $flakeAttrPathVersion |
  ( $flakeAttrPath | split(".") ) as $flakeAttrPathArray |
  ( $flakeOriginalUrl | ltrimstr("flake:") ) as $rawChannel |
  ( "\"" + $rawChannel + "\"" ) as $quotedChannel |
  ( if ($rawChannel | test(":|/|\\.")) then $quotedChannel else $rawChannel end ) as $channel |
  # Current format starting 9/17/22: flake:<channel>#evalCatalog.<system>.<stability>.<name>
  $flakeAttrPathArray[0] as $flakeType |
  if ($flakeType == "evalCatalog") then (
    $flakeAttrPathArray[2] as $stability |
    ( $flakeAttrPathArray[3:] | join(".") ) as $attrPath |
    [ ( if ($stability != "stable") then "stability=\"\($stability)\";" else empty end ),
      ( if $flakeAttrPathVersion then "version=\"\($flakeAttrPathVersion)\";" else empty end ) ]
      | join(" ") as $editorAttributes |
    ["packages.\($channel).\($attrPath)","-v","{\($editorAttributes)}"]
  ) elif ($flakeType == "legacyPackages") then (
    # Example flakeref:
    #   flake:nixpkgs#legacyPackages.x86_64-linux.hello
    # maps to: "packages.nixpkgs.hello","-v","{}"
    # Challenge is getting this right:
    #   flake:nixpkgs#legacyPackages.x86_64-linux.pythonPackages.foo.bar
    ($flakeAttrPathWithVersion | split(".") | .[2:] | join(".")) as $attrPath |
    ["packages.\($channel).\($attrPath)","-v","{}"]
  ) elif ($flakeType == "packages") then (
    # Some random flake:<channel>#<attrPath>
    ($flakeAttrPathWithVersion | split(".") | .[2:] | join(".")) as $attrPath |
    ["packages.\($channel).\($attrPath)","-v","{}"]
  ) else (
    ["packages.\($channel).\($flakeAttrPathWithVersion)","-v","{}"]
  ) end;

def _floxpkgToNixEditorArgs(args): expectedArgs(1; args) |
  if ( args[0] | startswith("/nix/store/") ) then (
    # Install by /nix/store path.
    ["packages.\"\(args[0])\"","-v","{}"]
  ) elif ( args[0] | startswith("flake:") ) then (
    flakerefToNixEditorArgs(args)
  ) elif ( args[0] | contains("#") ) then (
    flakerefToNixEditorArgs(args)
  ) else (
    floxpkgToFlakeref([ args[0] ]) as $flakeref |
    flakerefToNixEditorArgs([ $flakeref ])
  ) end;

# Convert array response from above to line-delimited output.
def floxpkgToNixEditorArgs(args): expectedArgs(1; args) |
  _floxpkgToNixEditorArgs(args)[];

def convert007to008(args):
  args[0] as $nixEditor |
  args[1] as $floxNix |
  (args | length) as $argc |
  $elements | map(
    floxpkgFromElement as $floxpkg |
    _floxpkgToNixEditorArgs([$floxpkg]) as $nixEditorArgs |
    "\($nixEditor) -i \($floxNix) '" + ($nixEditorArgs | join("' '")) + "'"
  ) | join("\n");

# For debugging.
def dump(args): expectedArgs(0; args) |
  $manifest | .[];

#
# Call requested function with provided args.
# Think of this as this script's public API specification.
#
# XXX Convert to some better way using "jq -L"?
#
     if $function == "floxpkgToFlakeref"       then floxpkgToFlakeref($funcargs)
else if $function == "flakerefToFloxpkg"       then flakerefToFloxpkg($funcargs)
else if $function == "floxpkgToPosition"       then floxpkgToPosition($funcargs)
else if $function == "flakerefToPosition"      then flakerefToPosition($funcargs)
else if $function == "storepathToPosition"     then storepathToPosition($funcargs)
else if $function == "positionToFloxpkg"       then positionToFloxpkg($funcargs)
else if $function == "listEnvironment"         then listEnvironment($funcargs)
else if $function == "listEnvironmentTOML"     then listEnvironmentTOML($funcargs)
else if $function == "convert007to008"         then convert007to008($funcargs)
else if $function == "listFlakesInEnvironment" then listFlakesInEnvironment($funcargs)
else if $function == "listStorePaths"          then listStorePaths($funcargs)
else if $function == "flakerefToNixEditorArgs" then flakerefToNixEditorArgs($funcargs)
else if $function == "floxpkgToNixEditorArgs"  then floxpkgToNixEditorArgs($funcargs)
else if $function == "positionToCatalogPath"   then positionToCatalogPath($funcargs)
else if $function == "dump"                    then dump($funcargs)
else error("unknown function: \"\($function)\"")
end end end end end end end end end end end end end end end
