# ============================================================================ #
#
# XXX: All the real targets are in the subdirectories.
#
# ---------------------------------------------------------------------------- #

ACLOCAL_AMFLAGS = -I build-aux/m4

SUBDIRS =

.PHONY: pkgdb cli cdb docs docs-recursive

# ---------------------------------------------------------------------------- #

if USE_LOCAL_PKGDB

SUBDIRS += pkgdb

pkgdb:
	$(MAKE) $(AM_MAKEFLAGS) -C $(builddir)/pkgdb

cdb:
	$(MAKE) $(AM_MAKEFLAGS) -C $(builddir)/pkgdb clean;
	PATH="$(dir $(BEAR_WRAPPER_CXX)):$(PATH)";                                 \
	  $(BEAR) -- $(MAKE) $(AM_MAKEFLAGS) -C $(builddir)/pkgdb bin/pkgdb tests;

else !USE_LOCAL_PKGDB

# Empty recipes
cdb:
pkgdb:

endif !USE_LOCAL_PKGDB


# ---------------------------------------------------------------------------- #

if USE_LOCAL_FLOX_CLI

SUBDIRS += cli

cli:
	$(MAKE) $(AM_MAKEFLAGS) -C $(builddir)/cli

else !USE_LOCAL_FLOX_CLI

cli:

endif !USE_LOCAL_FLOX_CLI



# ---------------------------------------------------------------------------- #

docs: docs-recursive

docs-recursive:
	$(MAKE) $(AM_MAKEFLAGS) -C $(builddir)/pkgdb $@


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
