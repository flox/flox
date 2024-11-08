# Exit on any failure, always be verbose
set -ex

# Assert that the FLOX_ENV environment variable is set
[ -n "$FLOX_ENV" ]

# Split the PATH environment variable into an array
declare -a path_array
IFS=: read -ra path_array <<< "$PATH"

# Assert that the PATH environment variable:
# 1) contains "$FLOX_ENV/bin" as its first element
[ "${path_array[0]}" = "$FLOX_ENV/bin" ] || {
  echo "ERROR: first PATH element not $FLOX_ENV/bin" >&2;
  exit 1;
}
# 2) contains "$FLOX_ENV/sbin" as its second element
[ "${path_array[1]}" = "$FLOX_ENV/sbin" ] || {
  echo "ERROR: second PATH element not $FLOX_ENV/sbin" >&2;
  exit 1;
}
# 3) contains neither of the above more than once
declare seen_bin=""
declare seen_sbin=""
for p in "${path_array[@]}"; do
    if [ "$p" = "$FLOX_ENV/bin" ]; then
        if [ -n "$seen_bin" ]; then
            exit 1
        else
            seen_bin=1
        fi
    fi
    if [ "$p" = "$FLOX_ENV/sbin" ]; then
        if [ -n "$seen_sbin" ]; then
            exit 1
        else
            seen_sbin=1
        fi
    fi
done
