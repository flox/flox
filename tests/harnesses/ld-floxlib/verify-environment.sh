### Verify environment
set -ex

# FLOX_ENV_LIB_DIRS is defined
test -n FLOX_ENV_LIB_DIRS

# LD_AUDIT is defined, exists and points to ld-floxlib.so
env | grep LD
ls -l .flox/run/*/etc/profile.d
test -e $LD_AUDIT
[[ "$LD_AUDIT" == */ld-floxlib.so ]]

# LD_LIBRARY_PATH is not defined
test -z LD_LIBRARY_PATH
