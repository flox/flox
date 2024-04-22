#
# A translation script mapping old pkgdb calling semantics to the new
# buildenv. Eventually we will retire this script once we update the
# rust CLI to call into buildenv and the various nix tools directly.
#
set -eu

declare NAME="$0"

function buildenv() {
  local OPTIONS=x
  local LONGOPTS=container,service-config:
  local USAGE="Usage: $NAME buildenv [ --container ] [ --service-config <path> ]"
  local PARSED=$("@getopt@/bin/getopt" --options="$OPTIONS" --longoptions="$LONGOPTS" --name "$NAME" -- "$@")
  # shellcheck disable=SC2181
  if [[ $? -ne 0 ]]; then
    echo "ERROR: failed to parse options"
    exit 1
  fi
  # Use eval to remove quotes and replace them with spaces.
  eval set -- "$PARSED"
  # Set default values for options.
  local buildContainer=false
  local serviceConfigYamlPath=""
  while true; do
    case "$1" in
      --container)
        buildContainer=true
        echo "ERROR: option --container is not supported" >&2
        exit 1
        ;;
      --service-config)
        shift
        serviceConfigYamlPath="$1"
        shift
        ;;
      -x)
        shift
        set -x
        ;;
      --)
        shift
        break
        ;;
      -*)
        echo "ERROR: invalid option: $1" >&2
        echo "$USAGE" >&2
        exit 1
        ;;
    esac
  done
  # Upon success the old pkgdb returned:
  #   {"store_path":"/nix/store/gs83baxsn1bsg9rkdqrv18i0lhk75arf-environment"}
  # ... whereas we now return information about all storepaths rendered:
  #   [{"drvPath":"/nix/store/lv7c3qnzkbvmj5sg26qbsxbbwxqsh19g-floxenv.drv","outputs":{"develop":"/nix/store/zy6r86vp164qnll9n3l02yqn7qz92yhx-floxenv-develop","out":"/nix/store/f7z7lsh7r69shyfs2vlfgdknp7hz8k1g-floxenv"}}]
  #
  # For now, use jq to report the "develop" output path as the "store_path".
  set -o pipefail && @out@/bin/buildenv -s "$serviceConfigYamlPath" "$@" | @jq@/bin/jq -r -M -c '.[0] | {"store_path": .outputs.develop}'
  exit 0
}

function linkenv() {
  local OPTIONS=
  local LONGOPTS=out-link:,store-path:
  local USAGE="Usage: $NAME buildenv --out-link <path> --store-path <path>"
  local PARSED=$("@getopt@/bin/getopt" --options="$OPTIONS" --longoptions="$LONGOPTS" --name "$NAME" -- "$@")
  # shellcheck disable=SC2181
  if [[ $? -ne 0 ]]; then
    echo "ERROR: failed to parse options"
    exit 1
  fi
  # Use eval to remove quotes and replace them with spaces.
  eval set -- "$PARSED"
  # Set default values for options.
  local OUT_LINK=
  local STORE_PATH=
  while true; do
    case "$1" in
      --out-link)
        shift
        OUT_LINK="$1"
        shift
        ;;
      --store-path)
        shift
        STORE_PATH="$1"
        shift
        ;;
      --)
        shift
        break
        ;;
      -*)
        echo "ERROR: invalid option: $1" >&2
        echo "$USAGE" >&2
        exit 1
        ;;
    esac
  done
  if [ -z "$OUT_LINK" ]; then
    echo "ERROR: missing required option --out-link" >&2
    echo "$USAGE" >&2
    exit 1
  fi
  if [ -z "$STORE_PATH" ]; then
    echo "ERROR: missing required option --store-path" >&2
    echo "$USAGE" >&2
    exit 1
  fi
  # Create GC root for the store path at $OUT_LINK.
  # Old command using nix-store:
  # store_path="$(@nix@/bin/nix-store --add-root "$OUT_LINK" -r "$STORE_PATH")"
  # New command (hack?) using nix build:
  store_path="$(@nix@/bin/nix --extra-experimental-features nix-command \
    build --print-out-paths --out-link "$OUT_LINK" "$STORE_PATH")"
  if [ $? -ne 0 ]; then
    echo "ERROR: failed to link store path" >&2
    exit 1
  fi
  # We're also expected to return the store path as a JSON object:
  #   {"store_path":"/nix/store/f7z7lsh7r69shyfs2vlfgdknp7hz8k1g-floxenv"}
  echo "{\"store_path\":\"$store_path\"}"
  exit 0
}

function lock_flake_installable() {
  echo "ERROR: lock-flake-installable subcommand is not supported" >&2
  exit 1
}

declare USAGE="Usage: $NAME (buildenv|linkenv|lock-flake-installable) <args>"
if [ $# -eq 0 ]; then
  echo "ERROR: missing subcommand" >&2
  echo "$USAGE" >&2
  exit 1
fi
case "$1" in
  buildenv)
    shift
    buildenv "$@"
    echo "ERROR: buildenv() should not return" >&2
    exit 1
    ;;
  linkenv)
    shift
    linkenv "$@"
    echo "ERROR: linkenv() should not return" >&2
    exit 1
    ;;
  lock-flake-installable)
    shift
    lock_flake_installable "$@"
    echo "ERROR: lock_flake_installable() should not return" >&2
    exit 1
    ;;
  *)
    echo "ERROR: unknown subcommand '$1'" >&2
    echo "$USAGE" >&2
    exit 1
    ;;
esac
