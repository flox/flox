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
	-DNIXPKGS_CACERT_BUNDLE_CRT='"$(NIXPKGS_CACERT_BUNDLE_CRT)"'
ifeq ($(OS),linux)
  # Fallbacks for non-`nix build'
  LOCALE_ARCHIVE ?= /lib/locale/locale-archive
  CFLAGS += -DLOCALE_ARCHIVE='"$(LOCALE_ARCHIVE)"'
endif
ifeq ($(OS),darwin)
  # Fallbacks for non-`nix build'
  NIX_COREFOUNDATION_RPATH ?= /Library/Frameworks
  PATH_LOCALE ?= /share/locale
  CFLAGS += \
	-DNIX_COREFOUNDATION_RPATH='"$(NIX_COREFOUNDATION_RPATH)"' \
	-DPATH_LOCALE='"$(PATH_LOCALE)"'
endif

MAN1 = flox.1
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
       lib/templateFloxEnv/flake.lock \
       lib/templateFloxEnv/flake.nix \
       lib/templateFloxEnv/pkgs/default/default.nix \
       lib/templateFloxEnv/pkgs/default/flox.nix
SHARE = share/bash-completion/completions/flox
ifeq ($(OS),darwin)
  SHARE += \
	share/flox/files/darwin-zshrc_Apple_Terminal.patch \
	share/flox/files/darwin-zshrc.patch
endif

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

all: $(MAN)

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
install: $(addprefix $(PREFIX)/share/man/man1/,$(MAN1)) \
         $(addprefix $(PREFIX)/,$(LIB) $(ETC) $(SHARE))

.PHONY: clean
clean:
	-rm -f $(BIN) $(MAN) result
	-rm -rf lib/commands/shells

result: $(SRC)
	nix --extra-experimental-features 'nix-command flakes' build '.#flox-bash' -L
