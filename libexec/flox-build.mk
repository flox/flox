#
# This makefile implements Tom's stepladder from manifest to Nix builds:
#
# 1. "local": sets $out in the environment, invokes the build commands in a subshell
#    (using bash), then turns the $out directory into a Nix package with all outpath
#    references replaced with the real $out and all bin/* commands wrapped with
#    $FLOX_ENV/activate
# 2. "sandbox": invokes that same script from within the runCommand builder, with no
#    network and filesystem access and a fake home directory
# 3. "sandbox with buildCache": does as above, with the build directory persisted
#    across builds
# 4. "staged": splits the builds into stages, each of which can be any of the above,
#    and whose "locked" values are stored as a result symlink or as a storePath
#    within the manifest
#

# Start by checking that the FLOX_ENV environment variable is set.
ifeq (,$(FLOX_ENV))
  $(error ERROR: FLOX_ENV not defined)
endif

# Set the default goal to be all builds if one is not specified.
.DEFAULT_GOAL := all

# Set a default TMPDIR variable if one is not already defined.
TMPDIR ?= /tmp

# Use the wildcard operator to identify targets in the provided $FLOX_ENV.
BUILDS := $(wildcard $(FLOX_ENV)/package-builds.d/*)

# The `nix build` command will rebuild the source in every instance,
# and we will presumably want `flox build` to do the same. However,
# we cannot just mark the various build targets as PHONY because they
# must be INTERMEDIATE to prevent `flox build foo` from rebuilding
# `bar` and `baz` as well (unless of course it was a prerequsite).
# So we instead derive the packages to be force-rebuilt from the special
# MAKECMDGOALS variable if defined, and otherwise rebuild them all.
BUILDGOALS = $(if $(MAKECMDGOALS),$(MAKECMDGOALS),$(notdir $(BUILDS)))
$(foreach _build,$(BUILDGOALS),\
  $(eval _pname = $(notdir $(_build)))\
  $(foreach _buildtype,local sandbox,\
    $(eval $(_pname)_$(_buildtype)_build: FORCE)))

# Scan for "${package}" references within the build instructions and add
# target prerequisites for any inter-package prerequisites, letting make
# flag any circular dependencies encountered along the way.
define DEPENDS_template =
  # Infer pname from script path.
  $(eval _pname = $(notdir $(build)))
  # Target names cannot have "-" in them so replace with "_" in the target name.
  $(eval _pvarname = $(subst -,_,$(_pname)))

  # Render the build script with the package prerequisites replaced with their
  # corresponding outpaths, using a temporary path that is stable across builds
  # so that we only perform a Nix rebuild when the contents actually change.
  $(eval _buildScript_checksum := $(shell sha256sum $(build) | head -c8))
  $(eval $(_pvarname)_buildScript := $(TMPDIR)/$(_buildScript_checksum)-build-$(_pname).bash)

  # Iterate over each possible {build,package} pair looking for references to
  # ${package} in the build script, being careful to avoid looking for references
  # to the package in its own build. If found add dependency from the build
  # script to the package.
  $(foreach package,$(notdir $(BUILDS)),\
    $(if $(filter-out $(package),$(notdir $(build))),\
      $(if $(shell grep '\$${$(package)}' $(build)),\
        $(eval _dep = result-$(package))\
        $(eval $(_pvarname)_buildDeps += $(realpath $(_dep)))\
        $($(_pvarname)_buildScript): $(_dep))))
endef

$(foreach build,$(BUILDS),$(eval $(call DEPENDS_template)))

# Template for rendering temporary stable buildcache symlink. We use
# the shorter relative result symlink path rather than the absolute
# nix storePath so it looks better on the graph. Marking it as
# INTERMEDIATE ensures that make will delete the link once it has
# been used.
define BUILDCACHE_template =
  .INTERMEDIATE: $(_buildCache)
  $(_buildCache): $(_result)-buildCache
	-rm -f $$@
	@# Copy, don't link to the previous buildCache because we want
	@# nix to import it as a content-addressed input rather than an
	@# ever-changing series of storePaths.
	cp $(_result)-buildCache $$@
endef

# The following template renders targets for each of the build modes.
# We render all the possible build modes here and then below we select
# the actual targets to be evaluated based on the build types observed.
space := $(subst x,,x x)
define BUILD_template =
  # Infer pname from script path.
  $(eval _pname = $(notdir $(build)))
  # Variable names cannot have "-" in them so create a copy of _pname replacing
  # "-" characters with "_".
  $(eval _pvarname = $(subst -,_,$(_pname)))
  # Identify result symlink basename.
  $(eval _result = result-$(_pname))
  # Eventually derive version somehow, but hardcode it in the meantime.
  $(eval _version = 0.0.0)
  # Calculate name.
  $(eval _name = $(_pname)-$(_version))
  # Short variable name for buildDependencies derived in the DEPENDS step.
  $(eval _buildDeps = $(strip \
    $(if $($(_pvarname)_buildDeps),\
      '["$(subst $(space),",$($(_pvarname)_buildDeps))"]')))

  # Set temp outpath of same strlen as eventual package storePath using sha256sum
  # derived from the package name, the current working directory and the $(FLOX_ENV)
  # package to provide a stable random seed to avoid collisions.
  $(eval _tmphash = $(shell ( \
    echo $(_name) && pwd && realpath "$$FLOX_ENV") | sha256sum | head -c32))
  $(eval _out = /tmp/store_$(_tmphash)-$(_name))

  # It is expected that the sandbox and caching modes will be specified on a
  # per-build basis within the manifest, but in the meantime while we wait for
  # the manifest parser to be implemented we will grep for explicit "buildCache"
  # and "sandbox" settings within the build script for setting the build and
  # caching modes.
  $(eval _build_mode = $(if $(shell grep -E '\.sandbox = true$$' $(build)),sandbox,local))

  # The buildCache value needs to be stable when nothing changes across builds,
  # so we create a symlink from a stable TMPDIR path and pass that to the
  # derivation instead. Note that realpath doubles as an existence check.
  $(eval _do_buildCache =)
  $(if $(shell grep -E '\.buildCache = true$$' $(build)), \
    $(eval _do_buildCache = 1) \
    $(eval _buildCache =) \
    $(if $(realpath $(_result)-buildCache), \
      $(eval _buildCache_checksum = $(shell sha256sum $(_result)-buildCache | head -c8)) \
      $(eval _buildCache = $(TMPDIR)/$(_buildCache_checksum)-$(_name)-buildCache) \
      $(eval $(call BUILDCACHE_template))))

  # By the time this rule will be evaluated all of the package dependencies
  # will have been added to the set of rule prerequisites in $^, using their
  # "safe" name (with "-" characters replaced with "_"), and these targets
  # will have successfully built the corresponding result-$(_pname) symlinks.
  # Iterate through this list, replacing all instances of "${package}" with the
  # corresponding storePath as identified by the result-* symlink.
  .INTERMEDIATE: $($(_pvarname)_buildScript)
  $($(_pvarname)_buildScript): $(build)
	@echo "Rendering $(_pname) build script to $$@"
	@cp $$< $$@
	@for i in $$^; do \
	  if [ -L "$$$$i" ]; then \
	    outpath="$$$$(readlink $$$$i)"; \
	    if [ -n "$$$$outpath" ]; then \
	      pkgname="$$$$(echo $$$$i | cut -d- -f2-)"; \
	      sed -i "s%\$$$${$$$$pkgname}%$$$$outpath%g" $$@; \
	    fi; \
	  fi; \
	done

  # Prepare temporary log file for capturing build output for inspection.
  $(eval $(_pvarname)_logfile := $(shell mktemp --dry-run --suffix=-build-$(_pname).log))

  # Type 1 "local" build
  .INTERMEDIATE: $(_pname)_local_build
  $(_pname)_local_build: $($(_pvarname)_buildScript)
	@echo "Building $(_name) in local mode"
	MAKEFLAGS= FLOX_TURBO=1 out=$(_out) $(FLOX_ENV)/activate bash -e $($(_pvarname)_buildScript)
	nix --extra-experimental-features nix-command \
	  build -L --file __FLOX_CLI_OUTPATH__/libexec/build-manifest.nix \
	    --argstr name "$(_name)" \
	    --argstr flox-env "$(FLOX_ENV)" \
	    --argstr install-prefix "$(_out)" \
	    --out-link "result-$(_pname)" \
	    --offline 2>&1 | tee $($(_pvarname)_logfile)

  # Type 2 "sandbox" build
  .INTERMEDIATE: $(_pname)_sandbox_build
  $(_pname)_sandbox_build: $($(_pvarname)_buildScript) $(if $(_do_buildCache),$(_buildCache))
	@echo "Building $(_name) in sandbox mode"
	@# N.B. realpath returns empty string if path does not exist.
	nix --extra-experimental-features nix-command \
	  build -L --file __FLOX_CLI_OUTPATH__/libexec/build-manifest.nix \
	    --argstr name "$(_name)" \
	    --argstr srcdir "$(realpath .)" \
	    --argstr flox-env "$(FLOX_ENV)" \
	    --argstr install-prefix "$(_out)" \
	    $(if $(_buildDeps),--arg buildDeps $(_buildDeps)) \
	    --argstr buildScript "$($(_pvarname)_buildScript)" \
	    $(if $(_do_buildCache),--argstr buildCache "$(_buildCache)") \
	    --out-link "result-$(_pname)" \
	    '^*' 2>&1 | tee $($(_pvarname)_logfile)

  # Select the desired build mode as we declare the result symlink target.
  $(_result): $(_pname)_$(_build_mode)
	@# Take this opportunity to fail the build if we spot fatal errors in the log.
	@if grep -q "flox build failed (caching build dir)" $($(_pvarname)_logfile); then \
	  echo "ERROR: flox build failed (see $($(_pvarname)_logfile))" 1>&2; \
	  rm -f $$@; \
	  exit 1; \
	fi

  # Create a helper target for referring to the package by its name rather
  # than the [real] result symlink we're looking to create.
  $(_pname): $(_result)

  # Accumulate a list of known build targets for the "all" target.
  all += $(_pname)
endef

$(foreach build,$(BUILDS),$(eval $(call BUILD_template)))

# Finally, we create the "all" target to build all known packages.
all: $(all)

.PHONY: FORCE
FORCE:
