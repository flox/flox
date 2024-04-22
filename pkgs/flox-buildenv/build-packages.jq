#
# Quick jq script to kick off builds of any missing flake outputs prior
# to rendering a flox environment.
#
# Usage:
#   sh -c "$(jq -f <this file> --arg system <system> <path/to/manifest.lock>)"
#

# Sample manifest.lock:
# {
#   "lockfile-version": 1,
#   "packages": [
#     {
#       "attr_path": "curl",
#       "group": "toplevel",
#       "outputs": {
#         "bin": "/nix/store/1033k8sfipzk1ly7igmawdra7lg348wb-curl-8.7.1-bin",
#         "dev": "/nix/store/fwbjf1ikz055flksk2isiam4ajl0rpa5-curl-8.7.1-dev",
#         "devdoc": "/nix/store/yvgigsmfgxx4raiqi7qfn1aygf3fp5lj-curl-8.7.1-devdoc",
#         "man": "/nix/store/lqb2rgnwxqvqppgda0p9lnw02ddzwiyc-curl-8.7.1-man",
#         "out": "/nix/store/37ydms17yxwi5y5rck08c93jad1rmrn8-curl-8.7.1"
#       },
#       "outputs_to_install": [
#         "bin",
#         "man"
#       ],
#       "priority": 5,
#       "system": "aarch64-darwin",
#       ...
#     },
#     {
#       "attr_path": "xorg.xeyes",
#       "group": "toplevel",
#       "outputs": {
#         "out": "/nix/store/zl1d3gmhvpb1s6jdbqxmy3y1rflrr71v-xeyes-1.3.0"
#       },
#       "outputs_to_install": [
#         "out"
#       ],
#       "priority": 5,
#       "system": "x86_64-linux",
#       ...
#     }
#   ],
#   ...
# }

# Load the manifest from the file passed in the first argument.
. as $manifest
|

# Verify we're talking to the expected schema version.
if $manifest."lockfile-version" != 1 then
  "ERROR: unsupported manifest schema lockfile-version: " +
  ( $manifest."lockfile-version" | tostring )
  | halt_error(1)
else . end
|

# Verify we've been called with `--arg system <system>`.
if ($ARGS.named | has("system")) then . else
  "ERROR: missing '--arg system'\n" +
  "Usage: jq -f <this file> --arg system <system> <path/to/manifest.lock>\n"
  | halt_error(1)
end
|

# Verify we've been called with a valid system.
if (($system == "x86_64-linux") or ($system == "aarch64-linux") or
    ($system == "x86_64-darwin") or ($system == "aarch64-darwin")) then . else
  "ERROR: invalid '--arg system' argument\n" +
  "Valid systems: x86_64-linux, aarch64-linux, x86_64-darwin, aarch64-darwin\n"
  | halt_error(1)
end
|

# Generate a list of (storepath,flakeref) tuples representing all
# store paths used in this environment. This list is then consumed
# by the calling script to invoke Nix to build any missing packages.
# TODO: group nix invocations by flake URL and free/unfree status
#       to maximize the use of the flake cache. Also investigate
#       nix plugin to allow caching of unfree flake evaluations.

$manifest.packages | map(
  select(.system == $system) |
  .locked_url as $lockedUrl |
  .attr_path as $attrPath |
  (.unfree // false) as $unfree |
  (.meta.broken // false) as $broken |
  .outputs_to_install[] as $output |
  .outputs[$output] as $storePath |
  "\($storePath) 'git+\($lockedUrl)#\($attrPath)' \($unfree) \($broken)"
)[]
