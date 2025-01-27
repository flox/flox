
set -euxo pipefail

FHS_ROOT="${1}"
LDCONFIG_MOCK="${2}"

# Get the function without executing it all.
_FLOX_ENV_CUDA_DETECTION=0 source "${FLOX_ENV}/etc/profile.d/0800_cuda.sh"

LD_FLOXLIB_FILES_PATH_BEFORE="${LD_FLOXLIB_FILES_PATH:-}"
activate_cuda "${FHS_ROOT}" "${LDCONFIG_MOCK}"
LD_FLOXLIB_FILES_PATH_AFTER="${LD_FLOXLIB_FILES_PATH:-}"

if [[ "$LD_FLOXLIB_FILES_PATH_AFTER" == "$LD_FLOXLIB_FILES_PATH_BEFORE" ]]; then
    set +x # make it easier to read the comparison
    echo "LD_FLOXLIB_FILES_PATH was not modified and it should have been"
    echo "  before: ${LD_FLOXLIB_FILES_PATH_BEFORE}"
    echo "  after:  ${LD_FLOXLIB_FILES_PATH_AFTER}"
    exit 1
fi

# Assert LD_FLOXLIB_FILES_PATH_AFTER is not empty and list its contents
# to help debug test failures.
[ -n "$LD_FLOXLIB_FILES_PATH_AFTER" ]
echo "LD_FLOXLIB_FILES_PATH_AFTER=$LD_FLOXLIB_FILES_PATH_AFTER"

# Non-exhaustive selection of patterns from the mock output.
# NB: libdxcore isn't covered by the mock.
declare -a expected=(
  "libcuda.so"
  "libcuda.so.1"
  "libcudart.so"
  "libcudart.so.12"
  "libnvidia-ml.so"
  "libnvidia-ml.so.1"
  "libnvidia-nvvm.so"
  "libnvidia-nvvm.so.4"
)
IFS=":"
for pattern in "${expected[@]}"; do
  echo "Checking for ${pattern}" 1>&2
  echo $LD_FLOXLIB_FILES_PATH_AFTER \
    | xargs -n 1 basename | grep "^${pattern}$" > /dev/null \
    || { echo "Failed to find ${pattern}" 1>&2; exit 1; }
done
