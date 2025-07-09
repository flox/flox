# --- /opt/foo/share/hello/HelloTarget.cmake ---
#
# This module defines a “hello” target that:
#  1. configure_file()s hello.in → hello.txt
#  2. adds an ALL-level custom target to build it

# locate the input and output relative to the top-level CMake dirs
configure_file(
  "${CMAKE_CURRENT_SOURCE_DIR}/hello.in"
  "${CMAKE_CURRENT_BINARY_DIR}/hello.txt"
  @ONLY
)

add_custom_target(hello ALL
  DEPENDS "${CMAKE_CURRENT_BINARY_DIR}/hello.txt"
  COMMENT "Generating hello.txt via HelloTarget.cmake"
)
