### Test 1: load libraries found in $FLOX_ENV_LIBS last
set -ex

# Build glibc version probe using "system" version of glibc.
cc -o get-glibc-version ./get-glibc-version.c

# Glean "system" glibc version by first clearing the environment with "env -i".
system_glibc_version=$( env -i -- ./get-glibc-version )

# Glean "environment" glibc version.
environment_glibc_version=$( ./get-glibc-version )

# Force use of "environment" glibc first with LD_LIBRARY_PATH.
forced_environment_glibc_version=$( LD_LIBRARY_PATH=$FLOX_ENV_LIBS ./get-glibc-version )

# Confirm that the system and environment invocations serve up the same version.
[ "$system_glibc_version" = "$environment_glibc_version" ]

# Confirm that the the forced environment version is different.
[ "$system_glibc_version" != "$forced_environment_glibc_version" ]

# Finally confirm that the environment is serving up the exact expected version.
[ "$forced_environment_glibc_version" == "2.37" ]
