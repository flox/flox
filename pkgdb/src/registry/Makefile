# ============================================================================ #
#
#
#
# ---------------------------------------------------------------------------- #

CAT ?= cat
RM  ?= rm -f
NIX ?= nix --experimental-features 'nix-command flakes'

# ---------------------------------------------------------------------------- #

.DEFAULT_GOAL = all

SUBDIRS = floxpkgs

LIBS    = libregistry.a
SOURCES = $(wildcard *.cc)
OBJECTS = $(SOURCES:.cc=.o)


# ---------------------------------------------------------------------------- #

# Rules for `pkgdb scrape' are stored in a JSON file.
RULES_JSON ?= $(shell $(NIX) store add-path -n rules.json floxpkgs/rules.json)
RULES_JSON := $(RULES_JSON)


# ---------------------------------------------------------------------------- #

$(OBJECTS): $(wildcard ../../include/flox/*.hh)
$(OBJECTS): $(wildcard ../../include/flox/registry/*.hh)
$(OBJECTS): $(wildcard ../../include/flox/core/*.hh)


# ---------------------------------------------------------------------------- #

floxpkgs/%:
	$(MAKE) -C floxpkgs $(patsubst floxpkgs/%,%,$@)


# ---------------------------------------------------------------------------- #

floxpkgs.o: floxpkgs/flake.nix.in.hh
floxpkgs.o: CPPFLAGS += '-DRULES_JSON="$(RULES_JSON)"'


# ---------------------------------------------------------------------------- #

libregistry.a: $(OBJECTS)
	$(AR) rcs $@ $^


# ---------------------------------------------------------------------------- #

.PHONY: all-generic all-recursive all

all-generic: $(LIBS)

all-recursive:
	for subdir in $(SUBDIRS); do $(MAKE) -C $$subdir all; done

all: all-generic all-recursive


# ---------------------------------------------------------------------------- #

CLEANFILES     = $(OBJECTS) $(LIBS)
FULLCLEANFILES =

.PHONY: clean-generic clean-recursive clean
.PHONY: fullclean-generic fullclean-recursive fullclean

clean-generic:
	-$(RM) $(CLEANFILES)

clean-recursive:
	for subdir in $(SUBDIRS); do $(MAKE) -C $$subdir clean; done

clean: clean-generic clean-recursive

fullclean-generic: clean-generic
	-$(RM) $(FULLCLEANFILES)

fullclean-recursive:
	for subdir in $(SUBDIRS); do $(MAKE) -C $$subdir fullclean; done

fullclean: fullclean-generic fullclean-recursive


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
