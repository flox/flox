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

# Make sure we avoid a race of service one failing to complete
"${TESTS_DIR}"/services/wait_for_service_status.sh one:Completed

"$FLOX_BIN" services start
"$FLOX_BIN" services status
"$FLOX_BIN" services logs one
