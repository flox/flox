#
# This makefile implements Tom's stepladder from manifest to Nix builds:

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

# Define macros for use in string substitution.
space := $(subst x,,x x)
comma := ,

# Substitute Nix store paths for packages required by this Makefile.
__bashInteractive := @bashInteractive@
__coreutils := @coreutils@
__cpio := @cpio@
__daemonize := @daemonize@
__findutils := @findutils@
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
_chmod := $(call __package_bin,$(__coreutils),chmod)
_comm := $(call __package_bin,$(__coreutils),comm)
_cp := $(call __package_bin,$(__coreutils),cp)
_cpio := $(call __package_bin,$(__cpio),cpio)
_cut := $(call __package_bin,$(__coreutils),cut)
_daemonize := $(call __package_bin,$(__daemonize),daemonize)
_env := $(call __package_bin,$(__coreutils),env)
_find := $(call __package_bin,$(__findutils),find)
_git := $(call __package_bin,$(__gitMinimal),git)
_grep := $(call __package_bin,$(__gnugrep),grep)
_head := $(call __package_bin,$(__coreutils),head)
_jq := $(call __package_bin,$(__jq),jq)
_mkdir := $(call __package_bin,$(__coreutils),mkdir)
_mktemp := $(call __package_bin,$(__coreutils),mktemp)
_mv := $(call __package_bin,$(__coreutils),mv)
_nix := $(call __package_bin,$(__nix),nix)
_nix_store := $(call __package_bin,$(__nix),nix-store)
_pwd := $(call __package_bin,$(__coreutils),pwd)
_readlink := $(call __package_bin,$(__coreutils),readlink)
_realpath := $(call __package_bin,$(__coreutils),realpath)
_rm := $(call __package_bin,$(__coreutils),rm)
_sed := $(call __package_bin,$(__gnused),sed)
_sha256sum := $(call __package_bin,$(__coreutils),sha256sum)
_sort := $(call __package_bin,$(__coreutils),sort)
_tar := $(call __package_bin,$(__gnutar),tar)
_touch := $(call __package_bin,$(__coreutils),touch)
_t3 := $(call __package_bin,$(__t3),t3) --relative $(if $(NO_COLOR),,--forcecolor)
_tr := $(call __package_bin,$(__coreutils),tr)
_uname := $(call __package_bin,$(__coreutils),uname)

# Identify path to build-manifest.nix, in same directory as this Makefile.
_libexec_dir := $(realpath $(dir $(lastword $(MAKEFILE_LIST))))
ifeq (,$(wildcard $(_libexec_dir)))
  $(error cannot identify flox-package-builder libexec directory)
endif

# Path to Nix expession (NEF) library.
_nef := $(_libexec_dir)/nef

# Set makefile verbosity based on the value of _FLOX_SUBSYSTEM_VERBOSITY [sic]
# as set in the environment by the flox CLI. First set it to 0 if not defined.
ifeq (,$(_FLOX_SUBSYSTEM_VERBOSITY))
  _FLOX_SUBSYSTEM_VERBOSITY = 0
endif

# Invoke nix with the required experimental features enabled.
_nix := $(_nix) --extra-experimental-features "flakes nix-command fetch-tree" $(intcmp 0,$(_FLOX_SUBSYSTEM_VERBOSITY),--trace-verbose)

# Ensure we use the Nix-provided SHELL.
SHELL := $(_bash) $(intcmp 2,$(_FLOX_SUBSYSTEM_VERBOSITY),-x)

# Identify target O/S.
OS := $(shell $(_uname) -s)

# Nix system
# TODO(nef): we might be passing that around differently (or call nef stuff with --impure)
NIX_SYSTEM_CURRENT := $(shell $(_nix) config show system)
ifeq (,$(NIX_SYSTEM))
  NIX_SYSTEM = $(NIX_SYSTEM_CURRENT)
endif

# Set the default goal to be all builds if one is not specified.
.DEFAULT_GOAL := usage

# Set a default TMPDIR variable if one is not already defined.
TMPDIR ?= /tmp

# Record PWD once, to be used throughout the Makefile.
PWD := $(shell $(_pwd))

# Create a project-specific TMPDIR variable so we don't have path clash
# between the same package name built from different project directories.
PROJECT_TMPDIR := $(TMPDIR)/$(shell echo $(PWD) | $(_sha256sum) | $(_head) -c8)

