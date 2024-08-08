set -euo pipefail

MANIFEST_CONTENTS_2="$(cat << "EOF"
  version = 1

  [services]
  one.command = "echo $FOO"
  two.command = "sleep infinity"

  [hook]
  on-activate = "export FOO=foo_two"
EOF
)"

echo "$MANIFEST_CONTENTS_2" | "$FLOX_BIN" edit -f -

# TODO: don't use follow once logs without follow are implemented
"$FLOX_BIN" services logs one --follow > one.log &
LOGS_PID="$!"
"$FLOX_BIN" services start
kill "$LOGS_PID"
"$FLOX_BIN" services status
