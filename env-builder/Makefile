# ============================================================================ #
#
#
#
# ---------------------------------------------------------------------------- #

MAKEFILE_DIR ?= $(patsubst %/,%,$(dir $(abspath $(lastword $(MAKEFILE_LIST)))))
REPO_ROOT    := $(patsubst %/,%,$(dir $(MAKEFILE_DIR)))


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
LIBENVBUILDER = libenvbuilder$(libExt)

# Various file and target lists

BINS           =  env-builder
LIBS           =  $(LIBENVBUILDER)
HEADERS        =  $(call rwildcard,include,*.hh)
SRCS           =  $(call rwildcard,src,*.cc)
ALL_SRCS       =  $(SRCS)
bin_SRCS       =  src/main.cc src/help.cc
lib_SRCS       =  $(filter-out $(bin_SRCS),$(SRCS))
CLEANDIRS      =
CLEANFILES     =  $(ALL_SRCS:.cc=.o)
CLEANFILES     += $(addprefix bin/,$(BINS)) $(addprefix lib/,$(LIBS))
FULLCLEANDIRS  =
FULLCLEANFILES =

# Where to find test suite input data files.
TEST_DATA_DIR = $(MAKEFILE_DIR)/tests/fixtures


# ---------------------------------------------------------------------------- #

# Files which effect dependencies, external inputs, and `*FLAGS' values.
DEPFILES =
DEPFILES += $(addprefix $(REPO_ROOT)/flake.,nix lock)
DEPFILES += $(REPO_ROOT)/pkgs/flox-pkgdb/default.nix
DEPFILES += $(REPO_ROOT)/pkgs/flox-env-builder/default.nix
DEPFILES += $(REPO_ROOT)/pkgs/nlohmann_json/default.nix
DEPFILES += $(REPO_ROOT)/shells/default/default.nix


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
libenvbuilder_LDFLAGS = '-L$(MAKEFILE_DIR)/lib'
ifeq (Linux,$(OS))
libenvbuilder_LDFLAGS += -Wl,--enable-new-dtags '-Wl,-rpath,$$ORIGIN/../lib'
else  # Darwin
ifneq "$(findstring install,$(MAKECMDGOALS))" ""
libenvbuilder_LDFLAGS = '-L$(LIBDIR)'
else
libenvbuilder_LDFLAGS += '-L$(MAKEFILE_DIR)/lib' -rpath @executable_path/../lib
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
tests:


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

ifeq (Linux,$(OS))
SONAME_FLAG = -Wl,-soname,$(LIBENVBUILDER)
else
SONAME_FLAG = -install_name '@rpath/$(LIBENVBUILDER)'
endif

lib/$(LIBENVBUILDER): LDFLAGS  += $(lib_LDFLAGS)
lib/$(LIBENVBUILDER): CXXFLAGS += $(lib_CXXFLAGS)
lib/$(LIBENVBUILDER): $(lib_SRCS:.cc=.o)
	$(MKDIR_P) $(@D);
	$(CXX) $(filter %.o,$^) $(LDFLAGS) $(SONAME_FLAG) -o $@;


# ---------------------------------------------------------------------------- #

$(bin_SRCS:.cc=.o): %.o: %.cc $(HEADERS)
	$(CXX) $(CXXFLAGS) $(bin_CXXFLAGS) -c $< -o $@;

bin/env-builder: $(bin_SRCS:.cc=.o) lib/$(LIBENVBUILDER)
	$(MKDIR_P) $(@D);
	$(CXX) $(filter %.o,$^) $(LDFLAGS) $(bin_LDFLAGS) -o $@;


# ---------------------------------------------------------------------------- #

# Install Targets
# ---------------

.PHONY: install-dirs install-bin install-lib install-include install

#: Install binaries, libraries, and include files
install: install-dirs install-bin install-lib install-include

