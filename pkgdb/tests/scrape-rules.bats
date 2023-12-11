#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# `pkgdb scrape --rules' CLI tests.
#
# These tests are meant to test the `pkgdb scrape --rules' CLI command.
#
# ---------------------------------------------------------------------------- #

load setup_suite.bash

# bats file_tags=cli,scrape,scrape-rules,flake

# Set up test environment
setup() {
	export DBPATH="$BATS_FILE_TMPDIR/test.sqlite"
	mkdir -p "$BATS_FILE_TMPDIR"
	# We don't parallelize these to avoid DB sync headaches and to recycle the
	# cache between tests.
	# Nonetheless this file makes an effort to avoid depending on past state in
	# such a way that would make it difficult to eventually parallelize in
	# the future.
	export BATS_NO_PARALLELIZE_WITHIN_FILE=true

	# Define rules.json content
	rules_json='{
    "allowRecursive": [
      ["legacyPackages", null, "darwin"]
    ],
    "allowPackage": [
      ["legacyPackages", null, "python310Packages", "pip"],
      ["legacyPackages", null, "python310Packages", "django"]
    ],
    "disallowRecursive": [
      ["legacyPackages", null, "python310Packages"],
      ["legacyPackages", null, "emacsPackages"],
      ["legacyPackages", null, "vimPlugins"]
    ],
    "disallowPackage": [
      ["legacyPackages", null, "sqlite"]
    ]
  }'
}

# Test pkgdb scrape with allow_json
@test "pkgdb scrape with allow_json" {
	#disable for now until feature is implemented
	skip "FIXME: disable for now until feature is implemented"
	run pkgdb scrape <(echo "$rules_json") <NIXPKGS >--rules
	assert_success
	run sqlite3 "$DBPATH" "SELECT attrName FROM Packages      \
    WHERE name = 'legacyPackages.x86_64-linux.darwin' LIMIT 1"
	assert_output 'darwin'
}

@test "pkgdb scrape with disallow emacs" {
	#disable for now until feature is implemented
	skip "FIXME: disable for now until feature is implemented"
	run pkgdb scrape <(echo "$rules_json") <NIXPKGS >--rules
	run sqlite3 "$DBPATH" "SELECT attrName FROM Packages      \
    WHERE name = 'legacyPackages.x86_64-linux.emacs' LIMIT 1"
	assert_output ''
}

@test "pkgdb scrape with both allow_json and disallow_json" {
	#disable for now until feature is implemented
	skip "FIXME: disable for now until feature is implemented"

	run pkgdb scrape <(echo "$rules_json") <NIXPKGS >--rules

	#allowRecursive test
	run sqlite3 "$DBPATH" "SELECT attrName FROM Packages      \
    WHERE name = 'legacyPackages.x86_64-linux.darwin' LIMIT 1"
	assert_output 'darwin'
	#disallowRecursive test
	run sqlite3 "$DBPATH" "SELECT attrName FROM Packages      \
    WHERE name = 'legacyPackages.x86_64-linux.emacs' LIMIT 1"
	assert_output ''
	#requests should not be found due to disallowRecursive
	run sqlite3 "$DBPATH" "SELECT attrName FROM Packages      \
    WHERE name = 'legacyPackages.x86_64-linux.python3Packages.requests' LIMIT 1"
	assert_output ''
	#pip and django should be found allowPackage takes priority over disallowRecursive.
	# that lets us "disable all python310Packages except for ____ (pip and django)"
	run sqlite3 "$DBPATH" "SELECT attrName FROM Packages      \
    WHERE name = 'legacyPackages.x86_64-linux.python3Packages.pip' LIMIT 1"
	assert_output 'pip'
	run sqlite3 "$DBPATH" "SELECT attrName FROM Packages      \
    WHERE name = 'legacyPackages.x86_64-linux.python3Packages.django' LIMIT 1"
	assert_output 'django'
	#disallowPackage test
	run sqlite3 "$DBPATH" "SELECT attrName FROM Packages      \
    WHERE name = 'legacyPackages.x86_64-linux.sqlite' LIMIT 1"
	assert_output ''
}
