# ============================================================================ #
#
# Target/Task Highlights:
#   - most (default)        Build binaries, libs, and generated files
#   - all                   Build binaries, libs, tests, and generated files
#
#   - bin                   Build binaries
#   - tests                 Build test executables and resources
#   - lib                   Build libraries
#   - include               Build/generate include files
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
#   - install               Install binaries, libraries, and include files
#   - install-bin           Install binaries
#   - install-dirs          Create directories in the install prefix
#   - install-include       Install include files
#
#   - ccls                  Create `.ccls' file used by CCLS LSP
#   - compile_commands.json Create `compile_commands.json' file used for LSPs
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

MAKEFILE_DIR ?= $(patsubst %/,%,$(dir $(abspath $(lastword $(MAKEFILE_LIST)))))

# ---------------------------------------------------------------------------- #

.PHONY: all clean fullclean FORCE ignores most
.DEFAULT_GOAL = most


# ---------------------------------------------------------------------------- #

# Utilities
# ---------
# We use variables to refer to all tools so that we can easily override them
# from the command line.

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
NIX        ?= nix
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

VERSION := $(file < $(MAKEFILE_DIR)/version)


# ---------------------------------------------------------------------------- #

# Install Prefixes
# ----------------

PREFIX     ?= $(MAKEFILE_DIR)/out
BINDIR     ?= $(PREFIX)/bin
LIBDIR     ?= $(PREFIX)/lib
INCLUDEDIR ?= $(PREFIX)/include


# ---------------------------------------------------------------------------- #

