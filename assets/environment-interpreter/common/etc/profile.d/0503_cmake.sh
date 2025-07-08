# shellcheck shell=bash

# ============================================================================ #
#
# Setup cmake
#
# ---------------------------------------------------------------------------- #

# Only run if `cmake' is installed to the environment.
if [ -x "$FLOX_ENV/bin/cmake" ]; then
  # cmake requires the CMAKE_PREFIX_PATH variable in order to locate libraries
  # and include files. Note that we replace rather than prepend to the
  # CMAKE_PREFIX_PATH variable, as it is the goal for the Flox developer
  # environment to include all the required dependencies for the project.
  if [ -n "${CMAKE_PREFIX_PATH:-}" ]; then
    echo "WARNING: overriding CMAKE_PREFIX_PATH='$FLOX_ENV'" 1>&2
  fi
  CMAKE_PREFIX_PATH="$FLOX_ENV"
  export CMAKE_PREFIX_PATH
fi

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
