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

set -eu

declare usage
usage="Usage: $0 [-x] \
  [-n <name>] \
  [-a <activation-scripts-pkg>] \
  [-s <path/to/service-config.yaml>] \
  <path/to/manifest.lock>
-x : Enable debugging output.
-n <name> : The name of the flox environment to render.
-a <path> : Path to the activation scripts package.
-s <path> : Path to the service configuration file.
"

OPTSTRING="m:n:a:s:x"

declare name="${FLOX_BUILDENV_BUILD_NAME:-environment}"
declare serviceConfigYaml=""
declare -i debug=0
while getopts $OPTSTRING opt; do
  case $opt in
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

# Binaries required for the script.
declare _nix="@nix@/bin/nix --extra-experimental-features flakes --extra-experimental-features nix-command"
declare _pkgdb="@floxPkgdb@/bin/pkgdb"

# Nicer name for referring to the manifest.
declare manifest="$1"

# Function for realising packages using legacy pkgdb. Returns the "array"
# of [one] store path to be used in the derivation's inputSrcs.
function realisePkgdb {
  # Invoke `pkgdb realise` to realise all packages in the manifest.
  # Ignore the list of realised packages emitted to stdout.
  $_pkgdb realise "$manifest" > /dev/null
}

# main()
#
# 1. Realise all packages in the manifest.
# 2. Build the flox environment.

# Enable debugging output if requested.
if [ $debug -gt 0 ]; then
  set -x
fi

# Realise all packages in the manifest using pkgdb.
TIMEFORMAT='It took %R seconds to realise the packages with pkgdb.'
time {
  realisePkgdb
}

# Render derivation for building the flox environment.
TIMEFORMAT='It took %R seconds to render the flox environment outputs as Nix packages.'
time {
  $_nix build -L --offline --no-link --json \
    --argstr manifest "$manifest" \
    --argstr name "$name" \
    --argstr serviceConfigYaml "$serviceConfigYaml" \
    --file @out@/lib/buildenv.nix '^*'
}
