#
# Simple nix wrapper to render a flox environment using buildenv.nix.
#
# A flox environment differs from the normal nix buildEnv in that it
# renders an extra tree of symbolic links to the ".develop" subdirectory
# containing the deep recursively-linked propagaged-user-env-packages
# of all packages contained within the environment.
#
# Usage:
#   buildenv \
#     [ -n <name> ] \
#     [ -a <activation-scripts-pkg> ] \
#     [ -m (nix|pkgdb) ] \
#     <path/to/manifest.lock>
#   -n <name> : The name of the flox environment to render.
#   -a <activation-scripts-pkg> : The store path of the activation scripts package.
#   -m (nix|pkgdb) : The method to use for realising packages. Defaults to "pkgdb".

set -eu

declare usage
usage="Usage: $0 [-x] \
  [-n <name>] \
  [-a <activation-scripts-pkg>] \
  [-m (nix|pkgdb)] \
  [-s <path/to/service-config.yaml>] \
  <path/to/manifest.lock>
-x : Enable debugging output.
-n <name> : The name of the flox environment to render.
-s <path/to/service-config.yaml> : Path to the service configuration file.
-m (nix|pkgdb) : The method to use for realising packages. Defaults to 'pkgdb'.
"

OPTSTRING="m:n:a:s:x"

declare buildMethod="${FLOX_BUILDENV_BUILD_METHOD:-pkgdb}"
declare name="${FLOX_BUILDENV_BUILD_NAME:-environment}"
declare serviceConfigYaml=""
declare -i debug=0
while getopts $OPTSTRING opt; do
  case $opt in
    m)
      buildMethod=$OPTARG
      ;;
    n)
      name=$OPTARG
      ;;
    s)
      serviceConfigYaml=$OPTARG
      ;;
    x)
      debug+=1
      ;;
    \?)
      echo "Invalid option: -$OPTARG" >&2
      exit 1
      ;;
    :)
      echo "Option -$OPTARG requires an argument." >&2
      exit 1
      ;;
  esac
done

shift $((OPTIND-1))

# Validate arguments.
if [ $# -ne 1 ]; then
  echo $usage >&2
  exit 1
fi
if [ "$buildMethod" != "nix" ] && [ "$buildMethod" != "pkgdb" ]; then
  echo $usage >&2
  exit 1
fi

# Binaries required for the script.
declare _jq="@jq@/bin/jq"
declare _nix="@nix@/bin/nix --extra-experimental-features flakes --extra-experimental-features nix-command"
declare _nix_store="@nix@/bin/nix-store"
declare _pkgdb="@floxPkgdb@/bin/pkgdb"
declare _xargs="@findutils@/bin/xargs"

# Nicer name for referring to the manifest.
declare manifest="$1"

# Function for realising packages using legacy pkgdb. Returns the "array"
# of [one] store path to be used in the derivation's inputSrcs.
function realisePkgdb {
  # Invoke `pkgdb realisepkgs` to realise all packages in the manifest.
  # Ignore the list of realised packages emitted to stdout.
  $_pkgdb realisepkgs "$manifest" > /dev/null
}

# Function for realising packages using flakes. Returns the array of store
# paths to be used in the derivation's inputSrcs. We don't use this at present
# because it is significantly slower than the legacy pkgdb method, but including
# it here for reference.
function realiseFlakes {
  # Build any packages required for the environment that are not already
  # present in the store. The build-packages.jq script will output a list
  # of tuples, where the first element is the store path of the package
  # the second element is the locked flakeref for building the package,
  # and the third and fourth elements are booleans indicating whether the
  # package is unfree or broken, respectively. We then filter out the store
  # paths that already exist in the store and build the rest.
  $_jq -r --arg system @system@ -f @out@/lib/build-packages.jq "$manifest" | (
    local -a tuple
    local -a flakerefs
    local impureArg=""
    while read -ra tuple; do
      if ! $_nix_store -r "${tuple[0]}" >/dev/null 2>&1; then
        flakerefs+=("${tuple[1]}")
        if [ "${tuple[2]}" = "true" ]; then
          export NIXPKGS_ALLOW_UNFREE=1
          impureArg="--impure"
        fi
        if [ "${tuple[3]}" = "true" ]; then
          export NIXPKGS_ALLOW_BROKEN=1
          impureArg="--impure"
        fi
      fi
    done
    # Actually kick off the nix build for any missing packages.
    # TODO: drop the --verbose flag below (?)
    echo "${flakerefs[@]}" | \
      $_xargs --verbose --no-run-if-empty $_nix build --no-link $impureArg
  )
}

# main()
#
# 1. Realise all packages in the manifest.
# 2. Build the flox environment.

# Enable debugging output if requested.
if [ $debug -gt 0 ]; then
  set -x
fi

# Realise all packages in the manifest using the selected method.
if [ "$buildMethod" = "nix" ]; then
  TIMEFORMAT='It took %R seconds to realise the packages with nix.'
  time {
    realiseFlakes
  }
else
  TIMEFORMAT='It took %R seconds to realise the packages with pkgdb.'
  time {
    realisePkgdb
  }
fi

# Render derivation for building the flox environment.
TIMEFORMAT='It took %R seconds to render the flox environment outputs as Nix packages.'
time {
  $_nix build -L --offline --no-link --json \
    --argstr manifest "$manifest" \
    --argstr name "$name" \
    --argstr serviceConfigYaml "$serviceConfigYaml" \
    --file @out@/lib/buildenv.nix '^*'
}
