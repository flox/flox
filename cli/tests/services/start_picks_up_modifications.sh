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
for i in {1..5}; do
  if "$FLOX_BIN" services status | grep "Completed"; then
    break
  fi
  sleep .1
done
if [ "$i" -eq 5 ]; then
  exit 1
fi

"$FLOX_BIN" services start
"$FLOX_BIN" services status
# TODO: check logs once implemented without --follow