# Use the wildcard operator to identify builds in the provided $FLOX_ENV.
MANIFEST_BUILDS := $(wildcard $(FLOX_ENV)/package-builds.d/*)

# TODO NIX_EXPRESSION_DIR may be absent
ifeq (,$(NIX_EXPRESSION_DIR))
  $(error NIX_EXPRESSION_DIR not defined)
endif

NIX_EXPRESSION_GIT_ROOT := \
  $(shell $(_git) -C '$(NIX_EXPRESSION_DIR)' rev-parse --show-toplevel 2> /dev/null || echo)

NIX_EXPRESSION_GIT_SUBDIR := \
  $(shell $(_git) -C '$(NIX_EXPRESSION_DIR)' rev-parse --show-prefix 2> /dev/null || echo)


ifeq (,$(NIX_EXPRESSION_DIR))
  $(error NIX_EXPRESSION_DIR not defined)
endif

# If there is no git root,
ifeq (,$(NIX_EXPRESSION_GIT_ROOT))
  NIX_EXPRESSION_DIR_ARGS := \
    --arg pkgs-dir '$(NIX_EXPRESSION_DIR)'
else
  NIX_EXPRESSION_DIR_ARGS := \
    --argstr pkgs-dir '$(NIX_EXPRESSION_GIT_ROOT)' \
    --argstr git-subdir '$(NIX_EXPRESSION_GIT_SUBDIR)'
endif


NIX_EXPRESSION_BUILDS := \
  $(shell $(_nix) eval \
    --argstr nixpkgs-url '$(BUILDTIME_NIXPKGS_URL)' \
    --argstr system $(NIX_SYSTEM) \
    $(NIX_EXPRESSION_DIR_ARGS) \
    --file $(_nef) \
    reflect.targets --raw)

# Quick sanity check; if no MANIFEST_BUILDS then what are we doing?
$(if $(MANIFEST_BUILDS),,\
  $(if $(NIX_EXPRESSION_BUILDS),,\
    $(error no manifest or Nix expression builds found in $(FLOX_ENV))))

# Then set them to empty string or "@" based on being greater than 0, 1, or 2.
$(eval _V_ = $(intcmp 0,$(_FLOX_SUBSYSTEM_VERBOSITY),,@))
$(eval _VV_ = $(intcmp 1,$(_FLOX_SUBSYSTEM_VERBOSITY),,@))
$(eval _VVV_ = $(intcmp 2,$(_FLOX_SUBSYSTEM_VERBOSITY),,@))

# Define a usage target to provide a helpful message when no target is specified.
.PHONY: usage
usage:
	@echo "Usage: make -f $(lastword $(MAKEFILE_LIST)) [TARGET]"
	@echo "Targets:"
	@echo "  build: build all packages"
	@echo "  build/[pname]: build the specified package"
	@echo "  clean: clean all build artifacts"
	@echo "  clean/[pname]: clean build artifacts for the specified package"

# Verify certain prerequisites and touch a timestamped file to
# act as a root prerequisite before kicking off the build DAG.
.PHONY: $(PROJECT_TMPDIR)/check-build-prerequisites
$(PROJECT_TMPDIR)/check-build-prerequisites:
	@# The BUILD_RESULT_FILE must be defined and exist.
	$(if $(BUILD_RESULT_FILE), \
	  $(if $(wildcard $(BUILD_RESULT_FILE)),, \
	    $(error BUILD_RESULT_FILE $(BUILD_RESULT_FILE) not found), \
	  $(error BUILD_RESULT_FILE not defined)))
	@# Check that the BUILDTIME_NIXPKGS_URL and EXPRESSION_BUILD_NIXPKGS_URL are defined.
	$(if $(BUILDTIME_NIXPKGS_URL),,$(error BUILDTIME_NIXPKGS_URL not defined))
	$(if $(EXPRESSION_BUILD_NIXPKGS_URL),,$(error EXPRESSION_BUILD_NIXPKGS_URL not defined))
	@$(_mkdir) -p $(@D)
	@$(_touch) $@

# The `nix build` command will attempt a rebuild in every instance,
# and we will presumably want `flox build` to do the same. However,
# we cannot just mark the various build targets as PHONY because they
# must be INTERMEDIATE to prevent `flox build foo` from rebuilding
# `bar` and `baz` as well (unless of course it was a prerequisite).
# So we instead derive the packages to be force-rebuilt from the special
# MAKECMDGOALS variable if defined, and otherwise rebuild them all.
# XXX Still used?
BUILDGOALS = $(if $(MAKECMDGOALS),$(MAKECMDGOALS),$(notdir $(MANIFEST_BUILDS)))
$(foreach _build,$(BUILDGOALS),\
  $(eval _pname = $(notdir $(_build)))\
  $(eval _pvarname = $(subst -,_,$(_pname)))\
  $(foreach _buildtype,local sandbox,\
    $(eval $(_pvarname)_$(_buildtype)_build: $(PROJECT_TMPDIR)/check-build-prerequisites)))

# Template for setting variables common to manifest and NEF builds.
define COMMON_BUILD_VARS_template =
  # N.B. _pname will have been set to the correct value before calling.

  # BUILD_OUTPUTS is used to track all known manifest and NEF build outputs,
  # and is used to rewrite ${output} references within the build script.
  $(eval BUILD_OUTPUTS += $(_pname))

  # We want to create build-specific variables, and variable names cannot
  # have "-" in them so we create a version of the build "pname" replacing
  # this with "_" for use in variable names.
  $(eval _pvarname = $(subst -,_,$(_pname)))
  # Define build-specific result symlink basename.
  $(eval $(_pvarname)_result = result-$(_pname))
  # Compute 32-character stable string for use in stable path generation
  # based on hash of pname, current working directory and FLOX_ENV.
  $(eval $(_pvarname)_hash = $(shell ( \
    ( echo $(_pname) $(PWD) ) | $(_sha256sum) | $(_head) -c32)))
  # And while we're at it, set a temporary basename in PROJECT_TMPDIR which
  # is a directory based on hash of pwd.
  $(eval $(_pvarname)_tmpBasename = $(PROJECT_TMPDIR)/$(_pname))

  # Create a target for cleaning up the temporary directory.
  .PHONY: clean/$(_pname)
  clean/$(_pname):
	-$(_V_) $(_find) $($(_pvarname)_tmpBasename) -type d -exec $(_chmod) +w {} \;
	-$(_V_) $(_rm) -rf $($(_pvarname)_tmpBasename)

  clean_targets += clean/$(_pname)

  # Create target-specific variables for constructing the JSON output to be
  # returned from the builds.
  $(eval $(_pvarname)_evalJSON = $($(_pvarname)_tmpBasename)/eval.json)
  $(eval $(_pvarname)_buildJSON = $($(_pvarname)_tmpBasename)/build.json)
  $(eval $(_pvarname)_buildMetaJSON = $($(_pvarname)_tmpBasename)/build-meta.json)

  # Create a temporary file for collecting log output from the build.
  $(eval $(_pvarname)_logfile = $($(_pvarname)_tmpBasename)/build.log)

  # For manifest builds only, we need to render a version of the build script
  # with package prerequisites replaced with their corresponding outpaths,
  # and we create that at a stable temporary path so that we only perform Nix
  # rebuilds when necessary.
  $(eval $(_pvarname)_buildScript = $($(_pvarname)_tmpBasename)/build.bash)

  # Perform post-build checks common to all build modes.
  .INTERMEDIATE: $(_pvarname)_CHECK_RESULT_LINKS
  $(_pvarname)_CHECK_RESULT_LINKS: $($(_pvarname)_buildJSON)
	@# Confirm that we see symlinks for all the expected build outputs.
	$$(eval _build_outputs = $$(shell $(_jq) -r ' \
	  .[0] | .outputs | to_entries | map ( \
	    ( if .key == "out" then "$($(_pvarname)_result)" else "$($(_pvarname)_result)-\(.key)" end ) as $$$$link | \
	    "\($$$$link),\(.value)" \
	  )[]' $$<))
	$$(foreach _build,$$(_build_outputs), \
	  $$(eval _link = $$(word 1,$$(subst $$(comma), ,$$(_build)))) \
	  $$(eval _store_path = $$(word 2,$$(subst $$(comma), ,$$(_build)))) \
	  $$(if $$(wildcard $$(_link)), \
	    $$(if $$(filter $$(_store_path),$$(shell $(_readlink) $$(_link))), \
	      $$(eval $(_pvarname)_resultLinks += "$(PWD)/$$(_link)":"$$(_store_path)"), \
	      $$(error $$(_link) of $$(_build) does not point to expected store path: $$(_store_path))), \
	    $$(error $$(_link) of $$(_build) does not exist)))
	@# Having confirmed the links, create the $(_pvarname)_resultLinks_json
	@# variable used to construct build-meta.json in the form of a json string
	@# like '{ "result-link1":"store-path1","result-link2":"store-path2",... }'.
	$$(eval $(_pvarname)_resultLinks_json = \
	  { $$(subst $$(space),$$(comma),$$($(_pvarname)_resultLinks)) })

endef

# Process common vars for NEF and manifest builds first in order to populate
# BUILD_OUTPUTS for the benefit of the MANIFEST_BUILD_DEPENDS_template later.
$(foreach _pname,$(NIX_EXPRESSION_BUILDS) $(notdir $(MANIFEST_BUILDS)), \
  $(eval $(call COMMON_BUILD_VARS_template)))

# Scan for "${package}" references within the build instructions and add
# target prerequisites for any inter-package prerequisites, letting make
# flag any circular dependencies encountered along the way.
define MANIFEST_BUILD_DEPENDS_template =
  $(eval _pvarname = $(subst -,_,$(notdir $(build))))

  # Iterate over each possible {build,package} pair looking for references to
  # ${package} in the build script, being careful to avoid looking for references
  # to the package in its own build. If found, declare dependency from the build
  # script to the package. Note that package can either imply the default output
  # (e.g. ${curl}) or explicitly specify an output (e.g. ${curl.bin}).
  $(foreach _output,$(filter-out $(notdir $(build)),$(BUILD_OUTPUTS)), \
    $(if $(shell $(_grep) '\$${$(_output)[.}]' $(build)), \
      $(eval _ovarname = $(subst -,_,$(_output))) \
      $(eval $(_pvarname)_deps_buildMetaJSON_files += $($(_ovarname)_buildMetaJSON)) \
      $($(_pvarname)_buildScript): $($(_ovarname)_buildMetaJSON)
    )
  )
endef

# Call MANIFEST_BUILD_DEPENDS_template to populate the build DAG.
$(foreach build,$(MANIFEST_BUILDS), \
  $(eval $(call MANIFEST_BUILD_DEPENDS_template)))

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

# Define a template target for cleaning up result symlinks
# NOTE: the commented lines to delete the associated store paths
#       are a temporary fix for flox#3017 where daemonized
#       `nix store delete` calls trigger a Nix bug and cause the `flox build`
#       command to fail.
define CLEAN_result_link_template =
  # Note that this template is evaluated at Makefile compilation time,
  # but is only called for the clean target, for which that's
  # a fine time to test for the existence of symlinks and storepaths,
  # so we can use GNU make functions to interrogate the filesystem
  # and create nicely formatted targets customized for each result link.

  # The builtin realpath function returns the empty string when the
  # result is a dangling symlink.
  # $(eval _storePath = $(realpath $(1)))

  .PHONY: clean_result_link/$(1)
  clean_result_link/$(1):
	-$(_V_) $(_rm) -f $(1)

  # .PHONY: clean_result_storepath/$(1)
  # clean_result_storepath/$(1): clean_result_link/$(1)
  #	$(_V_) $(_daemonize) $(_nix) store delete $(_storePath)

  clean/$(_pname): clean_result_link/$(1)
    # $(if $(_storePath),clean_result_storepath/$(1))
endef

# The manifest build strives to achieve reproducibility by first redacting
# the environment of variables actively managed by the Flox environment.
# We do this not only because we can rely on the Flox environment to set
# these variables to their correct values, but also because we want to
# avoid embedding untracked environment variables that were present at the
# time of the build.

# The following variables are used to track the set of variables to be redacted
# from the environment prior to kicking off a manifest build.
# TODO: move the clearing of variables to "activate --mode build"
FLOX_MANAGED_ENV_VARS = \
  FLOX_ACTIVATE_TRACE FLOX_ENV_DIRS FLOX_RUNTIME_DIR \
  INFOPATH CPATH PKG_CONFIG_PATH ACLOCAL_PATH XDG_DATA_DIRS \
  LD_AUDIT GLIBC_TUNABLES DYLD_FALLBACK_LIBRARY_PATH \
  PYTHONPATH PIP_CONFIG_FILE RUST_SRC_PATH JUPYTER_PATH LD_FLOXLIB_FILES_PATH
QUOTED_ENV_DISALLOW_ARGS = \
  $(foreach _arg,$(sort $(FLOX_MANAGED_ENV_VARS)),-u "$(_arg)")

# The following template renders targets for the in-situ build mode.
define BUILD_local_template =
  $(eval _virtualSandbox = $(filter-out null off,$(_sandbox)))

  # Set temp outpath of same strlen as eventual package storePath using the
  # 32-char hash previously derived from the package name, current working
  # directory and FLOX_ENV.
  $(eval $(_pvarname)_out = /tmp/store_$($(_pvarname)_hash)-$(_name))

  # Our aim in performing a manifest build is to replicate as closely as
  # possible the experience of running those same build script commands
  # from within an interactive `flox activate -m dev` shell (i.e. using
  # the "develop" environment), but we also need to provide a way to avoid
  # having to materialize compilers and tools not required at runtime, and
  # our approach for addressing this issue is the following:

  # 1. create "build-wrapper" environments for each build, customized to only
  #    include packages as found in the "runtime-packages" manifest attribute
  # 2. perform the build using the "develop" environment
  # 3. replace all references to the "develop" environment path with that of
  #    the "build-wrapper" environment

  # The easiest and most reliable way to replace those references (i.e. without
  # resorting to the use of tools like patchelf) is by way of a binary string
  # substitution, which carries the requirement that the paths involved in the
  # substitution must be of the same length. In other words, the path to the
  # "develop" environment _used for the build_ must have the same length as the
  # path to the "build-wrapper" environment.

  # We accomplish this by performing the build using a _copy_ of the "develop"
  # environment found in a path with the same strlen as the "build-wrapper"
  # environment, so that following a successful build we can replace references
  # to the former with the latter as we copy the output to its final path.

  $($(_pvarname)_out) $($(_pvarname)_logfile): $($(_pvarname)_buildScript)
	@# $(if $(FLOX_INTERPRETER),,$$(error FLOX_INTERPRETER not defined))
	@#
	@# Create a copy of the "develop" environment at a storepath with the
	@# same length as the "build-wrapper". N.B.: the "build-wrapper"
	@# environment bears the name "environment-build-$(_pname)" and
	@# strlen("environment") == strlen("developcopy").
	@#
	$$(eval $(_pvarname)_develop_copy_env = \
	  $$(shell $(_nix) store add --name "developcopy-build-$(_pname)" $(FLOX_ENV)))
	@#
	@# Throw error if the temporary build wrapper env path is empty.
	@#
	$$(if $$($(_pvarname)_develop_copy_env),, \
	  $$(error could not create copy of develop environment in store))
	@#
	@# Actually perform the build using the temporary build wrapper.
	@#
	@echo "Building $(_name) in local mode"
	-$(_VV_) $(_find) $($(_pvarname)_out) -type d -exec $(_chmod) +w {} \;
	-$(_VV_) $(_rm) -rf $($(_pvarname)_out)
	$(_V_) $(_env) $$(QUOTED_ENV_DISALLOW_ARGS) out=$($(_pvarname)_out) \
	  $(if $(_virtualSandbox),$(PRELOAD_VARS) FLOX_SRC_DIR=$(PWD) FLOX_VIRTUAL_SANDBOX=$(_sandbox)) \
	  $(FLOX_INTERPRETER)/activate --env $$($(_pvarname)_develop_copy_env) \
	    --mode build --env-project $(PWD) -- \
	    $(_t3) $($(_pvarname)_logfile) -- $(_bash) -e $$<
	@#
	@# Finally, rewrite references to temporary build wrapper in "out",
	@# making sure to return the substituted output to the same location
	@# in which it was built.
	@#
	$(_VV_) if [ -d "$($(_pvarname)_out)" ]; then \
	  $(_mkdir) -p $($(_pvarname)_out).new && \
	  set -o pipefail && \
	    ( cd $($(_pvarname)_out) && $(_find) . -print0 | \
	      $(_cpio) --null --create --format newc ) | \
	    $(_sed) --binary "s%$$($(_pvarname)_develop_copy_env)%$$($(_pvarname)_build_wrapper_env)%g" | \
	    ( cd $($(_pvarname)_out).new && $(_cpio) --extract --make-directories --preserve-modification-time \
	      --unconditional --no-absolute-filenames --quiet && $(_chmod) -R u+w . ) && \
	  $(_find) $($(_pvarname)_out) -type d -exec $(_chmod) +w {} \; && \
	  $(_rm) -rf $($(_pvarname)_out) && \
	  $(_mv) $($(_pvarname)_out).new $($(_pvarname)_out); \
	fi

  # Having built the package to $($(_pvarname)_out) outside of Nix, call
  # build-manifest.nix to turn it into a Nix package.
  $($(_pvarname)_buildJSON): $($(_pvarname)_out)
	$(_V_) $(_nix) build -L --file $(_libexec_dir)/build-manifest.nix \
	  --argstr pname "$(_pname)" \
	  --argstr version "$(_version)" \
	  --argstr flox-env "$(FLOX_ENV)" \
	  --argstr nixpkgs-url $(BUILDTIME_NIXPKGS_URL) \
	  --argstr build-wrapper-env "$$($(_pvarname)_build_wrapper_env)" \
	  --argstr install-prefix "$($(_pvarname)_out)" \
	  $$(if $$($(_pvarname)_buildDeps),--arg buildDeps '[$$($(_pvarname)_buildDeps)]') \
	  --out-link "result-$(_pname)" \
	  --json '^*' > $$@

  # Import the build log into the Nix store.
  $($(_pvarname)_result)-log: $($(_pvarname)_logfile)
	$(_V_) $(_nix) build -L `$(_nix) store add-file $$(shell $(_realpath) $$<)` --out-link $$@

  # Recall that the $(_pvarname)_CHECK_BUILD target as declared in the
  # MANIFEST_BUILD_template validates that the build is sane.

  # Assemble the final build metadata.
  $($(_pvarname)_buildMetaJSON): $($(_pvarname)_buildJSON) $($(_pvarname)_result)-log $(_pvarname)_CHECK_BUILD
	$(_V_) $(_jq) \
	  --arg name "$(_name)" \
	  --arg pname "$(_pname)" \
	  --arg version "$(_version)" \
	  --arg system "$(NIX_SYSTEM)" \
	  --slurpfile manifest "$(MANIFEST_LOCK)" \
	  --arg log "$(shell $(_readlink) $($(_pvarname)_result)-log)" \
	  --argjson resultLinks '$$($(_pvarname)_resultLinks_json)' \
	  '($$$$manifest[0].manifest.build."$(_pname)" | with_entries(select(.key == "description" or .key == "license"))) * { "outputsToInstall":["out"] } as $$$$meta | .[0] * { name:$$$$name, pname:$$$$pname, system: $$$$system, version:$$$$version, log:$$$$log, resultLinks: $$$$resultLinks, meta: $$$$meta }' $$< > $$@
	@echo "Completed build of $(_name) in local mode" && echo ""

endef

# The following template renders targets for the sandbox build mode.
define BUILD_nix_sandbox_template =
  # If set, the DISABLE_BUILDCACHE variable will cause the build to omit the
  # build cache.  This is used for (at least) publish.
  $(eval _do_buildCache = $(if $(DISABLE_BUILDCACHE),,true))

  # 'git ls-files' will list all _tracked_ files **including deleted files**.
  # Consequently, when we try to create a tarball with all files listed by
  # 'git ls-files' we may attempt packing files that are actually deleted.
  # To avoid this, we filter out deleted files.
  # Because 'git ls-files' does not have a flag to filter deleted files,
  # but allows to _only_ show deleted files, use 'comm' to do the filtering for
  # us.
  $(eval $(_pvarname)_src_list = $($(_pvarname)_tmpBasename)/src-list)
  $($(_pvarname)_src_list): $(PROJECT_TMPDIR)/check-build-prerequisites
	$(_comm) -23 <($(_git) ls-files -c | $(_sort)) <($(_git) ls-files -d | $(_sort)) > $$@

  # The sourceTarball value needs to be stable when nothing changes across
  # builds, so we create a tarball at a stable temporary path and pass that
  # to the derivation instead.
  $(eval $(_pvarname)_src_tar = $($(_pvarname)_tmpBasename)/src.tar)
  $($(_pvarname)_src_tar): $($(_pvarname)_src_list)
	$(_V_) $(_tar) -cf $$@ --no-recursion --files-from $$<

  # The buildCache value needs to be similarly stable when nothing changes across
  $(eval $(_pvarname)_buildCache = $($(_pvarname)_tmpBasename)/buildCache.tar)
  $($(_pvarname)_buildCache): $(PROJECT_TMPDIR)/check-build-prerequisites
	-$(_V_) $(_rm) -f $$@
	@# If a previous buildCache exists, then copy, don't link to the
	@# previous buildCache because we want nix to import it as a
	@# content-addressed input rather than an ever-changing series
	@# of storePaths. And if it does not exist, then create a new
	@# tarball containing only a single file indicating the time that
	@# the buildCache was created to differentiate it from other
	@# prior otherwise-empty buildCaches.
	$(_VV_) if [ -f "$($(_pvarname)_result)-buildCache" ]; then \
	  $(_cp) $($(_pvarname)_result)-buildCache $$@; \
	else \
	  tmpdir=$$$$($(_mktemp) -d); \
	  echo "Build cache initialized on $$$$(date)" > $$$$tmpdir/.buildCache.init; \
	  $(_tar) -cf $$@ -C $$$$tmpdir .buildCache.init; \
	  $(_find) $$$$tmpdir -type d -exec $(_chmod) +w {} \; && \
	  $(_rm) -rf $$$$tmpdir; \
	fi

  # Create a target for cleaning up the buildCache result symlink and store path.
  $(eval $(call CLEAN_result_link_template,$($(_pvarname)_result)-buildCache))

  # Perform the build, creating the JSON output as a result.
  $($(_pvarname)_buildJSON): $($(_pvarname)_buildScript) $($(_pvarname)_src_tar) \
    $(if $(_do_buildCache),$($(_pvarname)_buildCache))
	@echo "Building $(_name) in Nix sandbox (pure) mode"
	@# If a previous buildCache exists then move it out of the way
	@# so that we can detect later if it has been updated.
	$(_VV_) if [ -n "$(_do_buildCache)" ] && [ -f "$($(_pvarname)_result)-buildCache" ]; then \
	  $(_rm) -f "$($(_pvarname)_result)-buildCache.prevOutPath"; \
	  $(_readlink) "$($(_pvarname)_result)-buildCache" > "$($(_pvarname)_result)-buildCache.prevOutPath"; \
	fi
	$(_V_) $(_nix) build -L --file $(_libexec_dir)/build-manifest.nix \
	  --argstr pname "$(_pname)" \
	  --argstr version "$(_version)" \
	  --argstr srcTarball "$($(_pvarname)_src_tar)" \
	  --argstr flox-env "$(FLOX_ENV)" \
	  --argstr nixpkgs-url $(BUILDTIME_NIXPKGS_URL) \
	  --argstr build-wrapper-env "$$($(_pvarname)_build_wrapper_env)" \
	  $$(if $$($(_pvarname)_buildDeps),--arg buildDeps '[$$($(_pvarname)_buildDeps)]') \
	  --argstr buildScript "$($(_pvarname)_buildScript)" \
	  $(if $(_do_buildCache),--argstr buildCache "$($(_pvarname)_buildCache)") \
	  --out-link "result-$(_pname)" \
	  --json '^*' > $$@

  # Recall that the $(_pvarname)_CHECK_BUILD target as declared in the
  # MANIFEST_BUILD_template validates that the build is sane.

  $($(_pvarname)_buildMetaJSON): $($(_pvarname)_buildJSON) $(_pvarname)_CHECK_BUILD
	$(_V_) $(_jq) \
	  --arg name "$(_name)" \
	  --arg pname "$(_pname)" \
	  --arg version "$(_version)" \
	  --arg system "$(NIX_SYSTEM)" \
	  --slurpfile manifest "$(MANIFEST_LOCK)" \
	  --argjson resultLinks '$$($(_pvarname)_resultLinks_json)' \
	  '($$$$manifest[0].manifest.build."$(_pname)" | with_entries(select(.key == "description" or .key == "license"))) * { "outputsToInstall":["out"] } as $$$$meta | .[0] * { name:$$$$name, pname:$$$$pname, system: $$$$system, version:$$$$version, log:.[0].outputs.log, resultLinks:$$$$resultLinks, meta: $$$$meta }' $$< > $$@
	@echo "Completed build of $(_name) in Nix sandbox mode" && echo ""
	@# Check to see if a new buildCache has been created, and if so then go
	@# ahead and run 'nix store delete' on the previous cache, keeping in
	@# mind that the symlink will remain unchanged in the event of an
	@# unsuccessful build.
	$(_VV_) if [ -n "$(_do_buildCache)" ]; then \
	  if [ -f "$($(_pvarname)_result)-buildCache" ] && [ -f "$($(_pvarname)_result)-buildCache.prevOutPath" ]; then \
	    if [ $$$$($(_readlink) "$($(_pvarname)_result)-buildCache") != $$$$($(_cat) "$($(_pvarname)_result)-buildCache.prevOutPath") ]; then \
	      $(_daemonize) $(_nix) store delete \
	        $$$$($(_cat) "$($(_pvarname)_result)-buildCache.prevOutPath"); \
	    fi; \
	  fi; \
	  $(_rm) -f "$($(_pvarname)_result)-buildCache.prevOutPath"; \
	fi

endef

# The following jq script reads a list of build-meta.json files for the
# purpose of rendering a set of "-e 's%pkgname.output%/nix/store/...%g'"
# arguments suitable for use with sed.
define JSON_OUTPUTS_TO_SED_ARGS_jq =
  .pname as $$pname |
  .outputs | to_entries | map (
    " -e " + ("s%\\$$$${\($$pname).\(.key)}%\(.value)%g" | @sh) +
    (
      if .key == "out" then (
        " -e " + ("s%\\$$$${\($$pname)}%\(.value)%g" | @sh)
      ) else "" end
    )
  )[]
endef

# The following template selects between the local and nix_sandbox
# modes for rendering manifest build targets.
define MANIFEST_BUILD_template =
  # build mode passed as $(1)
  $(eval _build_mode = $(1))
  # We want to create build-specific variables, and variable names cannot
  # have "-" in them so we create a version of the build "pname" replacing
  # this with "_" for use in variable names.
  $(eval _pvarname = $(subst -,_,$(_pname)))
  # Calculate name.
  $(eval _name = $(_pname)-$(_version))

  # By the time this rule will be evaluated all of its package dependencies
  # will have been added to the set of rule prerequisites in $^, using their
  # "safe" name (with "-" characters replaced with "_"), and these targets
  # will have successfully built the corresponding result-$(_pname) symlinks.
  # Iterate through this list, replacing all instances of "${package}" with the
  # corresponding storePath as identified by the result-* symlink.
  .PRECIOUS: $($(_pvarname)_buildScript)
  $($(_pvarname)_buildScript): $(build) $(PROJECT_TMPDIR)/check-build-prerequisites
	@# Identify _at runtime_ the build wrapper environment with which
	@# to wrap the contents of bin, sbin.
	$$(eval $(_pvarname)_build_wrapper_env = $$(strip \
	  $$(if $$(FLOX_ENV_OUTPUTS), \
	    $$(shell $(_jq) -n -r \
	      --argjson results '$$(FLOX_ENV_OUTPUTS)' \
	      '$$$$results."build-$(_pname)"') \
	    $$(if $$(filter 0,$$(.SHELLSTATUS)),,$$(error could not identify build wrapper env for $(_pname))), \
	    $$$$(error FLOX_ENV_OUTPUTS not defined))))
	@# Variable for providing buildDependencies derived in the DEPENDS step
	@# to the Nix expression.
	$$(eval $(_pvarname)_buildMetaJSON_files = $$(wildcard $$($(_pvarname)_deps_buildMetaJSON_files)))
	$$(eval $(_pvarname)_buildDeps = $$(if $$($(_pvarname)_buildMetaJSON_files), \
	  $$(sort $$(shell $(_jq) -s -r '(map(.outputs[])[])' $$($(_pvarname)_buildMetaJSON_files)))))
	$(_V_) $(_mkdir) -p $$(@D)
	@echo "Rendering $(_pname) build script to $$@"
	@# Always echo lines in the build script as they are invoked.
	$(_V_) echo "set -x" > $$@.new
	$(_V_) $(_cat) $$< >> $$@.new
	$$(eval _sed_args =)
	$$(foreach _buildMetaJSON,$$(filter-out $(build) $(PROJECT_TMPDIR)/check-build-prerequisites,$$^), \
	  $$(eval _sed_args += $$(shell $(_jq) -r '$$(JSON_OUTPUTS_TO_SED_ARGS_jq)' $$(_buildMetaJSON))))
	$$(if $$(_sed_args),$(_V_) $(_sed) -i $$(strip $$(_sed_args)) $$@.new)
	$(_V_) $(_mv) -f $$@.new $$@

  # Insert mode-specific template.
  $(call BUILD_$(_build_mode)_template)

  # Recall that the $(_pvarname)_CHECK_RESULT_LINKS target as declared in the
  # COMMON_BUILD_VARS_template checks that the result symlinks point to the
  # expected store paths.

  # Perform post-build checks common to all build modes.
  .INTERMEDIATE: $(_pvarname)_CHECK_BUILD
  $(_pvarname)_CHECK_BUILD: $(_pvarname)_CHECK_RESULT_LINKS
	@# Take this opportunity to fail the build if we spot fatal errors in the
	@# build output. Recall that we force the Nix build to "succeed" in all
	@# cases so that we can persist the buildCache, so when errors do happen
	@# this is communicated by way of a $out that is 1) a file and 2) contains
	@# the string "flox build failed (caching build dir)".
	$(_VV_) if [ -f $($(_pvarname)_result) ] && $(_grep) -q "flox build failed (caching build dir)" $($(_pvarname)_result); then \
	  echo "ERROR: flox build failed (see $($(_pvarname)_result)-log)" 1>&2; \
	  $(_rm) -f $($(_pvarname)_result); \
	  exit 1; \
	fi
	@# Then use the 'validate-build' script to:
	@# 1. verify that the build only contains references to packages from
	@#    a) its own "build" environment and b) any extra NEF or manifest
	@#    build storepaths referenced in its build script (as tracked in
	@#    "_buildDeps" and passed with "-x <dep>")
	@# 2. scan output for path references not found within the "build"
	@#    environment that can occur when replacing the "developcopy"
	@#    environment at the conclusion of a manifest build
	@# Emits diagnostics to stderr and returns nonzero result upon failure.
	$(_V_) $(_libexec_dir)/validate-build \
	  --build-env $$($(_pvarname)_build_wrapper_env) \
	  --develop-env $(FLOX_ENV) \
	  --system $(NIX_SYSTEM) \
	  --pname $(_pname) \
	  $$(if $$($(_pvarname)_buildDeps),$$(foreach _dep,$$($(_pvarname)_buildDeps),-x $$(_dep))) \
	  $$(shell $(_readlink) $($(_pvarname)_result))

  # Create targets for cleaning up the result and log symlinks.
  $(eval $(call CLEAN_result_link_template,$($(_pvarname)_result)))
  $(eval $(call CLEAN_result_link_template,$($(_pvarname)_result)-log))

  # Create a helper target for referring to the package by its name rather
  # than the [real] result symlink we're looking to create. (UNUSED)
  .PHONY: build/$(_pname)
  build/$(_pname): $(_pvarname)_CHECK_BUILD
endef

# Glean various values from locked manifest as we call the template.

# The following jq script parses the JSON representation of the version
# string and converts it to a command that can be immediately evaluated
# by the shell prior to being represented as a Makefile variable. This is
# particularly relevant in the case that people attempt to set a version
# using a command like "echo $(whoami)", and you want the shell to invoke
# the command before make gets a chance of attempting to evaluate it as an
# [undefined] variable.
#
# The JSON version variable can take the form of a string like "1.2.3",
# or it can be a single-element object like {"file": "VERSION"} or
# {"command": "echo $(whoami)"}. This script converts these to strings
# "echo 1.2.3", "cat VERSION" and "echo $(whoami)", respectively so
# that the shell can then evaluate any variable expansions immediately
# as it invokes the commands.
#
define JSON_VERSION_TO_COMMAND_jq =
  (.manifest.build."\($$pname)".version // "0.0.0") | (
    if type == "object" then (
      to_entries[] | \
      if .key == "file" then "$(_cat) \(.value)" else (
        if .key == "command" then (
          "$(FLOX_ENV)/activate -c \(.value | @sh)"
        ) else (
          "unknown version type: \(.key)" | halt_error(1)
        ) end
      ) end
    ) else "echo \(. | @sh)" end
  )
endef
$(foreach build,$(MANIFEST_BUILDS), \
  $(eval _pname = $(notdir $(build))) \
  $(eval _sandbox = $(shell \
    $(_jq) -r '.manifest.build."$(_pname)".sandbox' $(MANIFEST_LOCK))) \
  $(eval _version = $(shell $(shell \
    $(_jq) -r --arg pname "$(_pname)" '$(strip $(JSON_VERSION_TO_COMMAND_jq))' $(MANIFEST_LOCK)))) \
  $(if $(filter null off,$(_sandbox)), \
    $(eval $(call MANIFEST_BUILD_template,local)), \
    $(eval $(call MANIFEST_BUILD_template,nix_sandbox))))


# The following template renders targets for the Nix expression build mode.
define NIX_EXPRESSION_BUILD_template =
  # We want to create build-specific variables, and variable names cannot
  # have "-" in them so we create a version of the build "pname" replacing
  # this with "_" for use in variable names.
  $(eval _pvarname = $(subst -,_,$(_pname)))

  # Start by evaluating the build

  $($(_pvarname)_evalJSON): $(PROJECT_TMPDIR)/check-build-prerequisites
	$(_V_) $(_mkdir) -p $$(@D)
	$(_V_) $(_nix) eval -L --file $(_nef) \
	  --argstr nixpkgs-url "$(EXPRESSION_BUILD_NIXPKGS_URL)" \
	  --argstr system $(NIX_SYSTEM) \
	  $(NIX_EXPRESSION_DIR_ARGS) \
	  --json \
	  --apply 'pkg: { \
	    drvPath = pkg.drvPath; \
	    version = pkg.version or "unknown"; \
	    name = pkg.name; \
	    pname = pkg.pname or pkg.name; \
	    meta = pkg.meta or null; \
	  }' \
	  pkgs.$(_pname) > $$@

  # Following a successful eval, carry on with building the drvPath directly.
  $($(_pvarname)_buildJSON): $($(_pvarname)_evalJSON)
	@# Now that we have the metadata, set the _name.
	$$(eval _name = $$(shell $(_jq) -r '.name' $$<))
	$$(eval _drvPath = $$(shell $(_jq) -r '.drvPath' $$<))
	@# Verify that the drvPath still exists in the store because there
	@# is a very small but non-zero chance that the package has been
	@# garbage collected since the eval was performed.
	$$(if $$(wildcard $$(_drvPath)),,\
	  $$(error $$(_drvPath) has been garbage collected - please try again))
	@echo "Building $$(_name) in Nix expression mode"
	$(_V_) $(_nix) build --json -L --out-link $($(_pvarname)_result) \
	  $$(_drvPath)'^*' > $$@

  # Recall that the $(_pvarname)_CHECK_RESULT_LINKS target as declared in the
  # COMMON_BUILD_VARS_template checks that the result symlinks point to the
  # expected store paths.

  # Perform post-build checks common to all build modes.
  .INTERMEDIATE: $(_pvarname)_CHECK_BUILD
  $(_pvarname)_CHECK_BUILD: $(_pvarname)_CHECK_RESULT_LINKS
	@# do something here?

  # Harvest the logfile from the build.
  $($(_pvarname)_logfile): $($(_pvarname)_buildJSON) $(_pvarname)_CHECK_BUILD
	$$(eval _drvPath = $$(shell $(_jq) -r '.[0].drvPath' $$<))
	$(_V_) ( $(_nix) log $$(_drvPath) 2>/dev/null || echo "No logs available" ) > $($(_pvarname)_logfile)

  # Add the log to the store and create a GCRoot for it.
  $($(_pvarname)_result)-log: $($(_pvarname)_logfile)
	$(_V_) $(_nix) build -L --out-link $($(_pvarname)_result)-log \
	  $$(shell $(_nix) store add-file $($(_pvarname)_logfile))

  # Following a successful build, merge in the eval json.
  $($(_pvarname)_buildMetaJSON): $($(_pvarname)_evalJSON) $($(_pvarname)_buildJSON) $($(_pvarname)_result)-log
	$(_V_) $(_jq) -n \
	  --arg logfile $$(shell $(_readlink) $($(_pvarname)_result)-log) \
	  --arg system $(NIX_SYSTEM) \
	  --argjson resultLinks '$$($(_pvarname)_resultLinks_json)' \
	  --slurpfile eval $($(_pvarname)_evalJSON) \
	  --slurpfile build $($(_pvarname)_buildJSON) \
	  '$$$$build[0][0] * $$$$eval[0] * { system: $$$$system, log: $$$$logfile, resultLinks: $$$$resultLinks }' > $$@
	@echo -e "Completed build of $$(_name) in Nix expression mode\n"

  # Create targets for cleaning up the result and log symlinks.
  $(eval $(call CLEAN_result_link_template,$($(_pvarname)_result)))
  $(eval $(call CLEAN_result_link_template,$($(_pvarname)_result)-log))

  # Create targets for cleaning up all other NEF output result symlinks.
  $(if $(wildcard $($(_pvarname)_buildJSON)), \
    $(eval _outputs = $(shell $(_jq) -r '.[0].outputs | keys[]' $($(_pvarname)_buildJSON))) \
    $(foreach _output,$(_outputs), \
      $(eval $(call CLEAN_result_link_template,$($(_pvarname)_result)-$(_output)))))

  # Create a helper target for referring to the package by its name rather
  # than the [real] result symlink we're looking to create. (UNUSED)
  .PHONY: build/$(_pname)
  build/$(_pname): $($(_pvarname)_buildMetaJSON)
endef

$(foreach _pname,$(NIX_EXPRESSION_BUILDS), \
  $(eval $(call NIX_EXPRESSION_BUILD_template)))

# Combine JSON build data for each build and write to BUILD_RESULT_FILE.
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
