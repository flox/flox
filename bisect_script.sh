#!/usr/bin/env bash
export FLOX_SHELL=zsh
echo "Building"
nix develop -c just build &> /dev/null
echo "Setting up environment"
cli/target/debug/flox delete -f &> /dev/null
cli/target/debug/flox init &> /dev/null
cli/target/debug/flox install nodejs &> /dev/null
version=$(cli/target/debug/flox activate -- node --version)
echo "version = $version"
if [ ! "$version" = "v20.18.1" ]; then
  echo "No match"
  exit 1
else
  echo "versions matched"
fi
