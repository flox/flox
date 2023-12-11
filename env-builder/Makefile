# ============================================================================ #
#
#
#
# ---------------------------------------------------------------------------- #

MAKEFILE_DIR ?= $(patsubst %/,%,$(dir $(abspath $(lastword $(MAKEFILE_LIST)))))

include $(MAKEFILE_DIR)/Include.mk

# ---------------------------------------------------------------------------- #

.PHONY: all clean fullclean FORCE ignores most
.DEFAULT_GOAL = most


# ---------------------------------------------------------------------------- #

include $(MK_DIR)/utils.mk
include $(MK_DIR)/platform.mk
include $(MK_DIR)/nix.mk


# ---------------------------------------------------------------------------- #

VERSION := $(file < $(ENV_BUILDER_ROOT)/version)


# ---------------------------------------------------------------------------- #

# Our shared library target
LIBFLOXENVBUILDER = libenvbuilder$(libExt)

# Various file and target lists

LIBS           = $(LIBFLOXENVBUILDER)
HEADERS = $(call rwildcard,include,*.hh)
SRCS           = $(call rwildcard,src,*.cc)
bin_SRCS       = src/main.cc src/help.cc
lib_SRCS       = $(filter-out $(bin_SRCS),$(SRCS))
BINS           = env-builder
CLEANDIRS	     =
CLEANFILES	   = $(SRCS:.cc=.o)
CLEANFILES     += $(addprefix bin/,$(BINS)) $(addprefix lib/,$(LIBS))
FULLCLEANDIRS  =
FULLCLEANFILES =

# Where to find test suite input data files.
TEST_DATA_DIR = $(ENV_BUILDER_ROOT)/tests/fixtures


# ---------------------------------------------------------------------------- #

# Compiler Flags
# --------------

# You can disable these optional gripes with `make EXTRA_CXXFLAGS='' ...;'
ifndef EXTRA_CXXFLAGS

EXTRA_CXXFLAGS = -Wall -Wextra -Wpedantic

# Clang only
ifeq (clang,$(TOOLCHAIN))
EXTRA_CXXFLAGS += -Wno-gnu-zero-variadic-macro-arguments
endif  # ifneq (clang,$(TOOLCHAIN))

endif	# ifndef EXTRA_CXXFLAGS


CXXFLAGS ?= $(EXTRA_CFLAGS) $(EXTRA_CXXFLAGS)
CXXFLAGS += '-I$(ENV_BUILDER_ROOT)/include'
CXXFLAGS += '-DFLOX_ENV_BUILDER_VERSION="$(VERSION)"'
CXXFLAGS += -D'PROFILE_D_SCRIPT_DIR="$(PROFILE_D_SCRIPT_DIR)"'
CXXFLAGS += -D'SET_PROMPT_BASH_SH="$(SET_PROMPT_BASH_SH)"'
LDFLAGS  ?= $(EXTRA_LDFLAGS)


ifeq (gcc,$(TOOLCHAIN))
lib_CXXFLAGS ?= -shared -fPIC
lib_LDFLAGS  ?= -shared -fPIC -Wl,--no-undefined
else # Clang
lib_CXXFLAGS ?= -fPIC
lib_LDFLAGS  ?= -shared -fPIC -Wl,-undefined,error
endif # ifeq (gcc,$(TOOLCHAIN))


bin_CXXFLAGS ?=
bin_LDFLAGS  ?=

# Debug Mode
ifneq ($(DEBUG),)
ifeq (gcc,$(TOOLCHAIN))
CXXFLAGS += -ggdb3 -pg -fno-omit-frame-pointer
LDFLAGS  += -ggdb3 -pg -fno-omit-frame-pointer
else # Clang
CXXFLAGS += -g -pg
LDFLAGS  += -g -pg
endif # ifeq (gcc,$(TOOLCHAIN))
endif # ifneq ($(DEBUG),)

# Coverage Mode
ifneq ($(COV),)
CXXFLAGS += --coverage -fprofile-arcs -ftest-coverage
LDFLAGS  += --coverage -fprofile-arcs -ftest-coverage
endif # ifneq ($(COV),)


# ---------------------------------------------------------------------------- #

# Dependency Flags
# ----------------

