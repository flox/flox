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
	$(CARGO) build

$(FLOX_CLI_ROOT)/target/debug/flox-cli: build


build-release: copy-srcs
	$(CARGO) build --release

$(FLOX_CLI_ROOT)/target/release/flox-cli: build-release


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
	$(INSTALL_PROGRAM) $(FLOX_CLI_ROOT)/target/release/flox-cli  \
	                   $(DESTDIR)$(bindir);


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
