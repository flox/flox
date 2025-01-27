set -euxo pipefail

FHS_ROOT="${1}"
LDCONFIG_MOCK="${2}"

# Get the function without loading support.
_FLOX_ENV_CUDA_DETECTION=0 source "${FLOX_ENV}/etc/profile.d/0800_cuda.sh"

LD_FLOXLIB_FILES_PATH_BEFORE="${LD_FLOXLIB_FILES_PATH:-}"
activate_cuda "${FHS_ROOT}" "${LDCONFIG_MOCK}"
LD_FLOXLIB_FILES_PATH_AFTER="${LD_FLOXLIB_FILES_PATH:-}"

if [[ "$LD_FLOXLIB_FILES_PATH_AFTER" != "$LD_FLOXLIB_FILES_PATH_BEFORE" ]]; then
    set +x # make it easier to read the comparison
    echo "LD_FLOXLIB_FILES_PATH was modified and it shouldn't have been"
    echo "  before: ${LD_FLOXLIB_FILES_PATH_BEFORE}"
    echo "  after:  ${LD_FLOXLIB_FILES_PATH_AFTER}"
    exit 1
fi
