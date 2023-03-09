#!/usr/bin/env sh

fork_point="$(git merge-base --fork-point main)"
main_head="$(git show-ref -s --heads main)"


if [[ -z "$fork_point" ]]; then
    exit 2
fi

if [[ -z "$main_head" ]]; then
    exit 3
fi

if [[ "$fork_point" != "$main_head" ]]; then
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
    # dont set the flag
fi

# enter nix shell before resetting to make sure any nix changes on release/ are used
git reset --soft main

nix develop .#ci \
    -c cz bump --yes "$increment_flag"

echo "TAG=$(git describe --abbrev=0 --tags)" >> "$GITHUB_OUTPUT"
