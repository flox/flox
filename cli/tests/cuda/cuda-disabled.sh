set -euxo pipefail

FHS_ROOT="${1}"
LDCONFIG_MOCK="${2}"

# Get the function without loading support.
_FLOX_ENV_CUDA_DETECTION=0 source "${FLOX_ENV}/etc/profile.d/0800_cuda.sh"

LIBS_BEFORE="$FLOX_ENV_LIB_DIRS"
activate_cuda "${FHS_ROOT}" "${LDCONFIG_MOCK}"
LIBS_AFTER="$FLOX_ENV_LIB_DIRS"

if [[ "$LIBS_AFTER" != "$LIBS_BEFORE" ]]; then
    set +x # make it easier to read the comparison
    echo "FLOX_ENV_LIB_DIRS was modified and it shouldn't have been"
    echo "  before: ${LIBS_BEFORE}"
    echo "  after:  ${LIBS_AFTER}"
    exit 1
fi

# Assert directory absence and list contents to help debug test failures.
! ls -al "${FLOX_ENV_PROJECT}/.flox/lib"
