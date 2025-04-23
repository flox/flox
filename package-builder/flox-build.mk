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
__daemonize := @daemonize@
__gitMinimal := @gitMinimal@
__gnugrep := @gnugrep@
__gnused := @gnused@
__gnutar := @gnutar@
__jq := @jq@
__nix := @nix@
__t3 := @t3@

# Access all required utilities by way of variables so that we don't depend
# on anything from the user's PATH in the packaged version of flox. Note that
# the __package_bin macro defined below will first test that the Nix package
# substitution was successful, and if not then it will fall back to finding
# the required tool from the PATH for use in the developer environment.
__package_bin = $(if $(filter @%@,$(1)),$(2),$(1)/bin/$(2))
_bash := $(call __package_bin,$(__bashInteractive),bash)
_cat := $(call __package_bin,$(__coreutils),cat)
_cp := $(call __package_bin,$(__coreutils),cp)
_cut := $(call __package_bin,$(__coreutils),cut)
_daemonize := $(call __package_bin,$(__daemonize),daemonize)
_env := $(call __package_bin,$(__coreutils),env)
_git := $(call __package_bin,$(__gitMinimal),git)
_grep := $(call __package_bin,$(__gnugrep),grep)
_head := $(call __package_bin,$(__coreutils),head)
_jq := $(call __package_bin,$(__jq),jq)
_mktemp := $(call __package_bin,$(__coreutils),mktemp)
_mv := $(call __package_bin,$(__coreutils),mv)
_nix := $(call __package_bin,$(__nix),nix)
_pwd := $(call __package_bin,$(__coreutils),pwd)
_readlink := $(call __package_bin,$(__coreutils),readlink)
_realpath := $(call __package_bin,$(__coreutils),realpath)
_rm := $(call __package_bin,$(__coreutils),rm)
_sed := $(call __package_bin,$(__gnused),sed)
_sha256sum := $(call __package_bin,$(__coreutils),sha256sum)
_tar := $(call __package_bin,$(__gnutar),tar)
_t3 := $(call __package_bin,$(__t3),t3) --relative $(if $(NO_COLOR),,--forcecolor)
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
.DEFAULT_GOAL := usage

# Set a default TMPDIR variable if one is not already defined.
TMPDIR ?= /tmp

