#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test flox edit
# TODO move other edit tests from integration.bats
#
# ---------------------------------------------------------------------------- #

load test_support.bash;

# bats file_tags=edit

# ---------------------------------------------------------------------------- #

EDIT_ENVIRONMENT=_edit_testing_

# ---------------------------------------------------------------------------- #

@test "flox edit -f" {
  run $FLOX_CLI destroy --force -e "$EDIT_ENVIRONMENT"
  run $FLOX_CLI create -e "$EDIT_ENVIRONMENT"
  assert_success

  # test reading from a file
  run $FLOX_CLI edit -e "$EDIT_ENVIRONMENT" -f "$TESTS_DIR/test-flox.nix"
  assert_success
  assert_output --partial "Environment '$EDIT_ENVIRONMENT' modified."

  EDITOR=cat run $FLOX_CLI edit -e "$EDIT_ENVIRONMENT"
  assert_success
  assert_output --partial 'environmentVariables.test = "file"'

  # test reading from stdin
  contentsStdin=$(cat <<'EOF'
{
  environmentVariables.test = "stdin";
}
EOF
)
  run $FLOX_CLI edit -e "$EDIT_ENVIRONMENT" --file - <<<"$contentsStdin"
  assert_success
  assert_output --partial "Environment '$EDIT_ENVIRONMENT' modified."
  EDITOR=cat run $FLOX_CLI edit -e "$EDIT_ENVIRONMENT"
  assert_success
  assert_output --partial "$contentsStdin"

  run $FLOX_CLI destroy --force -e "$EDIT_ENVIRONMENT"
  assert_success
}

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
