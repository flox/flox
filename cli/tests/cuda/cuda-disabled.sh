set -euxo pipefail

FHS_ROOT="${1}"
LDCONFIG_MOCK="${2}"

# Get the function without loading support.
_FLOX_ENV_CUDA_DETECTION=0 source "${FLOX_ENV}/etc/profile.d/0800_cuda.sh"
activate_cuda "${FHS_ROOT}" "${LDCONFIG_MOCK}"

# Assert directory absence and list contents to help debug test failures.
! ls -al "${FLOX_ENV_PROJECT}/.flox/lib"
