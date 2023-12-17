# ============================================================================ #
#
#
#
# ---------------------------------------------------------------------------- #

ACLOCAL_AMFLAGS = -I ../build-aux/m4


# ---------------------------------------------------------------------------- #

TARDIR = $(distdir)


# ---------------------------------------------------------------------------- #

copy-srcs:
	$(MKDIR_P) $(abs_builddir);
	$(RSYNC) -r --exclude dist/ --exclude target/ --exclude Makefile  \
                --exclude Makefile.am --exclude Makefile.in           \
	            $(FLOX_CLI_ROOT)/ $(abs_builddir);


# ---------------------------------------------------------------------------- #

build: copy-srcs
	$(MAKE) $(AM_MAKEFLAGS) -C $(abs_top_builddir)/pkgdb bin/pkgdb$(EXEEXT)
	$(CARGO) build $(CARGOFLAGS);

$(abs_builddir)/target/debug/flox: build


build-release: copy-srcs
	$(MAKE) $(AM_MAKEFLAGS) -C $(abs_top_builddir)/pkgdb install;
	PKGDB_BIN="$(DESTDIR)$(bindir)/pkgdb$(EXEEXT)"  \
	$(CARGO) build $(CARGOFLAGS) --release;

$(abs_builddir)/target/release/flox: build-release


# ---------------------------------------------------------------------------- #

##dist-local:
##	$(RM) -r $(FLOX_CLI_DISTDIR);
##	$(MKDIR_P) $(FLOX_CLI_DISTDIR)/$(TARDIR);
##	$(RSYNC) -r --exclude dist/ --exclude target/                 \
##	         $(FLOX_CLI_ROOT)/ $(FLOX_CLI_DISTDIR)/$(TARDIR);
##	cd $(FLOX_CLI_DISTDIR)/$(TARDIR);
##	$(TAR) -czf $(FLOX_CLI_DISTDIR)/$(TARDIR).tar.gz $(TARDIR);


# ---------------------------------------------------------------------------- #

# Default target builds the release version of the binary - otherwise
# `make install' builds twice!

all-local: CARGOFLAGS += --release
all-local: build


# ---------------------------------------------------------------------------- #

clean-local:
	-$(CARGO) clean


# ---------------------------------------------------------------------------- #

install-exec-local: $(abs_builddir)/target/release/flox
	$(MKDIR_P) $(DESTDIR)$(bindir);
	$(INSTALL_PROGRAM) $(abs_builddir)/target/release/flox $(DESTDIR)$(bindir);


# ---------------------------------------------------------------------------- #

EXTRA_DIST =
EXTRA_DIST += README.md
EXTRA_DIST += Cargo.toml Cargo.lock
EXTRA_DIST += flox floxd flox-rust-sdk flox-types
EXTRA_DIST += tests


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
