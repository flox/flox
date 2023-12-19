#!/usr/bin/env sh

set -euxo pipefail

# The common ancestor to the release branch and main
#
#   ... -- A -- (B) -- ...       main
#                 \
#                  \ - C -- ...  release/*
fork_point="$(git merge-base --fork-point main)"

# The current "head" commit pointed to by "main"
#
#   ... -- A -- (B) -- ... -- (X)   main
main_head="$(git show-ref -s --heads main)"

if [[ -z "$fork_point" ]]; then
  echo "::error::main and release branch do not share a common ancestor"
  exit 2
fi

if [[ -z "$main_head" ]]; then
  echo "::error::couldn't determine head commit of main"
  exit 3
fi

if [[ "$fork_point" != "$main_head" ]]; then
  echo "::error::release branch needs to be rebased onto main"
  exit 4
fi

increment_flag=
if [[ "$INCREMENT" = "MAJOR" ]]; then
  increment_flag="--increment=PATCH"
fi
if [[ "$INCREMENT" = "MINOR" ]]; then
  increment_flag="--increment=MINOR"
fi
if [[ "$INCREMENT" = "PATCH" ]]; then
  increment_flag="--increment=PATCH"
fi
if [[ "$INCREMENT" = "AUTO" ]]; then
  increment_flag=
fi

# enter nix shell before resetting to make sure any nix changes on release/ are used
git reset --soft main

nix develop .#ci \
  -c cz bump --yes "$increment_flag"

# store release tag
echo "TAG=$(git describe --abbrev=0 --tags)" >> "$GITHUB_OUTPUT"
