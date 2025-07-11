cmake_minimum_required(VERSION 3.31)
project(HelloWorld NONE)   # NO languages

# 1) Try to locate HelloTarget.cmake somewhere under CMAKE_PREFIX_PATH
find_file(HELLO_TARGET_FILE
  NAMES HelloTarget.cmake
  PATH_SUFFIXES share/hello
)

if(NOT HELLO_TARGET_FILE)
  message(FATAL_ERROR
    "Could not find HelloTarget.cmake in CMAKE_PREFIX_PATH (${CMAKE_PREFIX_PATH})"
  )
endif()
message(STATUS "Including helper: ${HELLO_TARGET_FILE}")

# 2) Pull in the custom-target definition
include(${HELLO_TARGET_FILE})
