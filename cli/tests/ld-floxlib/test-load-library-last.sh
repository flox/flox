### Verify environment
set -ex

# FLOX_ENV_LIB_DIRS is defined
test -n "$FLOX_ENV_LIB_DIRS"

# LD_AUDIT is defined, exists and points to ld-floxlib.so
test -e "$LD_AUDIT"
[[ "$LD_AUDIT" == */ld-floxlib.so ]]

# LD_LIBRARY_PATH is not defined
test -z "$LD_LIBRARY_PATH"

### Test 1: load libraries found in $FLOX_ENV_LIBS last

# Build glibc version probe using [assumed] older "env" version of glibc.
# It's essential that we build it with the older version because glibc
# is backwards-compatible to run against old binaries, but newer binaries
# cannot necessarily run against older versions of glibc.
cc -o get-glibc-version ./get-glibc-version.c -I"$FLOX_ENV"/include -L"$FLOX_ENV"/lib

# Remove all the tricks that Nix does to force this executable
# to run with the exact version it was compiled with.

# 1. Remove the custom RUNPATH added by gcc-wrapper.
patchelf --remove-rpath ./get-glibc-version

# 2. set the interpreter to the one used by the default system.
#    This is somewhat more challenging because we need to derive
#    what LD interpreter is being used by default on this variant of
#    Linux, and that is most reliably done by observing the one used
#    for /bin/sh, _the only_ path guaranteed to be present on all
#    variants of Linux, including NixOS.
system_interpreter=$(patchelf --print-interpreter /bin/sh)
patchelf --set-interpreter "$system_interpreter" ./get-glibc-version

# Invoke it once just to record the default behaviour for the logs.
LD_FLOXLIB_DEBUG=1 ./get-glibc-version

# Glean "system" glibc version by first clearing the environment with "env -i".
system_glibc_version=$( env -i -- ./get-glibc-version )

# Glean "environment" glibc version.
environment_glibc_version=$( ./get-glibc-version )

# Force use of "environment" glibc first with LD_LIBRARY_PATH.
forced_environment_glibc_version=$( LD_DEBUG=libs LD_LIBRARY_PATH="$FLOX_ENV_LIB_DIRS" ./get-glibc-version )

# Confirm that the system and environment invocations serve up the same version.
[ "$system_glibc_version" = "$environment_glibc_version" ]

# Confirm that the the forced environment version is different.
[ "$system_glibc_version" != "$forced_environment_glibc_version" ]

# Finally confirm that the environment is serving up the exact string for
# version 2.34 as found in the nixpkgs $PKGDB_NIXPKGS_REV_OLDER revision.
[ "$forced_environment_glibc_version" = "GNU C Library (glibc) version: 2.34" ]
