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

# Start by checking that the FLOX_ENV environment variable is set and that
# we find the expected manifest.lock file in the FLOX_ENV directory.
ifeq (,$(FLOX_ENV))
  $(error FLOX_ENV not defined)
endif
MANIFEST_LOCK := $(FLOX_ENV)/manifest.lock
ifeq (,$(wildcard $(MANIFEST_LOCK)))
  $(error $(MANIFEST_LOCK) not found)
endif

# Substitute Nix store paths for packages required by this Makefile.
__bashInteractive := @bashInteractive@
__coreutils := @coreutils@
__gitMinimal := @gitMinimal@
__gnugrep := @gnugrep@
__gnused := @gnused@
__gnutar := @gnutar@
__jq := @jq@
__nix := @nix@

# Access all required utilities by way of variables so that we don't depend
# on anything from the user's PATH in the packaged version of flox. Note that
# the __package_bin macro defined below will first test that the Nix package
# substitution was successful, and if not then it will fall back to finding
# the required tool from the PATH for use in the developer environment.
__package_bin = $(if $(filter @%@,$(1)),$(2),$(1)/bin/$(2))
_bash := $(call __package_bin,$(__bashInteractive),bash)
_cp := $(call __package_bin,$(__coreutils),cp)
_cut := $(call __package_bin,$(__coreutils),cut)
_git := $(call __package_bin,$(__gitMinimal),git)
_grep := $(call __package_bin,$(__gnugrep),grep)
_jq := $(call __package_bin,$(__jq),jq)
_mktemp := $(call __package_bin,$(__coreutils),mktemp)
_nix := $(call __package_bin,$(__nix),nix)
_readlink := $(call __package_bin,$(__coreutils),readlink)
_rm := $(call __package_bin,$(__coreutils),rm)
_sed := $(call __package_bin,$(__gnused),sed)
_sha256sum := $(call __package_bin,$(__coreutils),sha256sum)
_tar := $(call __package_bin,$(__gnutar),tar)
_uname := $(call __package_bin,$(__coreutils),uname)

# Identify path to build-manifest.nix, in same directory as this Makefile.
_libexec_dir := $(realpath $(dir $(lastword $(MAKEFILE_LIST))))
ifeq (,$(wildcard $(_libexec_dir)))
  $(error cannot identify flox-package-builder libexec directory)
endif

# Invoke nix with the required experimental features enabled.
_nix := $(_nix) --extra-experimental-features "flakes nix-command"

# Ensure we use the Nix-provided SHELL.
SHELL := $(_bash)

