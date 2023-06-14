# Diff catalog.json files to compute the set of removals, additions and upgrades.
#
# Invoke with:
#   jq -n -f lib/diff-catalogs.jq \
#     --slurpfile c1 path/to/catalog1.json \
#     --slurpfile c2 path/to/catalog2.json

# Call it a perl hangover, but refer to catalogs as $a and $b.
$c1[0] as $a |
$c2[0] as $b |

# Identify package paths in catalog by recursing through structure and
# popping the traversed path whenever encountering an attibute:
#
# - at least 3 deep (to account for channel and system), and
# - containing the package attributes build, element, eval and version

# Example flake element:
# {
#   "nixpkgs": {          # channel
#     "x86_64-linux": {   # system
#       "xorg": {         # pname (part 1 of n)
#         "xeyes": {      # pname (part 2 of n, can be more)
#           "build": [ ... ],
#           "element": [ ... ],
#           "eval": [ ... ],
#           "version": 1
#         }
#       }
#     }
#   }
# }
#
def isPackage:
  (has("type") and .["type"] == "catalogRender") or (has("element") and has("eval") and has("version"));

# Example catalog element:
# {
#   "nixpkgs-flox": {     # channel
#     "aarch64-darwin": { # system
#       "stable": {       # stability
#         "xorg": {       # pname (part 1 of n)
#           "xeyes": {    # pname (part 2 of n, can be more)
#             "latest": { # catalogVersion
#               "build": [ ... ],
#               "cache": [ ... ],
#               "element": { ... },
#               "eval": { ... },
#               "publish_element": { ... },
#               "source": { ... },
#               "version": 1
#             }
#           }
#         }
#       }
#     }
#   }
# }
def isCatalogPackage:
  isPackage and has("publish_element");

def packagePNames(keys):
  # Recurse, adding to keys as we go.
  to_entries | map(
    (keys + [.key]) as $newkeys |
    if ((.value | type) != "object") then (
      # We got to a leaf before finding a package, so there was never a valid catalog entry.
      "Internal error: invalid catalog data, encountered object of type \(type)" | halt_error(1)
    )
    elif (.value | isCatalogPackage) then
      # Don't return the final catalogVersion key (usually "latest").
      keys
    elif (.value | isFlakePackage) then
      # Return all keys including the last one.
      $newkeys
    else (
      .value | packagePNames($newkeys)[]
    ) end
  );

def packagePaths(catalog):
  catalog | to_entries | map(
    .key as $channel |
    .value | to_entries | map(
      .key as $system |
      .value | packagePNames([]) | map(
        [ $channel, $system ] + . | flatten
      )
    )
  ) | flatten(2);

# Identify package paths in each catalog.
packagePaths($a) as $a_paths |
packagePaths($b) as $b_paths |

# Walk through $a identifying items common to $a and $b.
( $a_paths | map(
  . as $packagePath |
  if (
    ($a | getpath($packagePath)) == ($b | getpath($packagePath))
  ) then $packagePath else empty end
) ) as $commonPaths |

# Prune identical packages found in $a and $b.
( $a | delpaths($commonPaths) ) as $unique_a |
( $b | delpaths($commonPaths) ) as $unique_b |

# Identify package paths in $unique_a and $unique_b.
packagePaths($unique_a) as $unique_a_paths |
packagePaths($unique_b) as $unique_b_paths |

# Walk unique package paths in $b and report as additions anything
# not found in $a.
(
  $unique_b_paths | map(
    . as $packagePath |
    ( $unique_a | getpath($packagePath) ) as $package_a |
    ( $unique_b | getpath($packagePath) ) as $package_b |
    if ($package_a == null) then [$packagePath, $package_b] else empty end
  )
) | flatten(1) as $additions |

# Walk unique package paths in $b and report as upgrades anything
# found in $a.
(
  $unique_b_paths | map(
    . as $packagePath |
    ( $unique_a | getpath($packagePath) ) as $package_a |
    ( $unique_b | getpath($packagePath) ) as $package_b |
    if ($package_a != null) then [$packagePath, $package_b] else empty end
  )
) | flatten(1) as $upgrades |

# Walk unique package paths in $a and report as deletions anything
# not found in $b.
(
  $unique_a_paths | map(
    . as $packagePath |
    ( $unique_a | getpath($packagePath) ) as $package_a |
    ( $unique_b | getpath($packagePath) ) as $package_b |
    if ($package_b == null) then [$packagePath, $package_a] else empty end
  )
) | flatten(1) as $removals |

# Tuples data presented as array of:
#    [ packagePath1, package1, packagePath2, package2, ... ]
# Recursively consume data in pairs until exhausted.
def mapTuples(data):
  if ((data | length) == 0) then [] else (
    data[0] as $packagePath |
    data[1] as $package |
    # Join "pname" attrPaths with "." to maintain a constant depth structure
    # for use with WebUI display elements, command construction, etc.
    ($packagePath[0:3] + [$packagePath[3:] | join(".")]) as $squashedPackagePath |
    [ {} | setpath($squashedPackagePath; $package) ] + mapTuples(data[2:])
  ) end;

# Finally, combine the upgrades, additions and removals. Do this explicitly
# rather than using "*" so that we always return the (add|remove|upgrade) keys.
{
  "add": mapTuples($additions),
  "remove": mapTuples($removals),
  "upgrade": mapTuples($upgrades)
}