sqlite3pp_CFLAGS ?=                                                     \
	$(patsubst -I%,-isystem %,$(shell $(PKG_CONFIG) --cflags sqlite3pp))
sqlite3pp_CFLAGS := $(sqlite3pp_CFLAGS)


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

ifndef pkgdb_CFLAGS
# Try `pkg-config'
pkgdb_CFLAGS = $(shell $(PKG_CONFIG) --cflags pkgdb||:)
# If `pkg-config' fails, try local path with manual includes.
ifeq (,$(pkgdb_CFLAGS))
pkgdb_CFLAGS =  -isystem $(REPO_ROOT)/pkgdb/include
pkgdb_CFLAGS += $(sqlite3pp_CFLAGS)
pkgdb_CFLAGS += -isystem $(nix_INCDIR) -include $(nix_INCDIR)/nix/config.h
endif # ifeq (,$(pkgdb_CFLAGS))
endif # ifndef pkgdb_CFLAGS

ifndef pkgdb_LDFLAGS
# Try `pkg-config'
pkgdb_LDFLAGS = $(shell $(PKG_CONFIG) --libs pkgdb||:)
# If `pkg-config' fails, try local path with manual linkage.
ifeq (,$(pkgdb_LDFLAGS))
# Add recipe to build libpkgdb
$(REPO_ROOT)/pkgdb/lib/libpkgdb$(libExt): $(REPO_ROOT)/pkgdb/Makefile
$(REPO_ROOT)/pkgdb/lib/libpkgdb$(libExt): $(call rwildcard,$(REPO_ROOT)/pkgdb/include,*.hh)
$(REPO_ROOT)/pkgdb/lib/libpkgdb$(libExt): $(call rwildcard,$(REPO_ROOT)/pkgdb/src,*.cc *.hh)
	$(MAKE) -C $(REPO_ROOT)/pkgdb lib/libpkgdb$(libExt)
lib/$(LIBENVBUILDER): $(REPO_ROOT)/pkgdb/lib/libpkgdb$(libExt)

pkgdb_LDFLAGS = '-L$(REPO_ROOT)/pkgdb/lib'
ifeq (Linux,$(OS))
pkgdb_LDFLAGS += -Wl,--enable-new-dtags '-Wl,-rpath,$$ORIGIN/../../pkgdb/lib'
else  # Darwin
ifeq "$(findstring install,$(MAKECMDGOALS))" ""
pkgdb_LDFLAGS += -rpath @executable_path/../../pkgdb/lib
endif # ifeq $(,$(findstring install,$(MAKECMDGOALS)))
endif # ifeq (Linux,$(OS))
pkgdb_LDFLAGS += -lpkgdb
pkgdb_LDFLAGS += $(shell $(PKG_CONFIG) --libs-only-L nix-main) -lnixfetchers
endif # ifeq (,$(pkgdb_LDFLAGS))
endif # ifndef pkgdb_LDFLAGS


ifndef libenvbuilder_LDFLAGS
libenvbuilder_LDFLAGS = '-L$(ENV_BUILDER_ROOT)/lib'
ifeq (Linux,$(OS))
libenvbuilder_LDFLAGS += -Wl,--enable-new-dtags '-Wl,-rpath,$$ORIGIN/../lib'
else  # Darwin
ifneq "$(findstring install,$(MAKECMDGOALS))" ""
libenvbuilder_LDFLAGS = '-L$(LIBDIR)'
else
libenvbuilder_LDFLAGS += '-L$(ENV_BUILDER_ROOT)/lib'
libenvbuilder_LDFLAGS += -rpath @executable_path/../lib
endif # ifneq $(,$(findstring install,$(MAKECMDGOALS)))
endif # ifeq (Linux,$(OS))
libenvbuilder_LDFLAGS += -lenvbuilder
endif # ifndef libenvbuilder_LDFLAGS


# ---------------------------------------------------------------------------- #

CXXFLAGS += $(pkgdb_CFLAGS) $(nix_CFLAGS)
LDFLAGS  += $(pkgdb_LDFLAGS) $(nix_LDFLAGS)

bin_LDFLAGS += $(libenvbuilder_LDFLAGS)


# ---------------------------------------------------------------------------- #

# Standard Targets
# ----------------

.PHONY: bin lib include tests

