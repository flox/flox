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
	$(RSYNC) -r --exclude dist/ --exclude target/ $(FLOX_CLI_ROOT)/  \
                                                  $(abs_builddir);


# ---------------------------------------------------------------------------- #

build: copy-srcs
	$(MAKE) $(AM_MAKEFLAGS) -C $(abs_top_builddir)/pkgdb bin/pkgdb$(EXEEXT)
	$(CARGO) build;

$(FLOX_CLI_ROOT)/target/debug/flox: build


build-release: copy-srcs
	$(MAKE) $(AM_MAKEFLAGS) -C $(abs_top_builddir)/pkgdb install;
	PKGDB_BIN="$(DESTDIR)$(bindir)/pkgdb$(EXEEXT)"  \
	$(CARGO) build --release;

$(FLOX_CLI_ROOT)/target/release/flox: build-release


# ---------------------------------------------------------------------------- #

##dist-local:
##	$(RM) -r $(FLOX_CLI_DISTDIR);
##	$(MKDIR_P) $(FLOX_CLI_DISTDIR)/$(TARDIR);
##	$(RSYNC) -r --exclude dist/ --exclude target/                 \
##	         $(FLOX_CLI_ROOT)/ $(FLOX_CLI_DISTDIR)/$(TARDIR);
##	cd $(FLOX_CLI_DISTDIR)/$(TARDIR);
##	$(TAR) -czf $(FLOX_CLI_DISTDIR)/$(TARDIR).tar.gz $(TARDIR);


# ---------------------------------------------------------------------------- #

all-local: build


# ---------------------------------------------------------------------------- #

clean-local:
	-$(CARGO) clean


# ---------------------------------------------------------------------------- #

install-exec-local: build-release
	$(MKDIR_P) $(DESTDIR)$(bindir);
	$(INSTALL_PROGRAM) target/release/flox $(DESTDIR)$(bindir);


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