# rwildcard DIRS, PATTERNS
# ------------------------
# Recursive wildcard.
#   Ex:  $(call rwildcard,src,*.cc *.hh)
rwildcard = $(foreach d,$(wildcard $(1:=/*)),$(call rwildcard,$d,$2)        \
                                             $(filter $(subst *,%,$2),$d))


# ---------------------------------------------------------------------------- #

# Our shared library target
LIBFLOXPKGDB = libflox-pkgdb$(libExt)

# Various file and target lists

LIBS           =  $(LIBFLOXPKGDB)
COMMON_HEADERS =  $(call rwildcard,include,*.hh)
SRCS           =  $(call rwildcard,src,*.cc)
bin_SRCS       =  src/main.cc src/repl.cc
bin_SRCS       += $(addprefix src/pkgdb/,scrape.cc get.cc command.cc)
bin_SRCS       += $(addprefix src/search/,command.cc)
bin_SRCS       += $(addprefix src/resolver/,command.cc)
bin_SRCS       += $(addprefix src/parse/,command.cc)
lib_SRCS       =  $(filter-out $(bin_SRCS),$(SRCS))
test_SRCS      =  $(sort $(wildcard tests/*.cc))
ALL_SRCS       = $(SRCS) $(test_SRCS)
BINS           =  pkgdb
TEST_UTILS     =  $(addprefix tests/,is_sqlite3 search-params)
TESTS          =  $(filter-out $(TEST_UTILS),$(test_SRCS:.cc=))
CLEANDIRS      =
CLEANFILES     =  $(ALL_SRCS:.cc=.o)
CLEANFILES     += $(addprefix bin/,$(BINS)) $(addprefix lib/,$(LIBS))
CLEANFILES     += $(TESTS) $(TEST_UTILS)
FULLCLEANDIRS  =
FULLCLEANFILES =

# Where to find test suite input data files.
TEST_DATA_DIR = $(MAKEFILE_DIR)/tests/data


# ---------------------------------------------------------------------------- #

# Files which effect dependencies, external inputs, and `*FLAGS' values.
DEPFILES =  flake.nix flake.lock pkg-fun.nix pkgs/nlohmann_json.nix
DEPFILES += pkgs/nix/pkg-fun.nix


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
CXXFLAGS += '-I$(MAKEFILE_DIR)/include'
CXXFLAGS += '-DFLOX_PKGDB_VERSION="$(VERSION)"'
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
CXXFLAGS += -ggdb3 -pg
LDFLAGS  += -ggdb3 -pg
else # Clang
CXXFLAGS += -g -pg
LDFLAGS  += -g -pg
endif # ifeq (gcc,$(TOOLCHAIN))
endif # ifneq ($(DEBUG),)

# Coverage Mode
ifneq ($(COV),)
CXXFLAGS += -fprofile-arcs -ftest-coverage
LDFLAGS  += -fprofile-arcs -ftest-coverage
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

sqlite3_CFLAGS ?=                                                     \
	$(patsubst -I%,-isystem %,$(shell $(PKG_CONFIG) --cflags sqlite3))
sqlite3_CFLAGS  := $(sqlite3_CFLAGS)
sqlite3_LDFLAGS ?= $(shell $(PKG_CONFIG) --libs sqlite3)
sqlite3_LDLAGS  := $(sqlite3_LDLAGS)

sqlite3pp_CFLAGS ?=                                                     \
	$(patsubst -I%,-isystem %,$(shell $(PKG_CONFIG) --cflags sqlite3pp))
sqlite3pp_CFLAGS := $(sqlite3pp_CFLAGS)

yaml_PREFIX ?=                                                          \
	$(shell $(NIX) build --no-link --print-out-paths 'nixpkgs#yaml-cpp')
yaml_PREFIX := $(yaml_PREFIX)
yaml_CFLAGS  = -isystem $(yaml_PREFIX)/include
yaml_LDFLAGS = -L$(yaml_PREFIX)/lib -lyaml-cpp

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

ifndef flox_pkgdb_LDFLAGS
ifeq (Linux,$(OS))
flox_pkgdb_LDFLAGS = -Wl,--enable-new-dtags '-Wl,-rpath,$$ORIGIN/../lib'
flox_pkgdb_LDFLAGS += '-L$(MAKEFILE_DIR)/lib' -lflox-pkgdb
else  # Darwin
ifneq "$(findstring install,$(MAKECMDGOALS))" ""
flox_pkgdb_LDFLAGS = '-L$(LIBDIR)'
else
flox_pkgdb_LDFLAGS += '-L$(MAKEFILE_DIR)/lib' -rpath @executable_path/../lib
endif # ifneq $(,$(findstring install,$(MAKECMDGOALS)))
flox_pkgdb_LDFLAGS += -lflox-pkgdb
endif # ifeq (Linux,$(OS))
endif # ifndef flox_pkgdb_LDFLAGS


# ---------------------------------------------------------------------------- #

lib_CXXFLAGS += $(sqlite3pp_CFLAGS)
bin_CXXFLAGS += $(argparse_CFLAGS)
CXXFLAGS     += $(nix_CFLAGS) $(nljson_CFLAGS) $(toml_CFLAGS) $(yaml_CFLAGS)

ifeq (gcc,$(TOOLCHAIN))
lib_LDFLAGS += -Wl,--as-needed
endif # ifeq (gcc,$(TOOLCHAIN))

lib_LDFLAGS += $(nix_LDFLAGS) $(sqlite3_LDFLAGS)

ifeq (gcc,$(TOOLCHAIN))
lib_LDFLAGS += -Wl,--no-as-needed
endif # ifeq (gcc,$(TOOLCHAIN))

bin_LDFLAGS += $(nix_LDFLAGS) $(flox_pkgdb_LDFLAGS) $(sqlite3_LDFLAGS)
LDFLAGS     += $(yaml_LDFLAGS)


# ---------------------------------------------------------------------------- #

# Locate `semver'
# ---------------

SEMVER_PATH ?=                                                        \
  $(shell $(NIX) build --no-link --print-out-paths                    \
	                     'github:aakropotkin/floco#semver')/bin/semver
CXXFLAGS += '-DSEMVER_PATH="$(SEMVER_PATH)"'


# ---------------------------------------------------------------------------- #

# Standard Targets
# ----------------

.PHONY: bin lib include tests

#: Build binaries
bin:     lib $(addprefix bin/,$(BINS))
#: Build libraries
lib:     $(addprefix lib/,$(LIBS))
#: Build/generate include files
include: $(COMMON_HEADERS)
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

%.o: %.cc $(COMMON_HEADERS)
	$(CXX) $(CXXFLAGS) -c $< -o $@;

ifeq (Linux,$(OS))
SONAME_FLAG = -Wl,-soname,$(LIBFLOXPKGDB)
else
SONAME_FLAG =
endif

ifneq (Linux,$(OS))
LINK_INAME_FLAG = -install_name '@rpath/$(LIBFLOXPKGDB)'
lib/$(LIBFLOXPKGDB): LDFLAGS += $(LINK_INAME_FLAG)
endif # ifneq (Linux,$(OS))
lib/$(LIBFLOXPKGDB): LDFLAGS  += $(lib_LDFLAGS)
lib/$(LIBFLOXPKGDB): CXXFLAGS += $(lib_CXXFLAGS)
lib/$(LIBFLOXPKGDB): $(lib_SRCS:.cc=.o)
	$(MKDIR_P) $(@D);
	$(CXX) $(filter %.o,$^) $(LDFLAGS) $(SONAME_FLAG) -o $@;


# ---------------------------------------------------------------------------- #

src/pkgdb/write.o: src/pkgdb/schemas.hh

$(bin_SRCS:.cc=.o): %.o: %.cc $(COMMON_HEADERS)
	$(CXX) $(CXXFLAGS) $(bin_CXXFLAGS) -c $< -o $@;

bin/pkgdb: $(bin_SRCS:.cc=.o) lib/$(LIBFLOXPKGDB)
	$(MKDIR_P) $(@D);
	$(CXX) $(filter %.o,$^) $(LDFLAGS) $(bin_LDFLAGS) -o $@;


# ---------------------------------------------------------------------------- #

$(TESTS) $(TEST_UTILS): $(COMMON_HEADERS)
$(TESTS) $(TEST_UTILS): bin_CXXFLAGS += '-DTEST_DATA_DIR="$(TEST_DATA_DIR)"'
$(TESTS) $(TEST_UTILS): tests/%: tests/%.cc lib/$(LIBFLOXPKGDB)
	$(CXX) $(CXXFLAGS) $(bin_CXXFLAGS) $< $(LDFLAGS) $(bin_LDFLAGS) -o $@;


# ---------------------------------------------------------------------------- #

# Install Targets
# ---------------

.PHONY: install-dirs install-bin install-lib install-include install

#: Install binaries, libraries, and include files
install: install-dirs install-bin install-lib install-include

#: Create directories in the install prefix
install-dirs: FORCE
	$(MKDIR_P) $(BINDIR) $(LIBDIR) $(LIBDIR)/pkgconfig;
	$(MKDIR_P) $(INCLUDEDIR)/flox $(INCLUDEDIR)/flox/core;
	$(MKDIR_P) $(INCLUDEDIR)/flox/pkgdb $(INCLUDEDIR)/flox/search $(INCLUDEDIR)/flox/parse;
	$(MKDIR_P) $(INCLUDEDIR)/flox/resolver $(INCLUDEDIR)/compat;

$(INCLUDEDIR)/%: include/% | install-dirs
	$(CP) -- "$<" "$@";

$(LIBDIR)/%: lib/% | install-dirs
	$(CP) -- "$<" "$@";

$(BINDIR)/%: bin/% | install-dirs
	$(CP) -- "$<" "$@";

# Darwin has to relink
ifneq (Linux,$(OS))
LINK_INAME_FLAG = -install_name '@rpath/$(LIBFLOXPKGDB)'
$(LIBDIR)/$(LIBFLOXPKGDB): CXXFLAGS += $(lib_CXXFLAGS)
$(LIBDIR)/$(LIBFLOXPKGDB): LDFLAGS  += $(lib_LDFLAGS)
$(LIBDIR)/$(LIBFLOXPKGDB): $(lib_SRCS:.cc=.o)
	$(MKDIR_P) $(@D);
	$(CXX) $(filter %.o,$^) $(LDFLAGS) -o $@;

$(BINDIR)/pkgdb: $(bin_SRCS:.cc=.o) $(LIBDIR)/$(LIBFLOXPKGDB)
	$(MKDIR_P) $(@D);
	$(CXX) $(filter %.o,$^) $(LDFLAGS) $(bin_LDFLAGS) -o $@;
endif # ifneq (Linux,$(OS))

#: Install binaries
install-bin: $(addprefix $(BINDIR)/,$(BINS))

#: Install libraries
install-lib: $(addprefix $(LIBDIR)/,$(LIBS))

#: Install include files
install-include:                                                     \
	$(addprefix $(INCLUDEDIR)/,$(subst include/,,$(COMMON_HEADERS)));


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

PRE_COMPILED_HEADERS = $(patsubst %,%.gch,$(COMMON_HEADERS))
CLEANFILES += $(PRE_COMPILED_HEADERS)

$(PRE_COMPILED_HEADERS): CXXFLAGS += $(lib_CXXFLAGS) $(bin_CXXFLAGS)
$(PRE_COMPILED_HEADERS): $(COMMON_HEADERS) $(DEPFILES)
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

FULLCLEANFILES += .ccls


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

compile_commands.json: EXTRA_CXXFLAGS += $(_CXX_SYSTEM_INCDIRS)
compile_commands.json: bear.d/c++ $(DEPFILES)
compile_commands.json: $(lastword $(MAKEFILE_LIST))
compile_commands.json: $(COMMON_HEADERS) $(ALL_SRCS)
	-$(MAKE) -C $(MAKEFILE_DIR) clean;
	EXTRA_CXXFLAGS='$(EXTRA_CXXFLAGS)'                  \
	  PATH="$(MAKEFILE_DIR)/bear.d/:$(PATH)"            \
	  $(BEAR) -- $(MAKE) -C $(MAKEFILE_DIR) bin tests;
	EXTRA_CXXFLAGS='$(EXTRA_CXXFLAGS)'                                      \
	  PATH="$(MAKEFILE_DIR)/bear.d/:$(PATH)"                                \
	  $(BEAR) --append -- $(MAKE) -C $(MAKEFILE_DIR) pre-compiled-headers;
	$(MAKE) -C $(MAKEFILE_DIR) clean-pch;

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

# Run `include-what-you-use' ( wrapped )
.PHONY: iwyu
#: Generate `include-what-you-use' report
iwyu: iwyu.log

iwyu.log: compile_commands.json $(COMMON_HEADERS) $(ALL_SRCS) flake.nix
iwyu.log: flake.lock pkg-fun.nix pkgs/nlohmann_json.nix pkgs/nix/pkg-fun.nix
iwyu.log: build-aux/iwyu build-aux/iwyu-mappings.json
	build-aux/iwyu|$(TEE) "$@";

FULLCLEANFILES += iwyu.log


# ---------------------------------------------------------------------------- #

.PHONY: lint
#: Run `clang-tidy' across entire project
lint: compile_commands.json $(COMMON_HEADERS) $(ALL_SRCS)
	$(TIDY) $(filter-out compile_commands.json,$^);


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
BATS_FILE ?= $(MAKEFILE_DIR)/tests
bats-check: bin $(TEST_UTILS)
	PKGDB="$(MAKEFILE_DIR)/bin/pkgdb"                        \
	IS_SQLITE3="$(MAKEFILE_DIR)/tests/is_sqlite3"            \
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

# Generate `pkg-config' file.
# ---------------------------
# The `PC_CFLAGS' and `PC_LIBS' variables carry flags that are not covered by
# `nlohmann_json`, `argparse`, `sqlite3pp`, `sqlite`, and `nix{main,cmd,expr}`
# `Requires' handling.
# This amounts to handling `boost', `libnixfetchers', forcing
# the inclusion of the `nix' `config.h' header, and some additional CPP vars.

PC_CFLAGS =  $(filter -std=%,$(CXXFLAGS))
PC_CFLAGS += $(boost_CFLAGS)
PC_CFLAGS += $(sqlite3pp_CFLAGS)
PC_CFLAGS += -isystem $(nix_INCDIR) -include $(nix_INCDIR)/nix/config.h
PC_CFLAGS += '-DFLOX_PKGDB_VERSION=\\\\\"$(VERSION)\\\\\"'
PC_CFLAGS += '-DSEMVER_PATH=\\\\\"$(SEMVER_PATH)\\\\\"'
PC_LIBS   =  $(shell $(PKG_CONFIG) --libs-only-L nix-main) -lnixfetchers
lib/pkgconfig/flox-pkgdb.pc: $(lastword $(MAKEFILE_LIST)) $(DEPFILES)
lib/pkgconfig/flox-pkgdb.pc: lib/pkgconfig/flox-pkgdb.pc.in version
	$(SED) -e 's,@PREFIX@,$(PREFIX),g'      \
	       -e 's,@VERSION@,$(VERSION),g'    \
	       -e 's,@CFLAGS@,$(PC_CFLAGS),g'   \
	       -e 's,@LIBS@,$(PC_LIBS),g'       \
	       $< > $@;

CLEANFILES += lib/pkgconfig/flox-pkgdb.pc

install-lib: $(LIBDIR)/pkgconfig/flox-pkgdb.pc


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
fmt: $(COMMON_HEADERS) $(ALL_SRCS)
	$(FMT) -i $^;


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