#: Build binaries
bin:     lib $(addprefix bin/,$(BINS))
#: Build libraries
lib:     $(addprefix lib/,$(LIBS))
#: Build/generate include files
include: $(HEADERS)
#: Build test executables and resources
tests:   $(TESTS) $(TEST_UTILS)


# ---------------------------------------------------------------------------- #

#: Remove build artifacts
clean: FORCE
	-$(RM) $(CLEANFILES);
	-$(RM) -r $(CLEANDIRS);
	-$(RM) result;
	-$(RM) **/gmon.out gmon.out **/*.log *.log;
	-$(RM) **/*.gcno *.gcno **/*.gcda *.gcda **/*.gcov *.gcov;


#: Remove build artifacts and metadata files
fullclean: clean
	-$(RM) $(FULLCLEANFILES);
	-$(RM) -r $(FULLCLEANDIRS);


# ---------------------------------------------------------------------------- #

%.o: %.cc $(HEADERS)
	$(CXX) $(CXXFLAGS) -c $< -o $@;

$(addprefix lib/,$(LIBS)): %: LDFLAGS  += $(lib_LDFLAGS)
$(addprefix lib/,$(LIBS)): %: CXXFLAGS += $(lib_CXXFLAGS)
$(addprefix lib/,$(LIBS)): %: $(lib_SRCS:.cc=.o)
	$(MKDIR_P) $(@D);
	$(CXX) $(filter %.o,$^) $(LDFLAGS) $(call set_binary_name,$(@F)) -o $@;


# ---------------------------------------------------------------------------- #

$(bin_SRCS:.cc=.o): %.o: %.cc $(HEADERS)
	$(CXX) $(CXXFLAGS) $(bin_CXXFLAGS) -c $< -o $@;

$(addprefix bin/,$(BINS)): %: $(bin_SRCS:.cc=.o) lib/$(LIBFLOXENVBUILDER)
	$(MKDIR_P) $(@D);
	$(CXX) $(filter %.o,$^) $(LDFLAGS) $(bin_LDFLAGS) -o $@;


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

$(PRE_COMPILED_HEADERS): CXXFLAGS += $(lib_CXXFLAGS) $(bin_CXXFLAGS)
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
ccls: .ccls

.ccls: $(lastword $(MAKEFILE_LIST)) $(DEPFILES)
	@echo '%compile_commands.json' > "$@";
	@_std='-std=$(call get_cc_std,$(CXXFLAGS) $(pkgdb_CFLAGS))';  \
	 echo "%h $$_std" >> $@;                                      \
	 echo "%cpp $$_std" >> $@;
	@printf '%s\n' $(CXXFLAGS) $(pkgdb_CFLAGS)                                 \
	               $(NIX_CC_WRAPPER_CFLAGS) $(NIX_CC_WRAPPER_CXXFLAGS) >> $@;

FULLCLEANFILES += .ccls


# ---------------------------------------------------------------------------- #

# Create `compile_commands.json' file used by LSPs.

BEAR_WRAPPER := $(dir $(shell command -v $(BEAR)))
BEAR_WRAPPER := $(dir $(patsubst %/,%,$(BEAR_WRAPPER)))lib/bear/wrapper

bear.d/c++:
	$(MKDIR_P) $(@D);
	$(LN) -s $(BEAR_WRAPPER) bear.d/c++;

FULLCLEANDIRS += bear.d

compile_commands.json: EXTRA_CXXFLAGS += $(_CXX_SYSTEM_INCDIRS)
compile_commands.json: bear.d/c++ $(DEPFILES)
compile_commands.json: $(lastword $(MAKEFILE_LIST))
compile_commands.json: $(HEADERS) $(SRCS)
	-$(MAKE) -C $(ENV_BUILDER_ROOT) clean;
	EXTRA_CXXFLAGS='$(EXTRA_CXXFLAGS)'                      \
	  PATH="$(ENV_BUILDER_ROOT)/bear.d/:$(PATH)"            \
	  $(BEAR) -- $(MAKE) -C $(ENV_BUILDER_ROOT) bin tests;
	EXTRA_CXXFLAGS='$(EXTRA_CXXFLAGS)'                                          \
	  PATH="$(ENV_BUILDER_ROOT)/bear.d/:$(PATH)"                                \
	  $(BEAR) --append -- $(MAKE) -C $(ENV_BUILDER_ROOT) pre-compiled-headers;
	$(MAKE) -C $(ENV_BUILDER_ROOT) clean-pch;