#: Create directories in the install prefix
install-dirs: FORCE
	$(MKDIR_P) $(BINDIR) $(LIBDIR) $(LIBDIR)/pkgconfig $(INCLUDEDIR);

$(INCLUDEDIR)/%: include/% | install-dirs
	$(MKDIR_P) $(@D);
	$(CP) -- "$<" "$@";

$(LIBDIR)/%: lib/% | install-dirs
	$(CP) -- "$<" "$@";

$(BINDIR)/%: bin/% | install-dirs
	$(CP) -- "$<" "$@";

# Darwin has to relink
ifneq (Linux,$(OS))
LINK_INAME_FLAG = -install_name '@rpath/$(LIBENVBUILDER)'
$(LIBDIR)/$(LIBENVBUILDER): CXXFLAGS += $(lib_CXXFLAGS)
$(LIBDIR)/$(LIBENVBUILDER): LDFLAGS  += $(lib_LDFLAGS)
$(LIBDIR)/$(LIBENVBUILDER): $(lib_SRCS:.cc=.o)
	$(MKDIR_P) $(@D);
	$(CXX) $(filter %.o,$^) $(LDFLAGS) -o $@;

$(BINDIR)/env-builder: $(bin_SRCS:.cc=.o) $(LIBDIR)/$(LIBENVBUILDER)
	$(MKDIR_P) $(@D);
	$(CXX) $(filter %.o,$^) $(LDFLAGS) $(bin_LDFLAGS) -o $@;
endif # ifneq (Linux,$(OS))

#: Install binaries
install-bin: $(addprefix $(BINDIR)/,$(BINS))

#: Install libraries
install-lib: $(addprefix $(LIBDIR)/,$(LIBS))

#: Install include files
install-include:                                                     \
	$(addprefix $(INCLUDEDIR)/,$(subst include/,,$(HEADERS)));


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
	{                                                                 \
	  if $(TEST) -n "$(NIX_CC)"; then                                 \
	    $(CAT) "$(NIX_CC)/nix-support/libc-cflags";                   \
	    $(CAT) "$(NIX_CC)/nix-support/libcxx-cxxflags";               \
	  fi;                                                             \
	  echo $(CXXFLAGS) $(pkgdb_CFLAGS) $(nix_CFLAGS);                 \
	  echo '-DTEST_DATA_DIR="$(TEST_DATA_DIR)"';                      \
	}|$(TR) ' ' '\n'|$(SED) 's/-std=\(.*\)/%cpp -std=\1|%h -std=\1/'  \
	 |$(TR) '|' '\n' >> "$@";

FULLCLEANFILES += .ccls


# ---------------------------------------------------------------------------- #

# TODO: Add `compile_commands.json' target.
# 	 This is a bit tricky because we need to ensure that it appends the `pkgdb'
# 	 targets while overriding any stale `env-builder' targets.


# ---------------------------------------------------------------------------- #

.PHONY: check bats-check

#: Run all tests
check: bats-check

#: Run all bats tests
ifneq "$(shell $${PKGDB_BIN:-pkgdb} --help >/dev/null 2>&1 && echo 1)" "1"
$(REPO_ROOT)/pkgdb/bin/pkgdb: $(REPO_ROOT)/pkgdb/lib/libpkgdb$(libExt)
$(REPO_ROOT)/pkgdb/bin/pkgdb:
	$(MAKE) -C $(REPO_ROOT)/pkgdb bin/pkgdb
bats-check: $(REPO_ROOT)/pkgdb/bin/pkgdb
bats-check: bin
	PATH="$$PATH:$(REPO_ROOT)/pkgdb/bin" flox-env-builder-tests
else # have pkgdb
bats-check: bin
	flox-env-builder-tests
endif # built pkgdb


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

# Formatter
# ---------

.PHONY: fmt
#: Run `clang-format' across entire project
fmt: $(HEADERS) $(ALL_SRCS)
	$(FMT) -i $^;


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
