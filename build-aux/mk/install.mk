# ============================================================================ #
#
# @file build-aux/mk/install.mk
#
# @brief Provides variables and recipes associated with installation.
#
#
# ---------------------------------------------------------------------------- #

ifndef __MK_INSTALL
__MK_INSTALL = 1

# ---------------------------------------------------------------------------- #

ifeq (,$(MK_DIR))
$(error "$(lastword $(MAKEFILE_LIST)): MK_DIR is not set")
endif  # ifeq (,$(MK_DIR))

include $(MK_DIR)/files.mk
include $(MK_DIR)/utils.mk

# ---------------------------------------------------------------------------- #

# Install Prefixes
# ----------------

PREFIX     ?= $(MAKEFILE_DIR)/out
BINDIR     ?= $(PREFIX)/bin
LIBDIR     ?= $(PREFIX)/lib
INCLUDEDIR ?= $(PREFIX)/include


# ---------------------------------------------------------------------------- #

# Install Targets
# ---------------

.PHONY: install-dirs install-bin install-lib install-include install

#: Install binaries, libraries, and include files
install: install-dirs install-bin install-lib install-include

#: Create directories in the install prefix
install-dirs: FORCE
	$(MKDIR_P) $(BINDIR) $(LIBDIR) $(LIBDIR)/pkgconfig;
	$(MKDIR_P) $(INCLUDEDIR)/flox/env-builder;

$(INCLUDEDIR)/%: include/% | install-dirs
	$(CP) -- "$<" "$@";

$(LIBDIR)/%: lib/% | install-dirs
	$(CP) -- "$<" "$@";

$(BINDIR)/%: bin/% | install-dirs
	$(CP) -- "$<" "$@";


# ---------------------------------------------------------------------------- #

#: Install binaries
install-bin: $(addprefix $(BINDIR)/,$(BINS))

#: Install libraries
install-lib: $(addprefix $(LIBDIR)/,$(LIBS))

#: Install include files
install-include:                                                       \
	$(addprefix $(INCLUDEDIR)/,$(subst include/,,$(COMMON_HEADERS)));


# ---------------------------------------------------------------------------- #

endif  # ifndef __MK_INSTALL

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