FULLCLEANFILES += compile_commands.json


# ---------------------------------------------------------------------------- #

# LSP Metadata
# ------------

.PHONY: compilation-databases cdb
#: Create `compile_commands.json' and `ccls' file used for LSPs
compilation-databases: compile_commands.json ccls
#: Create `compile_commands.json' and `ccls' file used for LSPs
cdb: compilation-databases


# ---------------------------------------------------------------------------- #

# TODO

## # Run `include-what-you-use' ( wrapped )
## .PHONY: iwyu
## #: Generate `include-what-you-use' report
## iwyu: iwyu.log
##
## iwyu.log: compile_commands.json $(HEADERS) $(SRCS) flake.nix
## iwyu.log: flake.lock pkg-fun.nix pkgs/nlohmann_json.nix pkgs/nix/pkg-fun.nix
## iwyu.log: build-aux/iwyu build-aux/iwyu-mappings.json
## 	build-aux/iwyu|$(TEE) "$@";
##
## FULLCLEANFILES += iwyu.log


# ---------------------------------------------------------------------------- #

.PHONY: lint
#: Run `clang-tidy' across entire project
lint: compile_commands.json $(HEADERS) $(SRCS)
	$(TIDY) $(filter-out compile_commands.json,$^);


# ---------------------------------------------------------------------------- #

.PHONY: check bats-check

#: Run all tests
check: bats-check

#: Run all bats tests
BATS_FILE ?= $(ENV_BUILDER_ROOT)/tests
bats-check: bin $(TEST_UTILS)
	PKGDB="$(ENV_BUILDER_ROOT)/bin/pkgdb"                    \
	IS_SQLITE3="$(ENV_BUILDER_ROOT)/tests/is_sqlite3"        \
	SEARCH_PARAMS="$(ENV_BUILDER_ROOT)/tests/search-params"  \
	  bats --print-output-on-failure --verbose-run --timing  \
	       "$(BATS_FILE)";


# ---------------------------------------------------------------------------- #

#: Build binaries, libraries, tests, and generated `.gitignore' files
all: bin lib tests ignores

#: Build binaries, libraries, and generated `.gitignore' files
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

# TODO

# Generate `pkg-config' file.
# ---------------------------
# The `PC_CFLAGS' and `PC_LIBS' variables carry flags that are not covered by
# `nlohmann_json`, `argparse`, `sqlite3pp`, `sqlite`, and `nix{main,cmd,expr}`
# `Requires' handling.
# This amounts to handling `boost', `libnixfetchers', forcing
# the inclusion of the `nix' `config.h' header, and some additional CPP vars.

## PC_CFLAGS =  $(filter -std=%,$(CXXFLAGS))
## PC_CFLAGS += $(boost_CFLAGS)
## PC_CFLAGS += $(sqlite3pp_CFLAGS)
## PC_CFLAGS += -isystem $(nix_INCDIR) -include $(nix_INCDIR)/nix/config.h
## PC_CFLAGS += '-DFLOX_PKGDB_VERSION=\\\\\"$(VERSION)\\\\\"'
## PC_CFLAGS += '-DSEMVER_PATH=\\\\\"$(SEMVER_PATH)\\\\\"'
## PC_LIBS   =  $(shell $(PKG_CONFIG) --libs-only-L nix-main) -lnixfetchers
## lib/pkgconfig/flox-pkgdb.pc: $(lastword $(MAKEFILE_LIST)) $(DEPFILES)
## lib/pkgconfig/flox-pkgdb.pc: lib/pkgconfig/flox-pkgdb.pc.in version
## 	$(SED) -e 's,@PREFIX@,$(PREFIX),g'      \
## 	       -e 's,@VERSION@,$(VERSION),g'    \
## 	       -e 's,@CFLAGS@,$(PC_CFLAGS),g'   \
## 	       -e 's,@LIBS@,$(PC_LIBS),g'       \
## 	       $< > $@;
##
## CLEANFILES += lib/pkgconfig/flox-pkgdb.pc
##
## install-lib: $(LIBDIR)/pkgconfig/flox-pkgdb.pc


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
fmt: $(HEADERS) $(SRCS)
	$(FMT) -i $^;


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
