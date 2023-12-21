# ============================================================================ #
#
#
#
# ---------------------------------------------------------------------------- #

ACLOCAL_AMFLAGS = -I ../build-aux/m4

# ---------------------------------------------------------------------------- #

DISTDIR   = $(abs_builddir)/$(FLOX_CLI_distdir)
TARDIR    = $(FLOX_CLI_distdir)/$(FLOX_CLI_tardir)
TARGETDIR = $(abs_builddir)/$(FLOX_CLI_targetdir)

.PHONY: copy-srcs build all-local clean-local install-exec-local docs

# ---------------------------------------------------------------------------- #

copy-srcs:
	$(MKDIR_P) $(abs_builddir);
	$(RSYNC) -r --exclude dist/ --exclude target/ --exclude 'Makefile.*'  \
	            $(FLOX_CLI_ROOT)/ $(abs_builddir)/;


# ---------------------------------------------------------------------------- #

# Define build targets for release and debug builds

# TODO: Use `--target-dir' and `--out-dir'.
#       You need to check other existing usage of `target/{debug,release}'

if ENABLE_RELEASE

# TODO: Use environment variable to set the pkgdb binary path
build: copy-srcs
	$(MAKE) $(AM_MAKEFLAGS) -C $(top_builddir)/pkgdb install;
	PKGDB_BIN="$(DESTDIR)$(bindir)/pkgdb$(EXEEXT)"  \
	  $(CARGO) build $(CARGOFLAGS);

else !ENABLE_RELEASE

build: copy-srcs
	$(MAKE) $(AM_MAKEFLAGS) -C $(top_builddir)/pkgdb bin/pkgdb$(EXEEXT)
	PKGDB_BIN="$(top_builddir)/pkgdb/bin/pkgdb$(EXEEXT)"  \
	  $(CARGO) build $(CARGOFLAGS);

endif !ENABLE_RELEASE

$(TARGETDIR)/flox: build


# ---------------------------------------------------------------------------- #

##dist-local:
##	$(RM) -r $(DISTDIR);
##	$(MKDIR_P) $(TARDIR);
##	$(RSYNC) -r --exclude dist/ --exclude target/ $(FLOX_CLI_ROOT)/ $(TARDIR);
##	cd $(DISTDIR);
##	$(TAR) -czf $(TARDIR).tar.gz $(TARDIR);


# ---------------------------------------------------------------------------- #

# Default target builds the release version of the binary - otherwise
# `make install' builds twice!

all-local: build


# ---------------------------------------------------------------------------- #

clean-local: copy-srcs
	-$(CARGO) clean


# ---------------------------------------------------------------------------- #

install-exec-local: $(TARGETDIR)/flox
	$(MKDIR_P) $(DESTDIR)$(bindir);
	$(INSTALL_PROGRAM) $(TARGETDIR)/flox $(DESTDIR)$(bindir);


# ---------------------------------------------------------------------------- #

EXTRA_DIST =
EXTRA_DIST += README.md
EXTRA_DIST += Cargo.toml Cargo.lock
EXTRA_DIST += flox floxd flox-rust-sdk flox-types
EXTRA_DIST += tests


# ---------------------------------------------------------------------------- #

# TODO
docs:


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
