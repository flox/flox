# ============================================================================ #
#
# Helper module containing several functions used by `flox search'.
#
# ---------------------------------------------------------------------------- #

# Processes results from `nix search <URL>#catalog.<SYSTEM>.<STABILITY> --json;'
# injecting the "channel" ( flake alias ) into the key, and adding info scraped
# from the `attrPath' as fields.
#
# NOTE: The format of these results differs from `nix search --json;'
# Ex:
# "catalog.x86_64-linux.stable.vimPlugins.vim-svelte.2022-02-1": {}
#   ->
# "catalog.x86_64-linux.stable.nixpkgs-flox.vimPlugins.vim-svelte.2022-02-1": {}
def nixPkgToCatalogPkg( $channel ):
  ( .key|split( "." ) ) as $key|
  $key[0] as $catalog|
  $key[1] as $system|
  $key[2] as $stability|
  ( $key[3:]|join( "." ) ) as $attrPathVersion|
  .key    = $catalog + "." + $system + "." + $channel + "." + $attrPathVersion|
  .value += {
    catalog:         $catalog
  , system:          $system
  , stability:       $stability
  , channel:         $channel
  , attrPathVersion: $attrPathVersion
  , attrPath:        $key[3:-1]|join( "." )
  };


# ---------------------------------------------------------------------------- #

# Convert the JSON representation of a catalog package emitted by `nix search'
# into an entry used by `flox search'.
def catalogPkgToSearchEntry:
  # Discard anything for which version = "latest".
  select( .key|endswith( ".latest" )|not )|.value|.+= {
    floxref: ( .channel + "." + .attrPath )
  };


# ---------------------------------------------------------------------------- #

# Convert a list of search entries to pretty results grouped by package name.
# This returns an attrset mapping "floxrefs" to pretty strings.
def searchEntriesToPrettyBlocks( $showDetail ):
  reduce .[] as $x (
    {};
    ( if $showDetail then $x.pname else (
        ( if $x.stability == "stable" then "" else $x.stability + "." end ) +
        ( if $x.channel == "nixpkgs-flox" then "" else $x.channel + "." end ) +
        $x.attrPath
      ) end
    ) as $alias|
    # Results are grouped under short headers which might have a description.
    ( if ( $x.description == null ) or ( $x.description == "" ) then $alias else
        $alias + ( if $showDetail then " - " else "|" end ) + $x.description
      end
    ) as $header|
    ( if $showDetail then $header else $x.floxref end ) as $key|
    # The first time seeing a floxref construct an array containing a
    # header as the previous value, otherwise use the previous array.
    ( if .[$key] then .[$key] else [$header] end ) as $prev|
    # Merge result with existing collection.
    # This potentially "updates" existing elements.
    . * {
      # Only include `$line' when `$showDetail' is enabled.
      "\($key)":
        # When `showDetails' is active, be show multiple lines under each header
        # as `<stability>.<channel>.<attrPath>@<version>'.
        ( if ( $showDetail|not ) then $prev else
            ( $prev + [( "  " + $x.floxref + "@" + $x.version )] )
          end )
    }
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
