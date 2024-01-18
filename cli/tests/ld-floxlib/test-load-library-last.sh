### Verify environment

# This script is run using `flox activate --` and is therefore not using
# `bats` for assertions, so treat every invocation in this script as an
# assertion (-e) and be verbose about it (-x).
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
# We build it with the older version because glibc is backwards-compatible
# to support old binaries, but newer binaries will be looking for symbols
# not necessarily found in older versions of glibc.
cc -o get-glibc-version ./get-glibc-version.c -I"$FLOX_ENV"/include -L"$FLOX_ENV"/lib

# In order to simulate a binary created on any old Linux version we
# must first unwind the [2] tricks that Nix performs to force executables
# to run with the exact version of glibc that it was compiled with.

# 1. Remove the custom RUNPATH added by gcc-wrapper.
patchelf --remove-rpath ./get-glibc-version

# 2. set the interpreter to the one used by the default system.
#    This is somewhat more challenging because we need to derive
#    what LD interpreter is being used by default on this variant of
#    Linux, and that is most reliably done by observing the one used
#    for /bin/sh, _the only_ path guaranteed to be present on all
#    variants of Linux, including NixOS.
original_interpreter=$(patchelf --print-interpreter ./get-glibc-version)
system_interpreter=$(patchelf --print-interpreter /bin/sh)
patchelf --set-interpreter \
  "$system_interpreter" ./get-glibc-version

# Invoke it once just to record the default behaviour for the logs.
LD_FLOXLIB_DEBUG=1 ./get-glibc-version

# Glean "system" glibc version by first clearing the environment with "env -i".
system_glibc_version="$( env -i -- ./get-glibc-version )"

# Glean "environment" glibc version.
environment_glibc_version="$( ./get-glibc-version )"

# Force use of "environment" glibc first with LD_LIBRARY_PATH. Unlike other
# libraries, with a change of glibc versions it's essential that the version
# of the ld interpreter exactly matches that of glibc, so before running this
# test we have to first set the interpreter back to the matching version.
patchelf --set-interpreter $FLOX_ENV/lib/ld-linux-*.so.* ./get-glibc-version
# Take note of the result for the logs
realpath "$(patchelf --print-interpreter ./get-glibc-version)"
forced_environment_glibc_version="$(
  LD_DEBUG=libs LD_LIBRARY_PATH="$FLOX_ENV_LIB_DIRS" ./get-glibc-version
)"

# Confirm that the system and environment invocations serve up the same version.
[ "$system_glibc_version" = "$environment_glibc_version" ]

# Confirm that the the forced environment version is different.
[ "$system_glibc_version" != "$forced_environment_glibc_version" ]

# Finally confirm that the environment is serving up the exact string for
# version 2.34 as found in the nixpkgs $PKGDB_NIXPKGS_REV_OLDER revision.
[ "$forced_environment_glibc_version" = "GNU C Library (glibc) version: 2.34" ]
