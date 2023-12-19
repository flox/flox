# ============================================================================ #
#
# XXX: All the real targets are in the subdirectories.
#
# ---------------------------------------------------------------------------- #

ACLOCAL_AMFLAGS = -I build-aux/m4

SUBDIRS = pkgdb cli


# ---------------------------------------------------------------------------- #

.PHONY: pkgdb cli

pkgdb:
	$(MAKE) $(AM_MAKEFLAGS) -C $(builddir)/pkgdb

cli:
	$(MAKE) $(AM_MAKEFLAGS) -C $(builddir)/cli


# ---------------------------------------------------------------------------- #

# FIXME: Move over the OSX compat stuff and kill the old Makefile
.PHONY: cdb
cdb:
	$(MAKE) $(AM_MAKEFLAGS) -C $(srcdir)/pkgdb --file=Makefile.bak $@;

##cdb:
##	$(MAKE) $(AM_MAKEFLAGS) -C $(builddir)/pkgdb clean;
##	$(BEAR) -- $(MAKE) $(AM_MAKEFLAGS) -C $(builddir)/pkgdb bin/pkgdb tests;


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
