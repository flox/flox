.DEFAULT_GOAL = all
PKGNAME = flox
PREFIX ?= build
VERSION ?= unknown


# Declare an empty recipe to unconditionally force rebiulds of certain targets
.PHONY: FORCE

# `nix show-config` does not work on Darwin, dies within a build:
#
#   libc++abi: terminating with uncaught exception of type nix::SysError: error: getting status of /System/Library/LaunchDaemons/com.apple.oahd.plist: Operation not permitted
#
# For now we'll just punt and fall back to (coreutils) `uname -s`.
#
# SYSTEM := $(shell nix --extra-experimental-features nix-command show-config | awk '/system = / {print $$NF}')
# CPU = $(firstword $(subst -, ,$(SYSTEM)))
# OS = $(lastword $(subst -, ,$(SYSTEM)))
OS := $(shell uname -s | tr A-Z a-z)

# Fallbacks for non-`nix build'
NIXPKGS_CACERT_BUNDLE_CRT ?= /etc/ssl/certs/ca-bundle.crt

CFLAGS = \
	-DFLOXSH='"$(PREFIX)/libexec/flox/flox"' \
	-DNIXPKGS_CACERT_BUNDLE_CRT='"$(NIXPKGS_CACERT_BUNDLE_CRT)"'
ifeq ($(OS),linux)
  # Fallbacks for non-`nix build'
  LOCALE_ARCHIVE ?= /lib/locale/locale-archive
  CFLAGS += -DLOCALE_ARCHIVE='"$(LOCALE_ARCHIVE)"'
  FLOX_ACTIVATE_BASH ?= lib/commands/shells/activate.linux.bash
endif
ifeq ($(OS),darwin)
  # Fallbacks for non-`nix build'
  NIX_COREFOUNDATION_RPATH ?= /Library/Frameworks
  PATH_LOCALE ?= /share/locale
  CFLAGS += \
	-DNIX_COREFOUNDATION_RPATH='"$(NIX_COREFOUNDATION_RPATH)"' \
	-DPATH_LOCALE='"$(PATH_LOCALE)"'
  FLOX_ACTIVATE_BASH ?= lib/commands/shells/activate.darwin.bash
endif

BIN = flox
MAN1 = $(addsuffix .1,$(BIN))
MAN = $(MAN1)
ETC = \
	etc/flox.bashrc \
	etc/flox.prompt.bashrc \
	etc/flox.toml \
	etc/flox.zdotdir/.zlogin \
	etc/flox.zdotdir/.zlogout \
	etc/flox.zdotdir/.zprofile \
	etc/flox.zdotdir/.zshenv \
	etc/flox.zdotdir/.zshrc \
	etc/flox.zdotdir/prompt.zshrc
LIB = \
	lib/bootstrap.sh \
	lib/commands.sh \
	lib/commands/activate.sh \
	lib/commands/development.sh \
	lib/commands/environment.sh \
	lib/commands/general.sh \
	lib/commands/publish.sh \
	lib/commands/shells/activate.bash \
	lib/diff-catalogs.jq \
	lib/diff-manifests.jq \
	lib/init.sh \
	lib/manifest.jq \
	lib/manifestTOML.jq \
	lib/merge-manifests.jq \
	lib/merge-search-results.jq \
	lib/metadata.sh \
	lib/environmentRegistry.jq \
	lib/registry.jq \
	lib/search.jq \
	lib/searchJSON.jq \
	lib/catalog-search.jq \
	lib/utils.sh \
	lib/catalog-ingest/flake.nix \
	lib/catalog-ingest/lib/analysis.nix \
	lib/catalog-ingest/lib/inspectBuild.nix \
	lib/catalog-ingest/lib/isValidDrv.nix \
	lib/catalog-ingest/lib/readPackage.nix \
	lib/catalog-ingest/placeholder/flake.nix \
	lib/catalog-ingest/plugins/eval.nix \
	lib/templateFloxEnv/flake.lock \
	lib/templateFloxEnv/flake.nix \
	lib/templateFloxEnv/pkgs/default/default.nix \
	lib/templateFloxEnv/pkgs/default/flox.nix
LIBEXEC = \
	libexec/flox/flox \
	libexec/flox/darwin-path-fixer.awk
SHARE = share/bash-completion/completions/flox
ifeq ($(OS),darwin)
  SHARE += \
	share/flox/files/darwin-zshrc_Apple_Terminal.patch \
	share/flox/files/darwin-zshrc.patch
endif
LINKBIN = # Add files to be linked to flox here

# Discern source files just for monitoring by entr with hivemind.
GIT_LS_FILES := $(shell git ls-files)
SRC_BASH = $(filter %.sh,$(GIT_LS_FILES))
SRC_BATS = $(filter %.bats,$(GIT_LS_FILES))
SRC_JQ = $(filter %.jq,$(GIT_LS_FILES))
SRC = $(SRC_BASH) $(SRC_BATS) $(SRC_JQ)

