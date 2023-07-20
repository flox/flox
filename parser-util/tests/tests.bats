#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test the `parser-util' executable.
#
# ---------------------------------------------------------------------------- #

bats_load_library bats-support;
bats_load_library bats-assert;
bats_require_minimum_version '1.5.0';


# ---------------------------------------------------------------------------- #

# Suppress the creation of file/suite homedirs.
setup_file() {
  mkdir -p "$BATS_FILE_TMPDIR";
  pushd "$BATS_FILE_TMPDIR" >/dev/null||exit;

  : "${PARSER_UTIL:=parser-util}";
  : "${JQ:=jq}";
  : "${SED:=sed}";

  $SED "s,\/tmp\/parser-util-test-root,$PWD"       \
       "$BATS_TEST_DIRNAME/ref-str-to-attrs.json"  \
       > ./ref-str-to-attrs.json;

  export PARSER_UTIL JQ SED;
}

teardown_file() { popd >/dev/null||cd /; }


# ---------------------------------------------------------------------------- #

@test "parseAndResolveRef ( strings )" {
  local _count _i _str _expected _rsl;
  _count="$( $JQ -r length ./ref-str-to-attrs.json; )";
  _i=0;
  while [[ "$_i" -lt "$_count" ]]; do
    _str='';
    _expected='';
    _rsl='';
    _str="$( $JQ -rcS ".[$_i].input" ./ref-str-to-attrs.json; )";
    _expected="$( $JQ -rcS ".[$_i]" ./ref-str-to-attrs.json; )";
    _rsl="$( $PARSER_UTIL -r "$_str"|$JQ -rcS; )";
    assert [[ "$_expected" = "$_rsl" ]];
  done
}


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
