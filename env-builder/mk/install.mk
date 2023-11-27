# ============================================================================ #
#
# Must be run after evaluating templates.
#
# ---------------------------------------------------------------------------- #

ifndef _MK_GEN_TARGETS
$(error "You must include `mk/gen-targets.mk' before `mk/install.mk'")
endif  # ifndef _MK_GEN_TARGETS


# ---------------------------------------------------------------------------- #

ifndef _MK_INSTALL

_MK_INSTALL = 1

# ---------------------------------------------------------------------------- #

PREFIX     ?= out
BINDIR     ?= $(PREFIX)/bin
LIBDIR     ?= $(PREFIX)/lib
INCLUDEDIR ?= $(PREFIX)/include


# ---------------------------------------------------------------------------- #

.PHONY:  install-bin install-lib install-include
install: install-bin install-lib install-include

$(INCLUDEDIR): include
	$(MKDIR_P) "$(@D)"
	$(CP) -rT -- "$<" "$@"

$(LIBDIR)/%: lib/%
	$(MKDIR_P) "$(@D)"
	$(CP) -- "$<" "$@"

$(BINDIR)/%: bin/%
	$(MKDIR_P) "$(@D)"
	$(CP) -- "$<" "$@"

install-bin:     $(patsubst bin/%,$(BINDIR)/%,$(BIN_TARGETS))
install-lib:     $(patsubst lib/%,$(LIBDIR)/%,$(LIB_TARGETS))
install-include: $(INCLUDEDIR)


# ---------------------------------------------------------------------------- #

endif  # ifndef _MK_INSTALL


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
