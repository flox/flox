#
# This makefile implements Tom's stepladder from manifest to Nix builds:
#
# 1. "local" (aka in-situ): sets $out in the environment, invokes the build
#    commands in a subshell (using bash), then turns the $out directory into
#    a Nix package with all outpath references replaced with the real $out
#    and all bin/* commands wrapped with $FLOX_ENV/activate
# 2. "sandbox": invokes that same script from within the runCommand builder,
#    with no network and filesystem access and a fake home directory
# 3. "sandbox with buildCache": does as above, with the build directory
#    persisted across builds
# 4. "staged": splits the builds into stages, each of which can be any of
#    the above, and whose "locked" values are stored as a result symlink or
#    as a storePath within the manifest
#

# Start by checking that the FLOX_ENV environment variable is set.
ifeq (,$(FLOX_ENV))
  $(error ERROR: FLOX_ENV not defined)
endif

# Identify target O/S.
OS := $(shell uname -s)

# Set the default goal to be all builds if one is not specified.
.DEFAULT_GOAL := all

# Set a default TMPDIR variable if one is not already defined.
TMPDIR ?= /tmp

# Use the wildcard operator to identify builds in the provided $FLOX_ENV.
BUILDS := $(wildcard $(FLOX_ENV)/package-builds.d/*)

# The `nix build` command will attempt a rebuild in every instance,
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
  # We want to create build-specific variables, and variable names cannot
  # have "-" in them so we create a version of the build "pname" replacing
  # this with "_" for use in variable names.
  $(eval _pvarname = $(subst -,_,$(_pname)))
  # Compute 32-character stable string for use in stable path generation
  # based on hash of pname, current working directory and FLOX_ENV.
  $(eval $(_pvarname)_hash = $(shell ( \
    ( echo $(_pname) $(realpath $(FLOX_ENV)) && pwd ) | sha256sum | head -c32)))
  # Render a shorter 8-character version as well.
  $(eval $(_pvarname)_shortHash = $(shell echo $($(_pvarname)_hash) | head -c8))
  # And while we're at it, set a temporary basename using the short hash.
  $(eval $(_pvarname)_tmpBasename = $(TMPDIR)/$($(_pvarname)_shortHash)-$(_pname))

  # We need to render a version of the build script with package prerequisites
  # replaced with their corresponding outpaths, and we create that at a stable
  # temporary path so that we only perform Nix rebuilds when necessary.
  $(eval $(_pvarname)_buildScript = $($(_pvarname)_tmpBasename)-build.bash)

  # Iterate over each possible {build,package} pair looking for references to
  # ${package} in the build script, being careful to avoid looking for references
  # to the package in its own build. If found, declare dependency from the build
  # script to the package.
  $(foreach package,$(filter-out $(notdir $(build)),$(notdir $(BUILDS))),\
    $(if $(shell grep '\$${$(package)}' $(build)),\
      $(eval _dep = result-$(package))\
      $(eval $(_pvarname)_buildDeps += $(realpath $(_dep)))\
      $($(_pvarname)_buildScript): $(_dep)))
endef

$(foreach build,$(BUILDS),$(eval $(call DEPENDS_template)))

# Define macro containing single space character for use in string substitution.
space := $(subst x,,x x)

# The method of calling the sandbox differs based on O/S. Define
# PRELOAD_ARGS to denote the correct way.
ifeq (Darwin,$(OS))
  PRELOAD_ARGS = DYLD_INSERT_LIBRARIES=__FLOX_CLI_OUTPATH__/lib/libsandbox.dylib
else
  ifeq (Linux,$(OS))
    PRELOAD_ARGS = LD_PRELOAD=__FLOX_CLI_OUTPATH__/lib/libsandbox.so
  else
    $(error unknown OS: $(OS))
  endif
endif

# The following template renders targets for the in-situ build mode.
define BUILD_local_template =
  .INTERMEDIATE: $(_pname)_local_build
  $(_pname)_local_build: $($(_pvarname)_buildScript)
	@echo "Building $(_name) in local mode"
	$(if $(_virtualSandbox),$(PRELOAD_ARGS) FLOX_SRC_DIR=$$$$(pwd) FLOX_VIRTUAL_SANDBOX=$(strip $(_virtualSandbox))) \
	MAKEFLAGS= FLOX_TURBO=1 out=$(_out) $(FLOX_ENV)/activate bash -e $($(_pvarname)_buildScript)
	nix --extra-experimental-features nix-command \
	  build -L --file __FLOX_CLI_OUTPATH__/libexec/build-manifest.nix \
	    --argstr name "$(_name)" \
	    --argstr flox-env "$(FLOX_ENV)" \
	    --argstr install-prefix "$(_out)" \
	    $(if $(_virtualSandbox),--argstr virtualSandbox "$(strip $(_virtualSandbox))") \
	    --out-link "result-$(_pname)" \
	    --offline 2>&1 | tee $($(_pvarname)_logfile)

endef

# The following template renders targets for the sandbox build mode.
define BUILD_sandbox_template =
  # Again, it is expected that the sandbox and caching modes will be specified
  # on a per-build basis within the manifest, but in the meantime while we wait
  # for the manifest parser to be implemented we will grep for the explicit
  # "buildCache" setting within the build script itself. (See below)
  $(eval _do_buildCache = $(if $(shell grep -E '\.buildCache = true$$' $(build)),true))

  # The sourceTarball value needs to be stable when nothing changes across builds,
  # so we create a tarball at a stable TMPDIR path and pass that to the derivation
  # instead.
  $(eval $(_pvarname)_src_tar = $($(_pvarname)_tmpBasename)-src.tar)
  $($(_pvarname)_src_tar): FORCE
	tar -cf - --no-recursion -T <(git ls-files) > $$@

  # The buildCache value needs to be similarly stable when nothing changes across
  $(eval $(_pvarname)_buildCache = $($(_pvarname)_tmpBasename)-buildCache.tar)
  $($(_pvarname)_buildCache): FORCE
	-rm -f $$@
	@# If a previous buildCache exists, then copy, don't link to the
	@# previous buildCache because we want nix to import it as a
	@# content-addressed input rather than an ever-changing series
	@# of storePaths. And if it does not exist, then create a new
	@# tarball containing only a single file indicating the time that
	@# the buildCache was created to differentiate it from other
	@# prior otherwise-empty buildCaches.
	@if [ -f "$(_result)-buildCache" ]; then \
	  cp $(_result)-buildCache $$@; \
	else \
	  tmpdir=$$$$(mktemp -d); \
	  echo "Build cache initialized on $$$$(date)" > $$$$tmpdir/.buildCache.init; \
	  tar -cf $$@ -C $$$$tmpdir .buildCache.init; \
	  rm -rf $$$$tmpdir; \
	fi

  .PHONY: $(_pname)_sandbox_build
  $(_pname)_sandbox_build: $($(_pvarname)_buildScript) $($(_pvarname)_src_tar) \
		$(if $(_do_buildCache),$($(_pvarname)_buildCache))
	@echo "Building $(_name) in sandbox mode"
	@# If a previous buildCache exists then move it out of the way
	@# so that we can detect later if it has been updated.
	@if [ -n "$(_do_buildCache)" ] && [ -f "$(_result)-buildCache" ]; then \
	  rm -f "$(_result)-buildCache.prevOutPath"; \
	  readlink "$(_result)-buildCache" > "$(_result)-buildCache.prevOutPath"; \
	fi
	nix --extra-experimental-features nix-command \
	  build -L --file __FLOX_CLI_OUTPATH__/libexec/build-manifest.nix \
	    --argstr name "$(_name)" \
	    --argstr srcTarball "$($(_pvarname)_src_tar)" \
	    --argstr flox-env "$(FLOX_ENV)" \
	    --argstr install-prefix "$(_out)" \
	    $(if $($(_pvarname)_buildDeps),--arg buildDeps $($(_pvarname)_buildDeps_arg)) \
	    --argstr buildScript "$($(_pvarname)_buildScript)" \
	    $(if $(_do_buildCache),--argstr buildCache "$($(_pvarname)_buildCache)") \
	    $(if $(_virtualSandbox),--argstr virtualSandbox "$(strip $(_virtualSandbox))") \
	    --out-link "result-$(_pname)" \
	    '^*' 2>&1 | tee $($(_pvarname)_logfile)
	@# Check to see if a new buildCache has been created, and if so then go
	@# ahead and run 'nix store delete' on the previous cache, keeping in
	@# mind that the symlink will remain unchanged in the event of an
	@# unsuccessful build.
	@if [ -n "$(_do_buildCache)" ]; then \
	  if [ -f "$(_result)-buildCache" ] && [ -f "$(_result)-buildCache.prevOutPath" ]; then \
	    if [ $$$$(readlink "$(_result)-buildCache") != $$$$(cat "$(_result)-buildCache.prevOutPath") ]; then \
	      nix --extra-experimental-features nix-command store delete \
	        $$$$(cat "$(_result)-buildCache.prevOutPath") >/dev/null 2>&1 || true; \
	    fi; \
	  fi; \
	  rm -f "$(_result)-buildCache.prevOutPath"; \
	fi

endef

define BUILD_template =
  # build mode passed as $(1)
  $(eval _build_mode = $(1))
  # Infer pname from script path.)
  $(eval _pname = $(notdir $(build)))
  # We want to create build-specific variables, and variable names cannot
  # have "-" in them so we create a version of the build "pname" replacing
  # this with "_" for use in variable names.
  $(eval _pvarname = $(subst -,_,$(_pname)))
  # Identify result symlink basename.
  $(eval _result = result-$(_pname))
  # Eventually derive version somehow, but hardcode it in the meantime.
  $(eval _version = 0.0.0)
  # Calculate name.
  $(eval _name = $(_pname)-$(_version))
  # Short variable name for buildDependencies derived in the DEPENDS step.
  $(eval $(_pvarname)_buildDeps_arg = $(strip \
    $(if $($(_pvarname)_buildDeps),\
      '["$(subst $(space),",$($(_pvarname)_buildDeps))"]')))

  # Set temp outpath of same strlen as eventual package storePath using the
  # 32-char hash previously derived from the package name, current working
  # directory and FLOX_ENV.
  $(eval _out = /tmp/store_$($(_pvarname)_hash)-$(_name))

  # By the time this rule will be evaluated all of its package dependencies
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

  # Insert mode-specific template.
  $(call BUILD_$(_build_mode)_template)

  # Select the desired build mode as we declare the result symlink target.
  $(_result): $(_pname)_$(_build_mode)_build
	@# Take this opportunity to fail the build if we spot fatal errors in the log.
	@if grep -q "flox build failed (caching build dir)" $($(_pvarname)_logfile); then \
	  echo "ERROR: flox build failed (see $($(_pvarname)_logfile))" 1>&2; \
	  rm -f $$@; \
	  exit 1; \
	fi

  # Create a helper target for referring to the package by its name rather
  # than the [real] result symlink we're looking to create.
  .PHONY: $(_pname)
  $(_pname): $(_result)

  # Accumulate a list of known build targets for the "all" target.
  all += $(_result)
endef

# It is expected that the sandbox and caching modes will be specified on a
# per-build basis within the manifest, but in the meantime while we wait for
# the manifest parser to be implemented we will grep for explicit "buildCache"
# and "sandbox" settings within the build script for setting the build and
# caching modes.
$(foreach build,$(BUILDS), \
  $(eval _virtualSandbox = $(shell grep -E '\.virtual-sandbox = ' $(build) | cut -d= -f2)) \
  $(if $(shell grep -E '\.sandbox = true$$' $(build)), \
    $(eval $(call BUILD_template,sandbox)), \
    $(eval $(call BUILD_template,local))))

# Finally, we create the "all" target to build all known packages.
.PHONY: all
all: $(all)

.PHONY: FORCE
FORCE:
