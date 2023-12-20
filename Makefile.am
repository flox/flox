# ============================================================================ #
#
# XXX: All the real targets are in the subdirectories.
#
# ---------------------------------------------------------------------------- #

ACLOCAL_AMFLAGS = -I build-aux/m4

SUBDIRS =

# ---------------------------------------------------------------------------- #

if USE_LOCAL_PKGDB

SUBDIRS += pkgdb

.PHONY: pkgdb

pkgdb:
	$(MAKE) $(AM_MAKEFLAGS) -C $(builddir)/pkgdb

PHONY: cdb
cdb:
	$(MAKE) $(AM_MAKEFLAGS) -C $(builddir)/pkgdb clean;
	PATH="$(dir $(BEAR_WRAPPER_CXX)):$(PATH)";                                 \
	  $(BEAR) -- $(MAKE) $(AM_MAKEFLAGS) -C $(builddir)/pkgdb bin/pkgdb tests;

endif USE_LOCAL_PKGDB


# ---------------------------------------------------------------------------- #

if USE_LOCAL_FLOX_CLI

SUBDIRS += cli

.PHONY: cli

cli:
	$(MAKE) $(AM_MAKEFLAGS) -C $(builddir)/cli

endif USE_LOCAL_FLOX_CLI



# ---------------------------------------------------------------------------- #

.PHONY: docs docs-recursive

docs: docs-recursive

docs-recursive:
	$(MAKE) $(AM_MAKEFLAGS) -C $(builddir)/pkgdb $@


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
