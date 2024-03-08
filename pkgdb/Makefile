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
PKGDB_ROOT   := $(MAKEFILE_DIR)
REPO_ROOT    := $(patsubst %/,%,$(dir $(MAKEFILE_DIR)))


# ---------------------------------------------------------------------------- #

.PHONY: all clean fullclean FORCE ignores most
.DEFAULT_GOAL = most


# ---------------------------------------------------------------------------- #

# Install Prefixes
# ----------------

PREFIX ?= $(PKGDB_ROOT)/build
BINDIR ?= $(PREFIX)/bin
LIBDIR ?= $(PREFIX)/lib


# ---------------------------------------------------------------------------- #

# Utilities
# ---------
# We use variables to refer to all tools so that we can easily override them
# from the command line.

BATS       ?= bats
BEAR       ?= bear
CAT        ?= cat
CP         ?= cp
CXX        ?= c++
DOXYGEN    ?= doxygen
FMT        ?= clang-format
GREP       ?= grep
LN         ?= ln -f
MKDIR      ?= mkdir
MKDIR_P    ?= $(MKDIR) -p
NIX        ?= nix --extra-experimental-features 'flakes nix-command'
PKG_CONFIG ?= pkg-config
RM         ?= rm -f
SED        ?= sed
TEE        ?= tee
TEST       ?= test
TIDY       ?= clang-tidy
TOUCH      ?= touch
TR         ?= tr
UNAME      ?= uname


# ---------------------------------------------------------------------------- #

# Detect OS and Set Shared Library Extension
# ------------------------------------------

OS ?= $(shell $(UNAME))
OS := $(OS)
ifndef libExt
ifeq (Linux,$(OS))
libExt ?= .so
else
libExt ?= .dylib
endif  # ifeq (Linux,$(OS))
endif  # ifndef libExt

TARGET_ARCH ?=


# ---------------------------------------------------------------------------- #

# Detect the C++ compiler toolchain
# ---------------------------------

ifndef TOOLCHAIN

ifneq "" "$(shell $(CXX) --version|$(GREP) -i 'gcc'||:)"
TOOLCHAIN = gcc
else ifneq "" "$(shell $(CXX) --version|$(GREP) -i 'clang'||:)"
TOOLCHAIN = clang
else
$(error "Unable to detect C++ compiler toolchain")
endif  # ifneq "" "$(shell $(CXX) --version|$(GREP) -i 'gcc'||:)"

else  # ifndef TOOLCHAIN

# If the user set TOOLCHAIN, ensure that it is valid.
ifeq "" "$(filter gcc clang,$(TOOLCHAIN))"
$(error "Invalid C++ compiler toolchain: $(TOOLCHAIN)")
endif  # ifeq "" "$(filter gcc clang,$(TOOLCHAIN))"

endif  # ifndef TOOLCHAIN


# ---------------------------------------------------------------------------- #

# Read info from files
# --------------------

VERSION := $(file < $(PKGDB_ROOT)/.version)

