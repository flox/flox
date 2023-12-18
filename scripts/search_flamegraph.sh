#!/usr/bin/env bash

set -euo pipefail

# Note: this only works on macOS and must be run as root since DTrace requires root
#       to collect traces. It also requires the `inferno` flamegraph toolkit.

SAMPLES="${PWD}/out.stacks"
COLLAPSED="${PWD}/stacks.collapsed"
SVG="${PWD}/flamegraph.svg"

# Delete any existing databases
rm -rf /var/root/.cache/flox

# Collect the traces
sudo dtrace -c 'pkgdb search --ga-registry --match-name hello' -o "$SAMPLES" -n 'profile-997 /execname == "pkgdb"/ { @[ustack(100)] = count(); }'

# Turn the traces into a flamegraph
inferno-collapse-dtrace "$SAMPLES" > "$COLLAPSED"
inferno-flamegraph --colordiffusion --truncate-text-right "$COLLAPSED" > "$SVG"

# Delete the intermediate files
rm "$SAMPLES" "$COLLAPSED"
