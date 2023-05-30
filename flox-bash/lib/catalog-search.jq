# ============================================================================ #
#
# Helper module containing several functions used by `flox search'.
#
# ---------------------------------------------------------------------------- #

# Convert the JSON representation of a catalog package emitted by `nix search'
# into an entry used by `flox search'.
def nixCatalogPkgToSearchEntry:
  # Discard anything for which version = "latest".
  select( .key|endswith( ".latest" )|not )|
  # Start by parsing and enhancing data into fields
  ( .key|split( "." ) ) as $key|
  .value.version as $_version|
  .value.catalog   = $key[0]|
  .value.system    = $key[1]|
  .value.stability = $key[2]|
  .value.channel   = $key[3]|
  .value.attrPath  = ($key[4:]|join( "." )|rtrimstr( ".\($key[-1])" ) )|
  .value.floxref   = "\(.value.channel).\(.value.attrPath)"|
  .value.alias     = (
    ( if .value.stability == "stable" then (
        if .value.channel == "nixpkgs-flox" then [] else [.value.channel] end
      ) else [.value.stability,.value.channel]
      end ) + $key[4:]|join( "." )|rtrimstr( ".\($key[-1])" )
  )|.value;


# ---------------------------------------------------------------------------- #

# Convert a list of search entries to pretty results grouped by package name.
# This returns an attrset mapping "floxrefs" to pretty strings.
def searchEntriesToPrettyBlocks( $showDetail ):
  reduce .[] as $x (
    {};
    # Results are grouped under short headers which might have a description.
    ( $x.alias + (
        if ( $x.description == null ) or ( $x.description == "" )
          then ""
          else " - " + $x.description
        end
      )
    ) as $header|
    # When `showDetails' is active, be show multiple lines under each header
    # as `<stability>.<channel>.<attrPath>@<version>'.
    ( $x.stability + "." + $x.floxref + "@" + $x.version ) as $line|
    # The first time seeing a floxref construct an array containing a
    # header as the previous value, otherwise use the previous array.
    ( if .[$x.floxref] then .[$x.floxref] else [$header] end ) as $prev|
    # Only include `$line' when `$showDetail' is enabled.
    ( if $showDetail then ( $prev + [( "  " + $line )] ) else $prev end
    ) as $result|
    # Merge result with existing collection.
    # This potentially "updates" existing elements.
    . * { "\($x.floxref)": $result }
  );


# ---------------------------------------------------------------------------- #

# Convert a list of search entries to pretty results by package name.
# This returns a single string ready for printing.
def searchEntriesToPretty( $showDetail ):
  searchEntriesToPrettyBlocks( $showDetail )|
  # Sort by key.
  to_entries|sort_by( .key )|
  # Join floxref arrays by newline.
  map( .value|join( "\n" ) )|
  # Our desire is to separate groupings of output with a newline but
  # unfortunately the Linux version of `column' which supports the
  # `--keep-empty-lines' option is not available on Darwin, so we
  # instead place a line with "---" between groupings and then use
  # `sed' to remove that on the flox.sh end.
  join( if $showDetail then "\n---\n" else "\n" end );


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
