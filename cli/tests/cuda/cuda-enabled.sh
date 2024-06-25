
set -euxo pipefail

FHS_ROOT="${1}"

# Get the function without executing it all.
_FLOX_ENV_CUDA_DETECTION=0 source "${FLOX_ENV}/etc/profile.d/0800_cuda.sh"

LIBS_BEFORE="$FLOX_ENV_LIB_DIRS"
activate_cuda "${FHS_ROOT}"
LIBS_AFTER="$FLOX_ENV_LIB_DIRS"
[[ "$LIBS_AFTER" != "$LIBS_BEFORE" ]]

IFS=':'
for lib_dir in $LIBS_AFTER; do
    find "$lib_dir" -type l || true
done
unset IFS
