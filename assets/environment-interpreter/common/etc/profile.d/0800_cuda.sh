# shellcheck shell=bash
_ldconfig="@iconv@/bin/ldconfig"

# ============================================================================ #
#
# Setup CUDA
#
# ---------------------------------------------------------------------------- #

activate_cuda() {
  local _find="@findutils@/bin/find"
  local _nawk="@nawk@/bin/nawk"
  # Strip a trailing or lone slash so that we can construct it later.
  local fhs_root_prefix="${1%/}"
  # Path to ldconfig that can be substituted for testing.
  local ldconfig_bin="$2"
  # Pattern of libraries that we support.
  local lib_pattern="lib(cuda|nvidia|dxcore).*\.so.*"

  # Only run if _FLOX_ENV_CUDA_DETECTION is set
  if [[ "${_FLOX_ENV_CUDA_DETECTION:-}" != 1 ]]; then
    return 0
  fi

  # Skip when not on Linux
  if [[ "$ldconfig_bin" == "__LINUX_ONLY__" || ! -f "$ldconfig_bin" || ! -x "$ldconfig_bin" ]]; then
    return 0
  fi

  # Skip when no Nvidia device
  if ! ("$_find" "${fhs_root_prefix}/dev" -maxdepth 1 -iname 'nvidia*' -o -iname dxg | read -r); then
    return 0
  fi

  # Use system library cache.
  # shellcheck disable=SC2016
  SYSTEM_LIBS=$(
    { "$ldconfig_bin" --print-cache -C /etc/ld.so.cache 2> /dev/null || echo; } \
      | "$_nawk" -v lib_pattern="^${lib_pattern}" \
        'BEGIN {files=""} $1 ~ lib_pattern {files = ( files == "" ? $NF : files ":" $NF)} END {print files}'
  )

  # Fallback for NixOS.
  if [ -z "$SYSTEM_LIBS" ]; then
    # shellcheck disable=SC2016
    SYSTEM_LIBS=$(
      "$_find" "${fhs_root_prefix}/run/opengl-driver" -type f -print 2> /dev/null \
        | "$_nawk" -v lib_pattern="${lib_pattern}" \
          'BEGIN {files=""} $0 ~ lib_pattern {files = ( files == "" ? $NF : files ":" $NF)} END {print files}'
    )
  fi

  # No matching libs from either results.
  if [ -z "$SYSTEM_LIBS" ]; then
    return 0
  fi

  export LD_FLOXLIB_FILES_PATH="${LD_FLOXLIB_FILES_PATH:+${LD_FLOXLIB_FILES_PATH}:}$SYSTEM_LIBS"
}

activate_cuda "/" "$_ldconfig"

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