# Use the wildcard operator to identify builds in the provided $FLOX_ENV.
BUILDS := $(wildcard $(FLOX_ENV)/package-builds.d/*)

# Quick sanity check; if no BUILDS then what are we doing?
$(if $(BUILDS),,$(error no packages found in $(FLOX_ENV)/package-builds.d))

# Set makefile verbosity based on the value of _FLOX_PKGDB_VERBOSITY [sic]
# as set in the environment by the flox CLI. First set it to 0 if not defined.
ifeq (,$(_FLOX_PKGDB_VERBOSITY))
  _FLOX_PKGDB_VERBOSITY = 0
endif
# Then set them to empty string or "@" based on being greater than 0, 1, or 2.
$(eval _V_ = $(intcmp 0,$(_FLOX_PKGDB_VERBOSITY),,@))
$(eval _VV_ = $(intcmp 1,$(_FLOX_PKGDB_VERBOSITY),,@))
$(eval _VVV_ = $(intcmp 2,$(_FLOX_PKGDB_VERBOSITY),,@))

# Define a usage target to provide a helpful message when no target is specified.
.PHONY: usage
usage:
	@echo "Usage: make -f $(lastword $(MAKEFILE_LIST)) [TARGET]"
	@echo "Targets:"
	@echo "  build: build all packages"
	@echo "  build/[pname]: build the specified package"
	@echo "  clean: clean all build artifacts"
	@echo "  clean/[pname]: clean build artifacts for the specified package"

# The `nix build` command will attempt a rebuild in every instance,
# and we will presumably want `flox build` to do the same. However,
# we cannot just mark the various build targets as PHONY because they
# must be INTERMEDIATE to prevent `flox build foo` from rebuilding
# `bar` and `baz` as well (unless of course it was a prerequisite).
# So we instead derive the packages to be force-rebuilt from the special
# MAKECMDGOALS variable if defined, and otherwise rebuild them all.
BUILDGOALS = $(if $(MAKECMDGOALS),$(MAKECMDGOALS),$(notdir $(BUILDS)))
$(foreach _build,$(BUILDGOALS),\
  $(eval _pname = $(notdir $(_build)))\
  $(eval _pvarname = $(subst -,_,$(_pname)))\
  $(foreach _buildtype,local sandbox,\
    $(eval $(_pvarname)_$(_buildtype)_build: FORCE)))

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
    ( echo $(_pname) && $(_pwd) ) | $(_sha256sum) | $(_head) -c32)))
  # Render a shorter 8-character version as well.
  $(eval $(_pvarname)_shortHash = $(shell echo $($(_pvarname)_hash) | $(_head) -c8))
  # And while we're at it, set a temporary basename using the short hash.
  $(eval $(_pvarname)_tmpBasename = $(TMPDIR)/$($(_pvarname)_shortHash)-$(_pname))

  # Create a target for cleaning up the temporary directory.
  .PHONY: clean/$(_pname)
  clean/$(_pname):
	-$(_rm) -rf $($(_pvarname)_tmpBasename)

  clean_targets += clean/$(_pname)

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
      $(eval $(_pvarname)_buildDeps += $(shell $(_realpath) $(_dep)))\
      $($(_pvarname)_buildScript): $(_dep)))

  # Prepare temporary file for constructing the JSON output to be returned
  # from the build.
  $(eval $(_pvarname)_buildMetaJSON = $($(_pvarname)_tmpBasename)-build-meta.json)
endef

$(foreach build,$(BUILDS),$(eval $(call DEPENDS_template)))

# Define macro containing single space character for use in string substitution.
space := $(subst x,,x x)

# The method of calling the sandbox differs based on O/S. Define
# PRELOAD_VARS to denote the correct way.
ifeq (Darwin,$(OS))
  PRELOAD_VARS = DYLD_INSERT_LIBRARIES=$(_libexec_dir)/libsandbox.dylib
else
  ifeq (Linux,$(OS))
    PRELOAD_VARS = LD_PRELOAD=$(_libexec_dir)/libsandbox.so
  else
    $(error unknown OS: $(OS))
  endif
endif

# Define a template target for cleaning up result symlinks and their
# associated storePaths, if they exist.
define CLEAN_result_link_template =
  # Note that this template is evaluated at Makefile compilation time,
  # but is only called for the clean target, for which that's
  # a fine time to test for the existence of symlinks and storepaths,
  # so we can use GNU make functions to interrogate the filesystem
  # and create nicely formatted targets customized for each result link.

  # The builtin realpath function returns the empty string when the
  # result is a dangling symlink.
  $(eval _storePath = $(realpath $(1)))

  .PHONY: clean_result_link/$(1)
  clean_result_link/$(1):
	-$(_rm) -f $(1)

  .PHONY: clean_result_storepath/$(1)
  clean_result_storepath/$(1): clean_result_link/$(1)
	$(_daemonize) $(_nix) store delete $(_storePath)

  clean/$(_pname): clean_result_link/$(1) \
    $(if $(_storePath),clean_result_storepath/$(1))
endef

# The following env vars need to be passed from the outer "develop" environment
# to the inner "build wrapper" environment in the in-situ build mode in support
# of the tools and compilers found in the outer environment, and for Flox to
# otherwise function properly.
ALLOW_OUTER_ENV_VARS := FLOX_RUNTIME_DIR HOME PATH \
  $(filter NIX_CFLAGS% NIX_CC%,$(.VARIABLES))

# The following template renders targets for the in-situ build mode.
define BUILD_local_template =
  $(eval _virtualSandbox = $(filter-out null off,$(_sandbox)))

  # Set temp outpath of same strlen as eventual package storePath using the
  # 32-char hash previously derived from the package name, current working
  # directory and FLOX_ENV.
  $(eval _out = /tmp/store_$($(_pvarname)_hash)-$(_name))

  # Prepare temporary log file for capturing build output for inspection.
  $(eval $(_pvarname)_logfile := $(shell $(_mktemp) --dry-run --suffix=-build-$(_pname).log))

  # Our aim in performing a manifest build is to replicate as closely as possible
  # the experience of running those same build script commands from within an
  # interactive `flox activate -m dev` shell (i.e. using the "develop" environment).

  # But unlike the interactive case, the manifest build seeks to use dependencies
  # as found in the target package's "wrapper" environment in preference to those
  # found in the "develop" environment. It does this for the express purpose of
  # preventing the resulting closure from depending on the "develop" environment,
  # which can contain compilers, libraries and tools not required at runtime.

  # The way it does this is by performing the build within a nested activation of
  # each of the "develop" and "wrapper" environments:
  # * the [outer] "develop" environment is activated first, providing access to
  #    runtime dependencies and compilers/tools required only at build time
  # * then the [inner] "wrapper" environment is activated, providing access
  #    to only those runtime dependencies

  # The only issue with the above approach is that references to the "develop"
  # environment can leak into the resulting build. For example, each of these
  # activations can prepend to a PYTHONPATH that gets embedded in the
  # build, which has the effect of pulling both environments into the closure.

  # We can use `env -i` to prevent that leakage of the "develop" environment
  # path into the inner activation, but then that causes problems for compilers
  # that rely on NIX_CC* environment variables set in the outer activation. To
  # address this problem we maintain a list of ALLOW_OUTER_ENV_VARS allowed
  # to be propagated from the outer to the inner activation, and again use the
  # `env` command to let those through.

  # The final result is approximately the following:
  #   $(FLOX_INTERPRETER)/activate ... -- \
  #     env -i $(foreach i,$(ALLOW_OUTER_ENV_VARS),$(i)="$$$$$(i)") \
  #       $(_build_wrapper_env)/wrapper ... -- bash -e buildScript
  .INTERMEDIATE: $(_pvarname)_local_build
  $(_pvarname)_local_build: $($(_pvarname)_buildScript)
	@# $(if $(FLOX_INTERPRETER),,$$(error FLOX_INTERPRETER not defined))
	@echo "Building $(_name) in local mode"
	$(_VV_) $(_rm) -rf $(_out)
	$(_V_) \
	  $(if $(_virtualSandbox),$(PRELOAD_VARS) FLOX_SRC_DIR=$$$$($(_pwd)) FLOX_VIRTUAL_SANDBOX=$(_sandbox)) \
	  $(FLOX_INTERPRETER)/activate --env $(FLOX_ENV) --mode dev --turbo --env-project $$$$($(_pwd)) -- \
	    env out=$(_out) $(_build_wrapper_env)/wrapper --env $(_build_wrapper_env) --set-vars -- \
	      $(_t3) $($(_pvarname)_logfile) -- $(_bash) -e $($(_pvarname)_buildScript)
	$(_V_) $(_nix) build -L `$(_nix) store add-file "$(shell $(_realpath) "$($(_pvarname)_logfile)")"` \
	  --out-link "result-$(_pname)-log"
	$(_V_) set -o pipefail && \
	$(_nix) build -L --file $(_libexec_dir)/build-manifest.nix \
	  --argstr pname "$(_pname)" \
	  --argstr version "$(_version)" \
	  --argstr flox-env "$(FLOX_ENV)" \
	  --argstr build-wrapper-env "$(_build_wrapper_env)" \
	  --argstr install-prefix "$(_out)" \
	  --argstr nixpkgs-url "$(BUILDTIME_NIXPKGS_URL)" \
	  --out-link "result-$(_pname)" \
	  --json '^*' | \
	$(_jq) --arg pname "$(_pname)" --arg version "$(_version)" --arg name "$(_name)" \
	  --arg log "$(shell $(_readlink) result-$(_pname)-log)" \
	  '.[0] * {name:$$$$name, pname:$$$$pname, version:$$$$version, log:$$$$log}' > $($(_pvarname)_buildMetaJSON)
	@echo "Completed build of $(_name) in local mode" && echo ""

endef

# The following template renders targets for the sandbox build mode.
define BUILD_nix_sandbox_template =
  # If set, the DISABLE_BUILDCACHE variable will cause the build to omit the
  # build cache.  This is used for (at least) publish.
  $(eval _do_buildCache = $(if $(DISABLE_BUILDCACHE),,true))

  # The sourceTarball value needs to be stable when nothing changes across builds,
  # so we create a tarball at a stable TMPDIR path and pass that to the derivation
  # instead.
  $(eval $(_pvarname)_src_tar = $($(_pvarname)_tmpBasename)-src.tar)
  $($(_pvarname)_src_tar): FORCE
	@# TIL that you have to explicitly call `wait` to harvest the exit status
	@# of a process substitution, and that `set -o pipefail` does nothing here.
	@# See: https://mywiki.wooledge.org/ProcessSubstitution
	$(_V_) $(_tar) -cf $$@ --no-recursion -T <($(_git) ls-files) && wait "$$$$!"

  # The buildCache value needs to be similarly stable when nothing changes across
  $(eval $(_pvarname)_buildCache = $($(_pvarname)_tmpBasename)-buildCache.tar)
  $($(_pvarname)_buildCache): FORCE
	-$(_V_) $(_rm) -f $$@
	@# If a previous buildCache exists, then copy, don't link to the
	@# previous buildCache because we want nix to import it as a
	@# content-addressed input rather than an ever-changing series
	@# of storePaths. And if it does not exist, then create a new
	@# tarball containing only a single file indicating the time that
	@# the buildCache was created to differentiate it from other
	@# prior otherwise-empty buildCaches.
	$(_VV_) if [ -f "$(_result)-buildCache" ]; then \
	  $(_cp) $(_result)-buildCache $$@; \
	else \
	  tmpdir=$$$$($(_mktemp) -d); \
	  echo "Build cache initialized on $$$$(date)" > $$$$tmpdir/.buildCache.init; \
	  $(_tar) -cf $$@ -C $$$$tmpdir .buildCache.init; \
	  $(_rm) -rf $$$$tmpdir; \
	fi

  # Create a target for cleaning up the buildCache result symlink.
  $(eval $(call CLEAN_result_link_template,$(_result)-buildCache))

  .PHONY: $(_pvarname)_nix_sandbox_build
  $(_pvarname)_nix_sandbox_build: $($(_pvarname)_buildScript) $($(_pvarname)_src_tar) \
		$(if $(_do_buildCache),$($(_pvarname)_buildCache))
	@echo "Building $(_name) in Nix sandbox (pure) mode"
	@# If a previous buildCache exists then move it out of the way
	@# so that we can detect later if it has been updated.
	$(_VV_) if [ -n "$(_do_buildCache)" ] && [ -f "$(_result)-buildCache" ]; then \
	  $(_rm) -f "$(_result)-buildCache.prevOutPath"; \
	  $(_readlink) "$(_result)-buildCache" > "$(_result)-buildCache.prevOutPath"; \
	fi
	$(_V_) set -o pipefail && \
	$(_nix) build -L --file $(_libexec_dir)/build-manifest.nix \
	  --argstr pname "$(_pname)" \
	  --argstr version "$(_version)" \
	  --argstr srcTarball "$($(_pvarname)_src_tar)" \
	  --argstr flox-env "$(FLOX_ENV)" \
	  --argstr build-wrapper-env "$(_build_wrapper_env)" \
	  --argstr nixpkgs-url "$(BUILDTIME_NIXPKGS_URL)" \
	  $(if $($(_pvarname)_buildDeps),--arg buildDeps $($(_pvarname)_buildDeps_arg)) \
	  --argstr buildScript "$($(_pvarname)_buildScript)" \
	  $(if $(_do_buildCache),--argstr buildCache "$($(_pvarname)_buildCache)") \
	  --out-link "result-$(_pname)" \
	  --json '^*' | \
	$(_jq) --arg pname "$(_pname)" --arg version "$(_version)" --arg name "$(_name)" \
	  '.[0] * {name:$$$$name, pname:$$$$pname, version:$$$$version, log:.[0].outputs.log}' > $($(_pvarname)_buildMetaJSON)
	@echo "Completed build of $(_name) in Nix sandbox mode" && echo ""
	@# Check to see if a new buildCache has been created, and if so then go
	@# ahead and run 'nix store delete' on the previous cache, keeping in
	@# mind that the symlink will remain unchanged in the event of an
	@# unsuccessful build.
	$(_VV_) if [ -n "$(_do_buildCache)" ]; then \
	  if [ -f "$(_result)-buildCache" ] && [ -f "$(_result)-buildCache.prevOutPath" ]; then \
	    if [ $$$$($(_readlink) "$(_result)-buildCache") != $$$$(cat "$(_result)-buildCache.prevOutPath") ]; then \
	      $(_daemonize) $(_nix) store delete \
	        $$$$(cat "$(_result)-buildCache.prevOutPath"); \
	    fi; \
	  fi; \
	  $(_rm) -f "$(_result)-buildCache.prevOutPath"; \
	fi

endef

# Verify certain prerequisites before kicking off the build DAG.
.PHONY: check_BUILD_PREREQUISITES
check_BUILD_PREREQUISITES: FORCE
	@# The BUILD_RESULT_FILE must be defined and exist.
	@$(if $(BUILD_RESULT_FILE), \
	  $(if $(wildcard $(BUILD_RESULT_FILE)),-, \
	    $$(error $(BUILD_RESULT_FILE) not found)), \
	  $$(error BUILD_RESULT_FILE not defined))
	@# Check that the BUILDTIME_NIXPKGS_URL is defined.
	@$(if $(BUILDTIME_NIXPKGS_URL),-,$(error BUILDTIME_NIXPKGS_URL not defined))

define BUILD_template =

  # Identify the build wrapper environment with which to wrap the contents
  # of bin, sbin.
  $(eval _build_wrapper_env = $$(strip \
    $(if $(FLOX_ENV_OUTPUTS), \
      $$(shell $(_jq) -n -r \
        --argjson results '$$(FLOX_ENV_OUTPUTS)' \
        '$$$$results."build-$(_pname)"') \
      $$(if $$(filter 0,$$(.SHELLSTATUS)),,$$(error could not identify build wrapper env for $(_pname))), \
      $$$$(error FLOX_ENV_OUTPUTS not defined))))

  # Take this opportunity to evaluate the "file" and "command" forms of
  # the "version" string.
  $(eval _vertype = $(firstword $(subst :, ,$(strip $(_version)))))
  $(eval _version = $(strip \
    $(if $(filter file,$(_vertype)),$(file <$(subst file:,,$(_version))), \
      $(if $(filter command,$(_vertype)),$(shell $(subst command:,,$(_version))), \
	$(_version)))))

  # build mode passed as $(1)
  $(eval _build_mode = $(1))
  # We want to create build-specific variables, and variable names cannot
  # have "-" in them so we create a version of the build "pname" replacing
  # this with "_" for use in variable names.
  $(eval _pvarname = $(subst -,_,$(_pname)))
  # Identify result symlink basename.
  $(eval _result = result-$(_pname))
  # Calculate name.
  $(eval _name = $(_pname)-$(_version))
  # Variable for providing buildDependencies derived in the DEPENDS step
  # to the Nix expression as a safely-quoted string.
  $(eval $(_pvarname)_buildDeps_arg = $(strip \
    $(if $($(_pvarname)_buildDeps),\
      '["$(subst $(space)," ",$($(_pvarname)_buildDeps))"]')))

  # By the time this rule will be evaluated all of its package dependencies
  # will have been added to the set of rule prerequisites in $^, using their
  # "safe" name (with "-" characters replaced with "_"), and these targets
  # will have successfully built the corresponding result-$(_pname) symlinks.
  # Iterate through this list, replacing all instances of "${package}" with the
  # corresponding storePath as identified by the result-* symlink.
  .PRECIOUS: $($(_pvarname)_buildScript)
  $($(_pvarname)_buildScript): $(build) check_BUILD_PREREQUISITES
	@echo "Rendering $(_pname) build script to $$@"
	@# Always echo lines in the build script as they are invoked.
	$(_VV_) echo "set -x" > $$@.new
	$(_VV_) $(_cat) $$< >> $$@.new
	$(_VV_) for i in $$^; do \
	  if [ "$$$$i" != "check_BUILD_PREREQUISITES" -a -L "$$$$i" ]; then \
	    outpath="$$$$($(_readlink) $$$$i)"; \
	    if [ -n "$$$$outpath" ]; then \
	      pkgname="$$$$(echo $$$$i | $(_cut) -d- -f2-)"; \
	      $(_sed) -i "s%\$$$${$$$$pkgname}%$$$$outpath%g" $$@.new; \
	    fi; \
	  fi; \
	done
	$(_VV_) $(_mv) -f $$@.new $$@

  # Insert mode-specific template.
  $(call BUILD_$(_build_mode)_template)

  # Select the desired build mode as we declare the result symlink target.
  $(_result): $(_pvarname)_$(_build_mode)_build
	@# Take this opportunity to fail the build if we spot fatal errors in the
	@# build output. Recall that we force the Nix build to "succeed" in all
	@# cases so that we can persist the buildCache, so when errors do happen
	@# this is communicated by way of a $out that is 1) a file and 2) contains
	@# the string "flox build failed (caching build dir)".
	$(_VV_) if [ -f $(_result) ] && $(_grep) -q "flox build failed (caching build dir)" $(_result); then \
	  echo "ERROR: flox build failed (see $(_result)-log)" 1>&2; \
	  $(_rm) -f $$@; \
	  exit 1; \
	fi

  # Note that the buildMetaJSON file is created as a side-effect of the build.
  $($(_pvarname)_buildMetaJSON): $(_result)

  # Create targets for cleaning up the result and log symlinks.
  $(eval $(call CLEAN_result_link_template,$(_result)))
  $(eval $(call CLEAN_result_link_template,$(_result)-log))

  # Create a helper target for referring to the package by its name rather
  # than the [real] result symlink we're looking to create.
  .PHONY: build/$(_pname)
  build/$(_pname): $(_result)

endef

# Glean various values from locked manifest as we call the template.
$(foreach build,$(BUILDS), \
  $(eval _pname = $(notdir $(build))) \
  $(eval _sandbox = $(shell \
    $(_jq) -r '.manifest.build."$(_pname)".sandbox' $(MANIFEST_LOCK))) \
  $(eval _version = $(shell \
    $(_jq) -r '.manifest.build."$(_pname)".version // "unknown"' $(MANIFEST_LOCK))) \
  $(if $(filter null off,$(_sandbox)), \
    $(eval $(call BUILD_template,local)), \
    $(eval $(call BUILD_template,nix_sandbox))))

# Combine JSON build data for each build and write to BUILD_RESULT_FILE.
# Mark it as phony to force it to be evaluated every time.
.PHONY: $(BUILD_RESULT_FILE)
$(BUILD_RESULT_FILE): $(foreach pname,$(PACKAGES),$($(subst -,_,$(pname))_buildMetaJSON))
	$(_VV_) [ -n "$^" ] || ( echo "ERROR: PACKAGES not defined or empty" 1>&2; exit 1 )
	$(_VV_) $(_jq) -s . $^ > $@

# Finally, we create the "build" target to invoke the $(BUILD_RESULT_FILE)
# target which has the effect of building all requested $(PACKAGES).
.PHONY: build
build: $(BUILD_RESULT_FILE)

# Add a target for cleaning up the build artifacts.
.PHONY: clean
clean: $(clean_targets)

.PHONY: FORCE
FORCE:
