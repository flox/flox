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

# Set path to `PKGDB_BIN' with recipe for _local_ builds.
if USE_LOCAL_PKGDB

if ENABLE_RELEASE
$(PKGDB_BIN):
	$(MAKE) $(AM_MAKEFLAGS) -C $(top_builddir)/pkgdb install-exec
else !ENABLE_RELEASE
$(PKGDB_BIN):
	$(MAKE) $(AM_MAKEFLAGS) -C $(top_builddir)/pkgdb bin/pkgdb$(EXEEXT)
endif !ENABLE_RELEASE

else !USE_LOCAL_PKGDB

$(PKGDB_BIN):

endif !USE_LOCAL_PKGDB


# ---------------------------------------------------------------------------- #

copy-srcs:
	$(MKDIR_P) $(abs_builddir);
	$(RSYNC) -r --exclude dist/ --exclude target/ --exclude 'Makefile.*'  \
	            $(FLOX_CLI_ROOT)/ $(abs_builddir)/;


# ---------------------------------------------------------------------------- #

# Define build targets for release and debug builds

# TODO: Use `--target-dir' and `--out-dir'.
#       You need to check other existing usage of `target/{debug,release}'

build: copy-srcs $(PKGDB_BIN)
	PKGDB_BIN="$(PKGDB_BIN)" $(CARGO) build $(CARGOFLAGS);

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

check-local: build
	PKGDB_BIN="$(PKGDB_BIN)" $(CARGO) test --workspace $(CARGOFLAGS);


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
