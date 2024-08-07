
set -euxo pipefail

FHS_ROOT="${1}"
LDCONFIG_MOCK="${2}"

# Get the function without executing it all.
_FLOX_ENV_CUDA_DETECTION=0 source "${FLOX_ENV}/etc/profile.d/0800_cuda.sh"

LIBS_BEFORE="$FLOX_ENV_LIB_DIRS"
activate_cuda "${FHS_ROOT}" "${LDCONFIG_MOCK}"
LIBS_AFTER="$FLOX_ENV_LIB_DIRS"

if [[ "$LIBS_AFTER" == "$LIBS_BEFORE" ]]; then
    set +x # make it easier to read the comparison
    echo "FLOX_ENV_LIB_DIRS was not modified and it should have been"
    echo "  before: ${LIBS_BEFORE}"
    echo "  after:  ${LIBS_AFTER}"
    exit 1
fi

# Assert directory presence and list contents to help debug test failures.
ls -al "${FLOX_ENV_PROJECT}/.flox/lib"

# Non-exhaustive selection of patterns from the mock output.
# NB: libdxcore isn't covered by the mock.
[ -L "${FLOX_ENV_PROJECT}/.flox/lib/libcuda.so" ]
[ -L "${FLOX_ENV_PROJECT}/.flox/lib/libcuda.so.1" ]
[ -L "${FLOX_ENV_PROJECT}/.flox/lib/libcudart.so" ]
[ -L "${FLOX_ENV_PROJECT}/.flox/lib/libcudart.so.12" ]
[ -L "${FLOX_ENV_PROJECT}/.flox/lib/libnvidia-ml.so" ]
[ -L "${FLOX_ENV_PROJECT}/.flox/lib/libnvidia-ml.so.1" ]
[ -L "${FLOX_ENV_PROJECT}/.flox/lib/libnvidia-nvvm.so" ]
[ -L "${FLOX_ENV_PROJECT}/.flox/lib/libnvidia-nvvm.so.4" ]
