# ============================================================================ #
#
#
#
# ---------------------------------------------------------------------------- #

CAT ?= cat
RM  ?= rm -f

# ---------------------------------------------------------------------------- #

.DEFAULT_GOAL = all

SUBDIRS =

GENERATED_FILES += flake.nix.in.hh


# ---------------------------------------------------------------------------- #

flake.nix.in.hh: flake.nix.in
	echo 'R"_NIX(' > $@;
	$(CAT) $< >> $@;
	echo ')_NIX"' >> $@;


# ---------------------------------------------------------------------------- #

.PHONY: all-generic all-recursive all

all-generic: $(GENERATED_FILES)

all-recursive:
	for subdir in $(SUBDIRS); do $(MAKE) -C $$subdir all; done

all: all-generic all-recursive


# ---------------------------------------------------------------------------- #

CLEANFILES     = $(GENERATED_FILES)
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
