# ============================================================================ #
#
# XXX: This file currently passes through all commands to `./pkgdb'.
#      In the future we will drive all build commands from this file.
#
# ============================================================================ #
#
# Target/Task Highlights:
#   - most (default)        Build binaries, libs, and generated files
#   - all                   Build binaries, libs, tests, and generated files
#
#   - bin                   Build binaries
#   - tests                 Build test executables and resources
#   - docs                  Generate documentation
#
#   - check                 Run all tests
#   - bats-check            Run all bats tests
#   - cc-check              Run all C++ unit tests
#
#   - clean                 Remove build artifacts
#   - clean-pch             Remove all `pre-compiled-headers'.
#   - fullclean             Remove build artifacts and metadata files
#
#   - install               Install binaries, shared data, and include files
#   - install-bin           Install binaries
#   - install-data          Install `share/flox/` files
#   - install-libexec       Install `libexec/` executables
#
#   - ccls                  Create `.ccls' file used by CCLS LSP
#   - compilation-databases Create `compile_commands.json' and `.ccls'
#   - cdb                   Create `compile_commands.json' and `.ccls'
#
#   - fmt                   Run `clang-format' across entire project#
#   - iwyu                  Generate `include-what-you-use' report
#   - lint                  Run `clang-tidy' across entire project
#
#
# Tips:
#   - Use `remake --tasks' to see a list of common targets.
#   - Recommend using `make -j' to build in parallel.
#     + For "build then test" `make -j all && make check' is recommended to
#       preserve colored test suite output.
#   - `make cdb` should be run any time you add a new source file so that it
#     can be added to the `compile_commands.json' file.
#   - Use `$(info CXXFLAGS: $(CXXFLAGS))' to print the value of a variable.
#     + This can be placed at global scope or inside of a target.
#     + This is useful for debugging `make' issues.
#     + To run `make' just to see `$(info ...)' output use `make -n'
#       or `make FORCE'.
#
# ---------------------------------------------------------------------------- #

# Warn if undefined variables are referenced.
MAKEFLAGS += --warn-undefined-variables

# Locate filesystem paths relative to this Makefile.
MAKEFILE_DIR ?= $(patsubst %/,%,$(dir $(abspath $(lastword $(MAKEFILE_LIST)))))
REPO_ROOT    := $(MAKEFILE_DIR)


# ---------------------------------------------------------------------------- #

.PHONY: most
.DEFAULT_GOAL = most

most:
	@$(MAKE) -C pkgdb most


# ---------------------------------------------------------------------------- #

compile_commands.json .ccls: %:
	@$(MAKE) -C pkgdb ../$@;


# ---------------------------------------------------------------------------- #

# XXX: This is passes all `make ...' targets through to `make -C pkgdb ...'.
# This attempts to strip `./pkgdb/' from the target name, which could cause
# issues if someone was trying to cook up edge cases; but this works fine for
# our purposes temporarily.
%:
	@$(MAKE) -C pkgdb $(patsubst ./%,%,$(patsubst pkgdb/%,%,$@));


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
