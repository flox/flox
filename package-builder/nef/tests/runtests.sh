#!/usr/bin/env nix
#!nix shell github:nix-community/nix-unit
#!nix --command bash

pushd "$(dirname -- "$0")"

nix-unit ./.
