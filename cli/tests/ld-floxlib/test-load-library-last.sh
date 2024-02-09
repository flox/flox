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

# Enable auditing so that the logs clearly highlight any instances where
# ld-floxlib.so serves up a library from $FLOX_ENV.
export LD_FLOXLIB_AUDIT=1

# Our aim in this test is to prove that our use of LD_AUDIT will not
# serve up Nix libraries to native "system" binaries, and to do that
# we need a representative binary and a library that it depends upon.
# As it happens, /bin/sh is the _the only_ "system" path guaranteed
# to be present on Linux variants, including NixOS, and the one shared
# library that it will always depend upon is libc itself.

# This test invokes /bin/sh to examine its own shared libraries (as
# found in /proc/$$/maps) to confirm that only one copy of libc is
# present as provided from the "system" location, and not the one
# as provided from $FLOX_ENV/lib.

# Gather full paths to required tools for the logs.
_env="$(type -P env)"
_patchelf="$(type -P patchelf)"
_awk="$(type -P awk)"
_realpath="$(type -P realpath)"
_realshell="$($_realpath /bin/sh)"

# Start by using the interpreter to discern the "system" libc from /bin/sh.
system_interpreter="$($_env -i $_patchelf --print-interpreter $_realshell)"
system_libc="$($_env -i $system_interpreter --list $_realshell | $_awk '/libc.* => / {print $3; exit}')"
system_libc_realpath="$($_realpath ${system_libc})"

# Invoke /bin/sh to examine the contents of /proc/$$/maps and assert
# that it has loaded a copy of the one expected libc version. Take care
# to avoid using `cat` to examine the contents of /proc/$$/maps because
# that has the effect of reporting the version of libc that it is using,
# and to invoke the while loop in the foreground so that it can set the
# found_* variables for inspection afterwards.
declare -i found_system_libc=0
declare -i found_other_libc=0
while read -a maps_line; do
    numwords=${#maps_line[@]}
    lastword="${maps_line[$((numwords-1))]}"
    case "$lastword" in
    "$system_libc_realpath")
        found_system_libc=1
        ;;
    "*/libc.so.*")
        found_other_libc=1
        ;;
    esac
done < <($_realshell -c 'while read line; do echo $line; done < /proc/$$/maps')

echo "Assert we found the system libc" 1>&2
[ $found_system_libc -eq 1 ]

echo "Assert we did not encounter another libc" 1>&2
[ $found_other_libc -eq 0 ]

# If we get this far without failing, that's success.
