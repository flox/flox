# Install the Flox systemd integration.
#
# Staged install for packaging:
#   make install DESTDIR=/path/to/buildroot
#
# Direct install for testing on a host:
#   sudo make install && sudo systemctl daemon-reload
#
# Paths follow the conventional locations a distro package would use:
# vendor units under /usr/lib, operator config under /etc.

PREFIX      ?= /usr
UNITDIR     ?= $(PREFIX)/lib/systemd/system
LIBEXECDIR  ?= $(PREFIX)/libexec/flox
TMPFILESDIR ?= $(PREFIX)/lib/tmpfiles.d
CONFDIR     ?= /etc/flox/services

INSTALL ?= install

UNITS   := $(wildcard units/*)
SCRIPTS := libexec/flox-pull libexec/flox-activate libexec/flox-exec-start

.PHONY: install uninstall check test test-user test-all

install:
	$(INSTALL) -d $(DESTDIR)$(UNITDIR) $(DESTDIR)$(LIBEXECDIR) \
	              $(DESTDIR)$(TMPFILESDIR) $(DESTDIR)$(CONFDIR)
	$(INSTALL) -m 0644 $(UNITS) $(DESTDIR)$(UNITDIR)/
	$(INSTALL) -m 0644 libexec/flox-common.sh $(DESTDIR)$(LIBEXECDIR)/
	$(INSTALL) -m 0755 $(SCRIPTS) $(DESTDIR)$(LIBEXECDIR)/
	$(INSTALL) -m 0644 tmpfiles.d/flox.conf $(DESTDIR)$(TMPFILESDIR)/

uninstall:
	rm -f $(addprefix $(DESTDIR)$(UNITDIR)/,$(notdir $(UNITS)))
	rm -rf $(DESTDIR)$(LIBEXECDIR)
	rm -f $(DESTDIR)$(TMPFILESDIR)/flox.conf

# Parse the units and syntax-check the scripts. Does not need them installed.
check:
	@for u in $(UNITS); do \
	  systemd-analyze verify "$$u" 2>&1 \
	    | grep -v 'is not executable' || true; \
	done
	@for s in $(SCRIPTS) libexec/flox-common.sh; do \
	  sh -n "$$s" && echo "$$s: ok"; \
	done

# Exercise the scripts against a stub flox. No systemd, no root, no real flox.
test:
	bats tests/scripts.bats

# Load the units into the caller's own `systemd --user` manager and start them.
# Skips itself if no user manager is reachable.
test-user:
	bats tests/user-manager.bats

test-all: check test test-user
