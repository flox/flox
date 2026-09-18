#! /usr/bin/env bats

load test_support.bash

@test "diagnostics: RUST_LOG does not enable CLI diagnostics" {
  run --separate-stderr env -u FLOX_LOG RUST_LOG=trace "$FLOX_BIN" --not-a-flox-option
  assert_failure
  assert_output ""
  assert_regex "$stderr" 'ERROR:'
  [[ "$stderr" != *"FLOX_VERSION="* ]]
}

@test "diagnostics: FLOX_LOG off preserves user errors even with verbosity" {
  run --separate-stderr env FLOX_LOG=off "$FLOX_BIN" -vvvv --not-a-flox-option
  assert_failure
  assert_output ""
  assert_regex "$stderr" 'ERROR:'
  [[ "$stderr" != *"FLOX_VERSION="* ]]
}

@test "diagnostics: quiet mode preserves user errors" {
  run --separate-stderr env -u FLOX_LOG "$FLOX_BIN" -q --not-a-flox-option
  assert_failure
  assert_output ""
  assert_regex "$stderr" 'ERROR:'
}

@test "diagnostics: verbose diagnostics only reach stderr" {
  run --separate-stderr env -u FLOX_LOG "$FLOX_BIN" -vv --not-a-flox-option
  assert_failure
  assert_output ""
  assert_regex "$stderr" 'FLOX_VERSION='
  assert_regex "$stderr" 'ERROR:'
}

@test "diagnostics: FLOX_LOG overrides quiet mode" {
  run --separate-stderr env FLOX_LOG=debug "$FLOX_BIN" -q --not-a-flox-option
  assert_failure
  assert_output ""
  assert_regex "$stderr" 'FLOX_VERSION='
  assert_regex "$stderr" 'ERROR:'
}

@test "diagnostics: quiet suppresses routine notices independently of FLOX_LOG" {
  run --separate-stderr env FLOX_LOG=off "$FLOX_BIN"
  assert_success
  assert_regex "$stderr" 'Usage: flox OPTIONS'

  run --separate-stderr env FLOX_LOG=off "$FLOX_BIN" -q
  assert_success
  assert_output ""
  [[ -z "$stderr" ]]

  run --separate-stderr env FLOX_LOG=info "$FLOX_BIN" -q
  assert_success
  assert_output ""
  assert_regex "$stderr" 'Command started'
  [[ "$stderr" != *"Usage: flox OPTIONS"* ]]
}

@test "diagnostics: quiet preserves help and errors on their streams" {
  run --separate-stderr env FLOX_LOG=off "$FLOX_BIN" -q --help
  assert_success
  assert_output --partial 'Usage:'
  [[ -z "$stderr" ]]

  run --separate-stderr env FLOX_LOG=off "$FLOX_BIN" -q --not-a-flox-option
  assert_failure
  assert_output ""
  assert_regex "$stderr" 'ERROR:'
  [[ "$stderr" != *$'\e['* ]]
}

@test "diagnostics: quiet preserves completion output on stdout" {
  run --separate-stderr env FLOX_LOG=off "$FLOX_BIN" -q --bpaf-complete-rev=8 act
  assert_success
  assert_output --partial 'activate'
  [[ -z "$stderr" ]]
}