# Identify target O/S.
OS := $(shell $(_uname) -s)

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
    ( echo $(_pname) $(realpath $(FLOX_ENV)) && pwd ) | $(_sha256sum) | head -c32)))
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
    $(if $(shell $(_grep) '\$${$(package)}' $(build)),\
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
  PRELOAD_ARGS = DYLD_INSERT_LIBRARIES=$(_libexec_dir)/libsandbox.dylib
else
  ifeq (Linux,$(OS))
    PRELOAD_ARGS = LD_PRELOAD=$(_libexec_dir)/libsandbox.so
  else
    $(error unknown OS: $(OS))
  endif
endif

# The following template renders targets for the in-situ build mode.
define BUILD_local_template =
  $(eval _virtualSandbox = $(filter-out off,$(_sandbox)))

  .INTERMEDIATE: $(_pname)_local_build
  $(_pname)_local_build: $($(_pvarname)_buildScript)
	@echo "Building $(_name) in local mode"
	$(if $(_virtualSandbox),$(PRELOAD_ARGS) FLOX_SRC_DIR=$$$$(pwd) FLOX_VIRTUAL_SANDBOX=$(_sandbox)) \
	MAKEFLAGS= out=$(_out) $(FLOX_ENV)/activate --turbo -- $(_bash) -e $($(_pvarname)_buildScript)
	set -o pipefail && $(_nix) build -L --file $(_libexec_dir)/build-manifest.nix \
	    --argstr name "$(_name)" \
	    --argstr flox-env "$(FLOX_ENV)" \
	    --argstr install-prefix "$(_out)" \
	    --out-link "result-$(_pname)" \
	    --offline 2>&1 | tee $($(_pvarname)_logfile)

endef

# The following template renders targets for the sandbox build mode.
define BUILD_nix_sandbox_template =
  # Again, it is expected that the sandbox and caching modes will be specified
  # on a per-build basis within the manifest, but in the meantime while we wait
  # for the manifest parser to be implemented we will grep for the explicit
  # "buildCache" setting within the build script itself. (See below)
  $(eval _do_buildCache = true)

  # The sourceTarball value needs to be stable when nothing changes across builds,
  # so we create a tarball at a stable TMPDIR path and pass that to the derivation
  # instead.
  $(eval $(_pvarname)_src_tar = $($(_pvarname)_tmpBasename)-src.tar)
  $($(_pvarname)_src_tar): FORCE
	$(_tar) -cf - --no-recursion -T <($(_git) ls-files) > $$@

  # The buildCache value needs to be similarly stable when nothing changes across
  $(eval $(_pvarname)_buildCache = $($(_pvarname)_tmpBasename)-buildCache.tar)
  $($(_pvarname)_buildCache): FORCE
	-$(_rm) -f $$@
	@# If a previous buildCache exists, then copy, don't link to the
	@# previous buildCache because we want nix to import it as a
	@# content-addressed input rather than an ever-changing series
	@# of storePaths. And if it does not exist, then create a new
	@# tarball containing only a single file indicating the time that
	@# the buildCache was created to differentiate it from other
	@# prior otherwise-empty buildCaches.
	@if [ -f "$(_result)-buildCache" ]; then \
	  $(_cp) $(_result)-buildCache $$@; \
	else \
	  tmpdir=$$$$($(_mktemp) -d); \
	  echo "Build cache initialized on $$$$(date)" > $$$$tmpdir/.buildCache.init; \
	  $(_tar) -cf $$@ -C $$$$tmpdir .buildCache.init; \
	  $(_rm) -rf $$$$tmpdir; \
	fi

  .PHONY: $(_pname)_nix_sandbox_build
  $(_pname)_nix_sandbox_build: $($(_pvarname)_buildScript) $($(_pvarname)_src_tar) \
		$(if $(_do_buildCache),$($(_pvarname)_buildCache))
	@echo "Building $(_name) in Nix sandbox (pure) mode"
	@# If a previous buildCache exists then move it out of the way
	@# so that we can detect later if it has been updated.
	@if [ -n "$(_do_buildCache)" ] && [ -f "$(_result)-buildCache" ]; then \
	  $(_rm) -f "$(_result)-buildCache.prevOutPath"; \
	  $(_readlink) "$(_result)-buildCache" > "$(_result)-buildCache.prevOutPath"; \
	fi
	set -o pipefail && $(_nix) build -L --file $(_libexec_dir)/build-manifest.nix \
	    --argstr name "$(_name)" \
	    --argstr srcTarball "$($(_pvarname)_src_tar)" \
	    --argstr flox-env "$(FLOX_ENV)" \
	    --argstr install-prefix "$(_out)" \
	    $(if $($(_pvarname)_buildDeps),--arg buildDeps $($(_pvarname)_buildDeps_arg)) \
	    --argstr buildScript "$($(_pvarname)_buildScript)" \
	    $(if $(_do_buildCache),--argstr buildCache "$($(_pvarname)_buildCache)") \
	    --out-link "result-$(_pname)" \
	    '^*' 2>&1 | tee $($(_pvarname)_logfile)
	@# Check to see if a new buildCache has been created, and if so then go
	@# ahead and run 'nix store delete' on the previous cache, keeping in
	@# mind that the symlink will remain unchanged in the event of an
	@# unsuccessful build.
	@if [ -n "$(_do_buildCache)" ]; then \
	  if [ -f "$(_result)-buildCache" ] && [ -f "$(_result)-buildCache.prevOutPath" ]; then \
	    if [ $$$$($(_readlink) "$(_result)-buildCache") != $$$$(cat "$(_result)-buildCache.prevOutPath") ]; then \
	      $(_nix) store delete \
	        $$$$(cat "$(_result)-buildCache.prevOutPath") >/dev/null 2>&1 || true; \
	    fi; \
	  fi; \
	  $(_rm) -f "$(_result)-buildCache.prevOutPath"; \
	fi

endef

define BUILD_template =
  # build mode passed as $(1)
  $(eval _build_mode = $(1))
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
  # Variable for providing buildDependencies derived in the DEPENDS step
  # to the Nix expression as a safely-quoted string.
  $(eval $(_pvarname)_buildDeps_arg = $(strip \
    $(if $($(_pvarname)_buildDeps),\
      '["$(subst $(space)," ",$($(_pvarname)_buildDeps))"]')))

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
	@$(_cp) $$< $$@
	@for i in $$^; do \
	  if [ -L "$$$$i" ]; then \
	    outpath="$$$$($(_readlink) $$$$i)"; \
	    if [ -n "$$$$outpath" ]; then \
	      pkgname="$$$$(echo $$$$i | $(_cut) -d- -f2-)"; \
	      $(_sed) -i "s%\$$$${$$$$pkgname}%$$$$outpath%g" $$@; \
	    fi; \
	  fi; \
	done

  # Prepare temporary log file for capturing build output for inspection.
  $(eval $(_pvarname)_logfile := $(shell $(_mktemp) --dry-run --suffix=-build-$(_pname).log))

  # Insert mode-specific template.
  $(call BUILD_$(_build_mode)_template)

  # Select the desired build mode as we declare the result symlink target.
  $(_result): $(_pname)_$(_build_mode)_build
	@# Take this opportunity to fail the build if we spot fatal errors in the log.
	@if $(_grep) -q "flox build failed (caching build dir)" $($(_pvarname)_logfile); then \
	  echo "ERROR: flox build failed (see $($(_pvarname)_logfile))" 1>&2; \
	  $(_rm) -f $$@; \
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
  $(eval _pname = $(notdir $(build))) \
  $(eval _sandbox = $(shell \
    $(_jq) -r '.manifest.build."$(_pname)".sandbox' $(MANIFEST_LOCK))) \
  $(if $(filter null pure,$(_sandbox)), \
    $(eval $(call BUILD_template,nix_sandbox)), \
    $(eval $(call BUILD_template,local))))

# Finally, we create the "all" target to build all known packages.
.PHONY: all
all: $(all)

.PHONY: FORCE
FORCE:
