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
# 2) does NOT contain "$FLOX_ENV/sbin" at all, sbin is excluded by
#    default; a separate test exercises the --add-sbin opt-in).
for p in "${path_array[@]}"; do
    if [ "$p" = "$FLOX_ENV/sbin" ]; then
        echo "ERROR: PATH unexpectedly contains $FLOX_ENV/sbin" >&2;
        exit 1;
    fi
done
# 3) does not contain "$FLOX_ENV/bin" more than once
declare seen_bin=""
for p in "${path_array[@]}"; do
    if [ "$p" = "$FLOX_ENV/bin" ]; then
        if [ -n "$seen_bin" ]; then
            exit 1
        else
            seen_bin=1
        fi
    fi
done
