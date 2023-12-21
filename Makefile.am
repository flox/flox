# ============================================================================ #
#
# XXX: All the real targets are in the subdirectories.
#
# ---------------------------------------------------------------------------- #

ACLOCAL_AMFLAGS = -I build-aux/m4

.PHONY: pkgdb cli cdb docs docs-recursive

# ---------------------------------------------------------------------------- #

if USE_LOCAL_PKGDB

# Do traverse into sub-directory
MAYBE_pkgdb = pkgdb

pkgdb:
	$(MAKE) $(AM_MAKEFLAGS) -C $(builddir)/pkgdb

cdb:
	$(MAKE) $(AM_MAKEFLAGS) -C $(builddir)/pkgdb clean;
	PATH="$(dir $(BEAR_WRAPPER_CXX)):$(PATH)";                                 \
	  $(BEAR) -- $(MAKE) $(AM_MAKEFLAGS) -C $(builddir)/pkgdb bin/pkgdb tests;

else !USE_LOCAL_PKGDB

# Don't traverse into sub-directory
MAYBE_pkgdb =

# Empty recipes
cdb:
pkgdb:

endif !USE_LOCAL_PKGDB


# ---------------------------------------------------------------------------- #

if USE_LOCAL_FLOX_CLI

# Do traverse into sub-directory
MAYBE_cli = cli

cli:
	$(MAKE) $(AM_MAKEFLAGS) -C $(builddir)/cli

else !USE_LOCAL_FLOX_CLI

# Don't traverse into sub-directory
MAYBE_cli =

# Empty recipes
cli:

endif !USE_LOCAL_FLOX_CLI


# ---------------------------------------------------------------------------- #

SUBDIRS = $(MAYBE_pkgdb) $(MAYBE_cli)


# ---------------------------------------------------------------------------- #

docs-recursive:
	@for dir in $(SUBDIRS); do                              \
	  $(MAKE) $(AM_MAKEFLAGS) -C "$(builddir)/$$dir" docs;  \
	done

docs: docs-recursive


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
