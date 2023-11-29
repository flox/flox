# ============================================================================ #
#
# Provides `test' and `check' targets.
#
# NOTE: `TEST_template' is defined by `./lib.mk' and processed
#       by `./gen-targets.mk'.
#
#
# ---------------------------------------------------------------------------- #

ifndef _MK_CHECK

_MK_CHECK = 1

# ---------------------------------------------------------------------------- #

.PHONY: tests check


# ---------------------------------------------------------------------------- #

TEST ?= test


# ---------------------------------------------------------------------------- #

check_TARGETS ::=


# ---------------------------------------------------------------------------- #

check: $(check_TARGETS) FORCE
	for c in $(check_TARGETS); do                \
	  if ! $(TEST) -e "$$c"; then continue; fi;  \
	  if "$$c"; then                             \
	    echo "PASS: $$c" >&2;                    \
	  else                                       \
	    echo "FAIL: $$c" >&2;                    \
	  fi;                                        \
	done

# ---------------------------------------------------------------------------- #


TEST_MANIFESTS := $(wildcard $(ROOT_DIR)/tests/fixtures/lockfiles/*/manifest.toml)
TEST_MANIFEST_LOCKS := $(TEST_MANIFESTS:%.toml=%.lock)

$(TEST_MANIFEST_LOCKS):
	@echo "Locking manifest '$(@:%.lock=%){.toml -> .lock}'"
	pkgdb manifest lock "$(@:%.lock=%.toml)" --ga-registry | jq > "$(@)"

test: $(BIN_flox-env-builder) $(wildcard $(ROOT_DIR)/tests/**) $(TEST_MANIFEST_LOCKS) FORCE
	PKGDB="$(ROOT_DIR)/bin/flox-env-builder"                        \
	  bats --print-output-on-failure --verbose-run --timing  \
	       "$(ROOT_DIR)/tests";


endif  # ifndef _MK_CHECK


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