# Files used by `pkgdb buildenv' need to be added to the `nix' store.
PROFILE_D_SCRIPTS_DIR ?= $(shell                            \
  $(NIX) build .\#flox-pkgdb.envs.PROFILE_D_SCRIPTS_DIR --print-out-paths)
SET_PROMPT_BASH_SH ?= $(shell                                                  \
  $(NIX) store add-path -n set-prompt.bash.sh src/buildenv/assets/set-prompt.bash.sh)
SET_PROMPT_ZSH_SH ?= $(shell                                                  \
  $(NIX) store add-path -n set-prompt.zsh.sh src/buildenv/assets/set-prompt.zsh.sh)
CONTAINER_BUILDER_PATH ?= $(shell                                                  \
  $(NIX) store add-path -n mkContainer.nix src/buildenv/assets/mkContainer.nix)

COMMON_NIXPKGS_URL ?= $(shell \
  $(NIX) eval --raw .\#flox-pkgdb.envs.COMMON_NIXPKGS_URL)

# ---------------------------------------------------------------------------- #

# rwildcard DIRS, PATTERNS
# ------------------------
# Recursive wildcard.
#   Ex:  $(call rwildcard,src,*.cc *.hh)
rwildcard = $(foreach d,$(wildcard $(1:=/*)),$(call rwildcard,$d,$2)        \
                                             $(filter $(subst *,%,$2),$d))


# ---------------------------------------------------------------------------- #

# Various file and target lists

HEADERS        = $(call rwildcard,include,*.hh)
SRCS           = $(call rwildcard,src,*.cc)
test_SRCS      = $(sort $(wildcard tests/*.cc))
ALL_SRCS       = $(SRCS) $(test_SRCS)
BINS           = bin/pkgdb
ifeq (Linux,$(OS))
LIBS           = lib/ld-floxlib.so
else
LIBS           =
endif  # ifeq (Linux,$(OS))
TEST_UTILS     = $(addprefix tests/,is_sqlite3 search-params)
TESTS          = $(filter-out $(TEST_UTILS),$(test_SRCS:.cc=))
TEST_DATA      =
ALL_BINS       = $(TESTS) $(TEST_UTILS) $(BINS)
ALL_LIBS       = $(LIBS)
CLEANDIRS      =
CLEANFILES     = $(ALL_SRCS:.cc=.o) $(ALL_BINS) $(ALL_LIBS)
FULLCLEANDIRS  =
FULLCLEANFILES =

# Where to find test suite input data files.
TEST_DATA_DIR = $(PKGDB_ROOT)/tests/data


# ---------------------------------------------------------------------------- #

# Files which effect dependencies, external inputs, and `*FLAGS' values.
DEPFILES =
DEPFILES += $(addprefix $(REPO_ROOT)/flake.,nix lock)
DEPFILES += $(REPO_ROOT)/pkgs/flox-pkgdb/default.nix
DEPFILES += $(REPO_ROOT)/pkgs/nlohmann_json/default.nix
DEPFILES += $(REPO_ROOT)/shells/default/default.nix


# ---------------------------------------------------------------------------- #

# Compiler Flags
# --------------

EXTRA_LDFLAGS ?=
EXTRA_CFLAGS  ?=
CPPFLAGS  	  ?=

# You can disable these optional gripes with `make EXTRA_CXXFLAGS='' ...;'
ifndef EXTRA_CXXFLAGS
EXTRA_CXXFLAGS = -Wall -Wextra -Wpedantic
# Clang only
ifeq (clang,$(TOOLCHAIN))
EXTRA_CXXFLAGS += -Wno-gnu-zero-variadic-macro-arguments
endif  # ifneq (clang,$(TOOLCHAIN))
endif	# ifndef EXTRA_CXXFLAGS

CXXFLAGS ?= $(EXTRA_CFLAGS) $(EXTRA_CXXFLAGS)
CXXFLAGS += '-I$(PKGDB_ROOT)/include'
CXXFLAGS += '-DFLOX_PKGDB_VERSION="$(VERSION)"'

LDFLAGS  ?= $(EXTRA_LDFLAGS)

# Release Mode
RELEASE ?=
ifneq (,$(RELEASE))
CXXFLAGS += -O3 -DNDEBUG
LDFLAGS  += -O3
endif # ifneq ($(RELEASE),)

# Debug Mode
DEBUG ?=
ifneq (,$(DEBUG))
DEBUG ?=
ifeq (gcc,$(TOOLCHAIN))
CXXFLAGS += -ggdb3 -pg -fno-omit-frame-pointer
LDFLAGS  += -ggdb3 -pg -fno-omit-frame-pointer
else # Clang
CXXFLAGS += -g -glldb -fno-omit-frame-pointer -mno-omit-leaf-frame-pointer
LDFLAGS  += -g -glldb -fno-omit-frame-pointer -mno-omit-leaf-frame-pointer
endif # ifeq (gcc,$(TOOLCHAIN))
endif # ifneq ($(DEBUG),)

# Coverage Mode
COV ?=
ifneq (,$(COV))
CXXFLAGS += --coverage -fprofile-arcs -ftest-coverage
LDFLAGS  += --coverage -fprofile-arcs -ftest-coverage
endif # ifneq ($(COV),)


# ---------------------------------------------------------------------------- #

# Dependency Flags
# ----------------

nljson_CFLAGS ?=                                                            \
	$(patsubst -I%,-isystem %,$(shell $(PKG_CONFIG) --cflags nlohmann_json))
nljson_CFLAGS := $(nljson_CFLAGS)

argparse_CFLAGS ?=                                                     \
	$(patsubst -I%,-isystem %,$(shell $(PKG_CONFIG) --cflags argparse))
argparse_CFLAGS := $(argparse_CFLAGS)

boost_CFLAGS ?=                                                              \
  -isystem                                                                   \
  $(shell $(NIX) build --no-link --print-out-paths 'nixpkgs#boost')/include
boost_CFLAGS := $(boost_CFLAGS)

toml_CFLAGS ?=                                                                \
  -isystem                                                                    \
  $(shell $(NIX) build --no-link --print-out-paths 'nixpkgs#toml11')/include
toml_CFLAGS := $(toml_CFLAGS)

sentry_PREFIX ?=                                                        \
	$(shell $(NIX) build --no-link --print-out-paths '.#sentry-native')
sentry_PREFIX := $(sentry_PREFIX)
sentry_CFLAGS  = -isystem $(sentry_PREFIX)/include
sentry_LDFLAGS ?= $(shell $(PKG_CONFIG) --libs sentry-native)
sentry_LDFLAGS = -L$(sentry_PREFIX)/lib -lsentry

sqlite3_CFLAGS ?=                                                     \
	$(patsubst -I%,-isystem %,$(shell $(PKG_CONFIG) --cflags sqlite3))
sqlite3_CFLAGS  := $(sqlite3_CFLAGS)
sqlite3_LDFLAGS ?= $(shell $(PKG_CONFIG) --libs sqlite3)
sqlite3_LDFLAGS := $(sqlite3_LDFLAGS)

sqlite3pp_CFLAGS ?=                                                     \
	$(patsubst -I%,-isystem %,$(shell $(PKG_CONFIG) --cflags sqlite3pp))
sqlite3pp_CFLAGS := $(sqlite3pp_CFLAGS)

yaml_PREFIX ?=                                                          \
	$(shell $(NIX) build --no-link --print-out-paths 'nixpkgs#yaml-cpp')
yaml_PREFIX := $(yaml_PREFIX)
yaml_CFLAGS  = -isystem $(yaml_PREFIX)/include
yaml_LDFLAGS = -L$(yaml_PREFIX)/lib -lyaml-cpp

semver_PREFIX ?=                                                        \
	$(shell $(NIX) build --no-link --print-out-paths '.#cpp-semver')
semver_PREFIX := $(semver_PREFIX)
semver_CFLAGS  = -isystem $(semver_PREFIX)/include

nix_INCDIR ?= $(shell $(PKG_CONFIG) --variable=includedir nix-cmd)
nix_INCDIR := $(nix_INCDIR)
ifndef nix_CFLAGS
_nix_PC_CFLAGS =  $(shell $(PKG_CONFIG) --cflags nix-main nix-cmd nix-expr)
nix_CFLAGS     =  $(boost_CFLAGS) $(patsubst -I%,-isystem %,$(_nix_PC_CFLAGS))
nix_CFLAGS     += -include $(nix_INCDIR)/nix/config.h
endif # ifndef nix_CFLAGS
nix_CFLAGS := $(nix_CFLAGS)
undefine _nix_PC_CFLAGS

ifndef nix_LDFLAGS
nix_LDFLAGS =                                                        \
	$(shell $(PKG_CONFIG) --libs nix-main nix-cmd nix-expr nix-store)
nix_LDFLAGS += -lnixfetchers
endif # ifndef nix_LDFLAGS
nix_LDFLAGS := $(nix_LDFLAGS)


# ---------------------------------------------------------------------------- #

# Add Dependency Flags
# --------------------

CXXFLAGS += $(sqlite3pp_CFLAGS)
CXXFLAGS += $(argparse_CFLAGS)
CXXFLAGS += $(nix_CFLAGS)
CXXFLAGS += $(nljson_CFLAGS)
CXXFLAGS += $(toml_CFLAGS)
CXXFLAGS += $(yaml_CFLAGS)
CXXFLAGS += $(semver_CFLAGS)
CXXFLAGS += $(sentry_CFLAGS)

LDFLAGS += $(nix_LDFLAGS)
LDFLAGS += $(sqlite3_LDFLAGS)
LDFLAGS += $(yaml_LDFLAGS)
LDFLAGS += $(sentry_LDFLAGS)


# ---------------------------------------------------------------------------- #

# Flags for Specific Files
# ------------------------

src/buildenv/realise.o: CXXFLAGS +=                   \
	'-DPROFILE_D_SCRIPTS_DIR="$(PROFILE_D_SCRIPTS_DIR)"'

src/buildenv/realise.o: CXXFLAGS +=               \
	'-DSET_PROMPT_BASH_SH="$(SET_PROMPT_BASH_SH)"'

src/buildenv/realise.o: CXXFLAGS +=               \
	'-DSET_PROMPT_ZSH_SH="$(SET_PROMPT_ZSH_SH)"'

src/buildenv/realise.o: CXXFLAGS +=               \
	'-DCONTAINER_BUILDER_PATH="$(CONTAINER_BUILDER_PATH)"'

src/buildenv/realise.o: CXXFLAGS +=               \
	'-DCOMMON_NIXPKGS_URL="$(COMMON_NIXPKGS_URL)"'

src/buildenv/realise.o: CXXFLAGS +=               \
	'-DFLOX_BASH_BIN="$(FLOX_BASH_BIN)"'

# ---------------------------------------------------------------------------- #

#: Remove build artifacts
CLEANFILES += $(call rwildcard,,gmon.out *.log *.gcno *.gcda *.gcov)
CLEANFILES += $(call rwildcard,,result)
clean: FORCE
	-$(RM) $(CLEANFILES);
	-$(RM) -r $(CLEANDIRS);


# For local installs clean install dir.
ifeq ($(PREFIX),$(PKGDB_ROOT)/build)
FULLCLEANDIRS += $(PREFIX)
endif # ifeq ($(PREFIX),$(PKGDB_ROOT)/build)

#: Remove build artifacts and metadata files
fullclean: clean
	-$(RM) $(FULLCLEANFILES);
	-$(RM) -r $(FULLCLEANDIRS);


# ---------------------------------------------------------------------------- #

# Install Targets
# ---------------

.PHONY: install install-bin install-lib

#: Install binaries, libraries, and include files
install: install-bin install-lib

$(BINDIR)/%: bin/%
	$(MKDIR_P) $(@D);
	if $(TEST) -f $<; then $(CP) -- "$<" "$@"; fi

#: Install binaries
install-bin: $(addprefix $(BINDIR)/,$(patsubst bin/%,%,$(BINS)))

$(LIBDIR)/%: lib/%
	$(MKDIR_P) $(@D);
	if $(TEST) -f $<; then $(CP) -- "$<" "$@"; fi

#: Install libraries
install-lib: $(addprefix $(LIBDIR)/,$(patsubst lib/%,%,$(LIBS)))


# ---------------------------------------------------------------------------- #

# The nix builder deletes many of these files and they aren't used inside of
# the nix build environment.
# We need to ensure that these files exist nonetheless to satisfy prerequisites.
$(DEPFILES): %:
	if ! $(TEST) -e $<; then $(TOUCH) $@; fi


# ---------------------------------------------------------------------------- #

# Create pre-compiled-headers specifically so that we can force our headers
# to appear in `compile_commands.json'.
# We don't actually use these in our build.
.PHONY: pre-compiled-headers clean-pch

PRE_COMPILED_HEADERS = $(patsubst %,%.gch,$(HEADERS))
CLEANFILES += $(PRE_COMPILED_HEADERS)

$(PRE_COMPILED_HEADERS): $(HEADERS) $(DEPFILES)
$(PRE_COMPILED_HEADERS): $(lastword $(MAKEFILE_LIST))
$(PRE_COMPILED_HEADERS): %.gch: %
	$(CXX) $(CXXFLAGS) -x c++-header -c $< -o $@ 2>/dev/null;

#: Create pre-compiled-headers
pre-compiled-headers: $(PRE_COMPILED_HEADERS)

#: Remove all `pre-compiled-headers'.
# This is used when creating our compilation databases to ensure that
# pre-compiled headers aren't taking priority over _real_ headers.
clean-pch: FORCE
	$(RM) $(PRE_COMPILED_HEADERS);


# ---------------------------------------------------------------------------- #

# Create `.ccls' file used by CCLS LSP as a fallback when a file is undefined
# in `compile_commands.json'.
# This will be ignored by other LSPs such as `clangd'.

.PHONY: ccls
#: Create `.ccls' file used by CCLS LSP
ccls: ../.ccls

../.ccls: $(lastword $(MAKEFILE_LIST)) $(DEPFILES)
	@echo '%compile_commands.json' > "$@";
	{                                                                     \
	  if $(TEST) -n "$(NIX_CC)"; then                                     \
	    $(CAT) "$(NIX_CC)/nix-support/libc-cflags";                       \
	    $(CAT) "$(NIX_CC)/nix-support/libcxx-cxxflags";                   \
	  fi;                                                                 \
	  echo $(CXXFLAGS) $(nljson_CFLAGS) $(nix_CFLAGS);                    \
	  echo $(argparse_CFLAGS) $(sqlite3pp_CFLAGS);                        \
	  echo '-DTEST_DATA_DIR="$(TEST_DATA_DIR)"';                          \
	}|$(TR) ' ' '\n'|$(SED) 's/-std=\(.*\)/%cpp -std=\1|%h -std=\1/'      \
	 |$(TR) '|' '\n' >> "$@";

FULLCLEANFILES += ../.ccls


# ---------------------------------------------------------------------------- #

# Create `compile_commands.json' file used by LSPs.

# Get system include paths from `nix' C++ compiler.
# Filter out framework directory, e.g.
# /nix/store/q2d0ya7rc5kmwbwvsqc2djvv88izn1q6-apple-framework-CoreFoundation-11.0.0/Library/Frameworks (framework directory)
# We might be able to strip '(framework directory)' instead and append
# CoreFoundation.framework/Headers but I don't think we need to.
_CXX_SYSTEM_INCDIRS := $(shell                                \
  $(CXX) -E -Wp,-v -xc++ /dev/null 2>&1 1>/dev/null           \
  |$(GREP) -v 'framework directory'|$(GREP) '^ /nix/store')
_CXX_SYSTEM_INCDIRS := $(patsubst %,-isystem %,$(_CXX_SYSTEM_INCDIRS))

BEAR_WRAPPER := $(dir $(shell command -v $(BEAR)))
BEAR_WRAPPER := $(dir $(patsubst %/,%,$(BEAR_WRAPPER)))lib/bear/wrapper

bear.d/c++:
	$(MKDIR_P) $(@D);
	$(LN) -s $(BEAR_WRAPPER) bear.d/c++;

FULLCLEANDIRS += bear.d

../compile_commands.json: EXTRA_CXXFLAGS += $(_CXX_SYSTEM_INCDIRS)
../compile_commands.json: bear.d/c++ $(DEPFILES)
../compile_commands.json: $(lastword $(MAKEFILE_LIST))
../compile_commands.json: $(HEADERS) $(ALL_SRCS)
	-$(MAKE) -C $(MAKEFILE_DIR) clean;
	EXTRA_CXXFLAGS='$(EXTRA_CXXFLAGS)'                              \
	  PATH="$(MAKEFILE_DIR)/bear.d/:$(PATH)"                        \
	  $(BEAR) --output $@ -- $(MAKE) -C $(MAKEFILE_DIR) bin tests;
	EXTRA_CXXFLAGS='$(EXTRA_CXXFLAGS)'                    \
	  PATH="$(MAKEFILE_DIR)/bear.d/:$(PATH)"              \
	  $(BEAR) --output $@ --append --                     \
	    $(MAKE) -C $(MAKEFILE_DIR) pre-compiled-headers;
	$(MAKE) -C $(MAKEFILE_DIR) clean-pch;

FULLCLEANFILES += ../compile_commands.json


# ---------------------------------------------------------------------------- #

# LSP Metadata
# ------------

.PHONY: compilation-databases cdb
#: Create `compile_commands.json' and `ccls' file used for LSPs
compilation-databases: ../compile_commands.json ccls
#: Create `compile_commands.json' and `ccls' file used for LSPs
cdb: compilation-databases


# ---------------------------------------------------------------------------- #

# Run `include-what-you-use' ( wrapped )
.PHONY: iwyu
#: Generate `include-what-you-use' report
iwyu: iwyu.log

iwyu.log: compile_commands.json $(HEADERS) $(ALL_SRCS) flake.nix
iwyu.log: flake.lock pkg-fun.nix pkgs/nlohmann_json.nix pkgs/nix/pkg-fun.nix
iwyu.log: build-aux/iwyu build-aux/iwyu-mappings.json
	build-aux/iwyu|$(TEE) "$@";

FULLCLEANFILES += iwyu.log


# ---------------------------------------------------------------------------- #

.PHONY: lint
#: Run `clang-tidy' across entire project
lint: ../compile_commands.json $(HEADERS) $(ALL_SRCS)
	$(TIDY) $(filter-out ../compile_commands.json,$^);


# ---------------------------------------------------------------------------- #

.PHONY: check cc-check bats-check

#: Run all tests
check: cc-check bats-check

#: Run all C++ unit tests
cc-check: $(TESTS:.cc=)
	@_ec=0;                     \
	echo '';                    \
	for t in $(TESTS:.cc=); do  \
	  echo "Testing: $$t";      \
	  if "./$$t"; then          \
	    echo "PASS: $$t";       \
	  else                      \
	    _ec=1;                  \
	    echo "FAIL: $$t";       \
	  fi;                       \
	  echo '';                  \
	done;                       \
	exit "$$_ec";

#: Run all bats tests
PKGDB_BATS_FILES ?= "$(PKGDB_ROOT)/tests"
PKGDB_BATS_OPTS ?= ""
bats-check: bin $(TEST_UTILS)
	PKGDB_BIN="$(PKGDB_ROOT)/bin/pkgdb"                          \
	PKGDB_IS_SQLITE3_BIN="$(PKGDB_ROOT)/tests/is_sqlite3"        \
	PKGDB_SEARCH_PARAMS_BIN="$(PKGDB_ROOT)/tests/search-params"  \
	  $(BATS) --print-output-on-failure --verbose-run --timing   \
	          $(PKGDB_BATS_OPTS) "$(PKGDB_BATS_FILES)";


# ---------------------------------------------------------------------------- #

# Static Asset Headers
# --------------------
# We include scripts for SQL, Bash, and some text files as static strings in
# our binary.
# The following targets/recipes generate the `.hh' files ( if needed ), and
# declare prerequisite rules explicitly for files that use them.
#
# NOTE: These are not included in `HEADERS' because they are not interface
#       declarations of "headers" in the conventional sense.

# SQL Schemas
src/pkgdb/write.o: src/pkgdb/schemas.hh

src/pkgdb/scrape-rules.o: src/pkgdb/rules.json.hh

# ---------------------------------------------------------------------------- #

# Make all `.o' files depend on all `include/**/*.hh' files.
$(ALL_SRCS:.cc=.o): %.o: %.cc $(HEADERS)


# ---------------------------------------------------------------------------- #

# Build Targets

$(BINS): %: $(SRCS:.cc=.o)
	$(MKDIR_P) $(@D);
	$(CXX) $(filter %.o,$^) $(LDFLAGS) -o $@;

lib/ld-floxlib.so: src/ld-floxlib.c
	$(MKDIR_P) $(@D);
	$(CC) -shared -fPIC $< -o $@;
	patchelf --remove-rpath $@;


# ---------------------------------------------------------------------------- #

$(test_SRCS:.cc=.o): CXXFLAGS += '-DTEST_DATA_DIR="$(TEST_DATA_DIR)"'
$(TESTS) $(TEST_UTILS): tests/%: $(filter-out src/main.o,$(SRCS:.cc=.o))
$(TESTS) $(TEST_UTILS): tests/%: tests/%.o tests/test.hh
	$(CXX) $(filter %.o,$^) $(LDFLAGS) -o $@;

# ---------------------------------------------------------------------------- #

# Standard Targets
# ----------------

.PHONY: bin tests

#: Build binaries
bin: $(BINS)

#: Build libs
lib: $(ALL_LIBS)

#: Build test executables and resources
tests: $(TESTS) $(TEST_UTILS)

#: Build binaries, tests, libs, and generated `.gitignore' files
all: bin lib tests ignores

#: Build binaries, libs, and generated `.gitignore' files
most: bin lib ignores


# ---------------------------------------------------------------------------- #

.PHONY: docs

#: Generate documentation
docs: docs/index.html

docs/index.html: FORCE
	$(DOXYGEN) ./Doxyfile

CLEANFILES += $(addprefix docs/,*.png *.html *.svg *.css *.js)
CLEANDIRS  += docs/search


# ---------------------------------------------------------------------------- #

#: Generate `.gitignore' files for
ignores: tests/.gitignore
tests/.gitignore: FORCE
	$(MKDIR_P) $(@D);
	@echo 'Generating $@' >&2;
	@printf '%s\n' $(patsubst tests/%,%,$(test_SRCS:.cc=)) > $@;


# ---------------------------------------------------------------------------- #

# Formatter
# ---------

.PHONY: fmt
#: Run `clang-format' across entire project
fmt: $(HEADERS) $(ALL_SRCS)
	$(FMT) -i $^;


# ---------------------------------------------------------------------------- #

# Generated Files
# ---------------

src/fetchers/flake-v0.nix.in.hh: src/fetchers/flake-v0.nix.in Makefile
	$(MKDIR_P) $(@D);
	echo 'R"_NIX(' > $@;
	$(CAT) $< >> $@;
	echo ')_NIX";' >> $@;

CLEANFILES += src/fetchers/flake-v0.nix.in.hh
src/fetchers/wrapped-nixpkgs-input.o: src/fetchers/flake-v0.nix.in.hh


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