SHFMT = shfmt --language-dialect bash
# SC1090 (warning): ShellCheck can't follow non-constant source. Use a directive to specify location.
# SC1091 (info): Not following: ./init.sh was not specified as input (see shellcheck -x).
# SC2034 (warning): floxMetricsConsent appears unused. Verify use (or export if used externally).
# SC2086 (info): Double quote to prevent globbing and word splitting.
# SC2154 (warning): _nix is referenced but not assigned.
# SC2166 (warning): Prefer [ p ] && [ q ] as [ p -a q ] is not well defined.
SHELLCHECK = shellcheck -s bash -e SC1090,SC1091,SC2034,SC2086,SC2154,SC2166

all: $(BIN) $(MAN)

.SUFFIXES: .md .sh

.md:
	pandoc -s -t man $< -o $@

$(PREFIX)/%: %
	-@rm -f $@
	@mkdir -p $(@D)
	cp $< $@

$(PREFIX)/bin/%: %
	-@rm -f $@
	@mkdir -p $(@D)
	cp $< $@

lib/commands/shells/activate.darwin.bash: pkgs/flox/activate.bash pkgs/flox/activate.darwin.bash
	mkdir -p "$(@D)"
	sed -e "s/@cacert@//g" -e "s/@locale@//g" -e "s/@coreFoundation@//g" $^ > "$@"

lib/commands/shells/activate.linux.bash: pkgs/flox/activate.bash pkgs/flox/activate.linux.bash
	mkdir -p "$(@D)"
	sed -e "s/@\(cacert\|glibcLocales\)@//g" $^ > "$@"


# These files are mastered in default.nix, passed by makeFlags.
$(PREFIX)/lib/commands/shells/activate.bash: $(FLOX_ACTIVATE_BASH)
	-@rm -f "$@"
	@mkdir -p "$(@D)"
	cp "$<" "$@"

$(PREFIX)/etc/%: etc/%
	-@rm -f "$@"
	@mkdir -p "$(@D)"
	sed \
	  -e 's%@@PREFIX@@%$(PREFIX)%' \
	  "$<" > "$@"

$(PREFIX)/lib/%: lib/%
	-@rm -f "$@"
	@mkdir -p "$(@D)"
	sed \
	  -e 's%@@PREFIX@@%$(PREFIX)%' \
	  -e 's%@@VERSION@@%$(VERSION)%' \
	  -e 's%@@FLOXPATH@@%$(FLOXPATH)%' \
	  -e 's%@@NIXPKGS_CACERT_BUNDLE_CRT@@%$(NIXPKGS_CACERT_BUNDLE_CRT)%' \
	  "$<" > "$@"
	$(if $(filter %.sh,$@),$(SHFMT) $@ >/dev/null)
	# TODO: clean up and remove "-" once shellcheck runs cleanly on libs
	-$(if $(filter %.sh,$@),$(SHELLCHECK) $@)

$(PREFIX)/libexec/flox/flox: flox.sh
	-@rm -f $@
	@mkdir -p $(@D)
	sed \
	  -e 's%@@PREFIX@@%$(PREFIX)%' \
	  -e 's%@@VERSION@@%$(VERSION)%' \
	  -e 's%@@FLOXPATH@@%$(FLOXPATH)%' \
	  $< > $@
	$(SHFMT) $@ >/dev/null
	$(SHELLCHECK) $@
	chmod +x $@

$(PREFIX)/libexec/%: libexec/%
	-@rm -f $@
	@mkdir -p $(@D)
	cp $< $@

$(PREFIX)/share/man/man1/%: %
	-@rm -f $@
	@mkdir -p $(@D)
	cp $< $@

$(PREFIX)/share/%: share/%
	-@rm -f $@
	@mkdir -p $(@D)
	cp $< $@

.PHONY: install
install: $(addprefix $(PREFIX)/bin/,$(BIN)) \
         $(addprefix $(PREFIX)/share/man/man1/,$(MAN1)) \
         $(addprefix $(PREFIX)/,$(LIBEXEC) $(LIB) $(ETC) $(SHARE))

define LINK_template =
  $(PREFIX)/bin/$(link):
	mkdir -p $$(@D)
	ln -s flox $$@

  LINKS += $(PREFIX)/bin/$(link)
endef

$(foreach link,$(LINKBIN),$(eval $(call LINK_template)))
install: $(LINKS)

.PHONY: clean
clean:
	-rm -f $(BIN) $(MAN) result
	-rm -rf lib/commands/shells

result: $(SRC)
	nix --extra-experimental-features 'nix-command flakes' build '.#flox-bash' -L

.PHONY: test watch-test
watch-test: FORCE
	for i in $(SRC); do echo "$$i"; done|entr -s '    \
	  echo Building ...;                              \
	  rm -f ./result;                                 \
	  $(MAKE) result;                                 \
	  FLOX_PACKAGE="$${FLOX_PACKAGE:-$(PREFIX)}"      \
	    bats $(if $(MATCH),-f ".*$(MATCH).*") tests;  \
	';

test: result
	FLOX_PACKAGE="$${FLOX_PACKAGE:-$(PREFIX)}"       \
	  bats $(if $(MATCH),-f ".*$(MATCH).*") tests;
