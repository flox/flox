#!/usr/bin/env bash
# ============================================================================ #
#
# Verify the /nix sticky disk before anything uses it.
#
# Every run starts from the last snapshot saved on main, so one corrupted
# store would fail every job until the disk is deleted by hand. If the store
# does not verify, wipe it: this job then starts cold, and the next main run
# saves a fresh snapshot over the broken one.
#
# ---------------------------------------------------------------------------- #

set -euo pipefail

# Installed by cachix/install-nix-action on an earlier run. Missing means the
# disk is empty (first run, or wiped), so there is nothing to verify.
nix_store=/nix/var/nix/profiles/default/bin/nix-store
if [ ! -x "$nix_store" ]; then
  echo "No Nix installation on the sticky disk, nothing to verify."
  exit 0
fi

if sudo "$nix_store" --verify --check-contents; then
  echo "Nix store verified."
  exit 0
fi

echo "::warning::Nix store on the sticky disk failed verification, starting cold"
sudo find /nix -mindepth 1 -maxdepth 1 -exec rm -rf {} +
sudo mkdir -p /nix/store
sudo chmod -R 777 /nix
