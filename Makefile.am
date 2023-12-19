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

# FIXME: Move over the OSX compat stuff and kill the old Makefile
.PHONY: cdb
cdb:
	$(MAKE) $(AM_MAKEFLAGS) -C $(srcdir)/pkgdb --file=Makefile.bak $@;

##cdb:
##	$(MAKE) $(AM_MAKEFLAGS) -C $(builddir)/pkgdb clean;
##	$(BEAR) -- $(MAKE) $(AM_MAKEFLAGS) -C $(builddir)/pkgdb bin/pkgdb tests;


endif USE_LOCAL_PKGDB


# ---------------------------------------------------------------------------- #

if USE_LOCAL_FLOX_CLI

SUBDIRS += cli

.PHONY: cli

cli:
	$(MAKE) $(AM_MAKEFLAGS) -C $(builddir)/cli

endif USE_LOCAL_FLOX_CLI



# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
