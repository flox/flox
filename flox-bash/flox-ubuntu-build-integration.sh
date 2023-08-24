#!/usr/bin/env bash
set -e 
set -o pipefail
export FLOXPATH="/home/ubuntu/flox-internal"
echo "**** PERFORMING BUILD OF FLOX AGAINST LATEST REVISION ****"
export TERM="xterm-256color"
mkdir -p "$FLOXPATH" && cd "$FLOXPATH"
#here we used latest installed verison of flox via package to build flox
flox build github:flox/floxpkgs#stable.flox --override-input flox github:flox/flox-bash-private?rev="$1"
echo "**** PERFORMING FLOX DEFAULT TEMPLATE INTEGRATION TEST ****"
#now we use the built version of flox wrapper from the PR or commit on gh to perform tests

curl -O https://raw.githubusercontent.com/flox/flox-bash-private/"$1"/test.bats
bats test.bats
#du -hDd0 /nix
rm -rf myproj
mkdir -p myproj && cd myproj
curl -O https://raw.githubusercontent.com/flox/floxpkgs/master/test/flox-init-default.exp
curl -O https://raw.githubusercontent.com/flox/floxpkgs/master/test/default.exp
curl -O https://raw.githubusercontent.com/flox/floxpkgs/master/test/default.bats
expect flox-init-default.exp
expect default.exp
##du -hDd0 /nix
cd ../ && rm -rf myproj
