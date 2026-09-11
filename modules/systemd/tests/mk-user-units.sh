#!/bin/sh
#
# Generate user-manager copies of the units for the service-manager tests.
#
#   mk-user-units.sh <output-dir> <state-dir> <conf-dir> <libexec-dir> <flox-bin>
#
# The units under ../units are written for the system manager. Four things
# have to change to load them into a `systemd --user` manager, and they are
# each applied as a narrow substitution so it stays obvious what fidelity the
# harness gives up:
#
#   1. Names are prefixed with `floxtest-`, including the cross-references
#      between them, so a test run cannot collide with a real installation in
#      the developer's own user manager.
#   2. `User=`/`Group=` are dropped. A user manager cannot change uid; the
#      lines load but fail at exec time.
#   3. Absolute install paths (/usr/libexec/flox, /var/lib/flox) are pointed
#      at the checkout and a temp directory.
#   4. `Environment=` lines are added so the scripts find the temp conf and
#      state directories and the stub flox.
#
# What survives the transform - and is therefore what these tests are for -
# is the structure: %i instantiation, Requires=/After= ordering, the drop-in
# ExecStart= reset, and timer scheduling.
set -eu

outdir=${1:?usage: mk-user-units.sh <outdir> <statedir> <confdir> <libexec> <floxbin>}
statedir=${2:?}
confdir=${3:?}
libexec=${4:?}
floxbin=${5:?}

srcdir=$(CDPATH= cd -- "$(dirname -- "$0")/../units" && pwd)
mkdir -p "$outdir"

for src in "$srcdir"/*; do
  base=$(basename "$src")
  case "$base" in
    flox@*)          dest="floxtest@${base#flox@}" ;;
    flox-pull@*)     dest="floxtest-pull@${base#flox-pull@}" ;;
    flox-autopull@*) dest="floxtest-autopull@${base#flox-autopull@}" ;;
    *)               dest="floxtest-$base" ;;
  esac

  sed \
    -e 's|^User=.*$||' \
    -e 's|^Group=.*$||' \
    -e "s|/usr/libexec/flox|$libexec|g" \
    -e "s|/var/lib/flox|$statedir|g" \
    -e 's|flox-pull@|floxtest-pull@|g' \
    -e 's|flox-autopull@|floxtest-autopull@|g' \
    -e 's|^Description=Flox|Description=Floxtest|' \
    "$src" > "$outdir/$dest"

  # Point the scripts at the harness's directories and stub flox.
  if grep -q '^ExecStart=' "$outdir/$dest"; then
    sed -i "/^\[Service\]/a\\
Environment=FLOX_CONF_DIR=$confdir\\
Environment=FLOX_STATE_DIR=$statedir\\
Environment=FLOX_LIBEXEC=$libexec\\
Environment=FLOX_BIN=$floxbin\\
Environment=FLOX_STUB_LOG=$statedir/invocations" "$outdir/$dest"
  fi
done

# The system unit orders itself after network-online.target, which does not
# exist in a user manager and would leave every job queued forever.
sed -i -e 's|^Wants=network-online.target$||' \
       -e 's|^After=network-online.target |After=|' \
       -e 's|^After=network-online.target$||' \
       "$outdir"/floxtest*.service
