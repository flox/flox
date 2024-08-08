set -euo pipefail

MANIFEST_CONTENTS_2="$(cat << "EOF"
  version = 1

  [services]
  one.command = "sleep infinity"
  two.command = "sleep infinity"
EOF
)"

echo "$MANIFEST_CONTENTS_2" | "$FLOX_BIN" edit -f -

"$FLOX_BIN" services start two
