#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# `pkgdb parse descriptor' tests.
#
# ---------------------------------------------------------------------------- #

load setup_suite.bash;

# bats file_tags=parse:descriptor

setup_file() {
  export TDATA="$TESTS_DIR/data/search";

  # Path to `search-params' utility.
  export SEARCH_PARAMS="$TESTS_DIR/search-params";

  export PKGDB_CACHEDIR="$BATS_FILE_TMPDIR/pkgdbs";
  echo "PKGDB_CACHEDIR: $PKGDB_CACHEDIR" >&3;
}

# ---------------------------------------------------------------------------- #

@test "parse descriptor 'hello'" {
  run "$PKGDB" parse descriptor --to manifest "hello";
  assert_success;
  assert_equal $(echo "$output" | jq -r '.name') "hello";
  unset output;
  run "$PKGDB" parse descriptor --to query "hello";
  assert_success;
  assert_equal $(echo "$output" | jq -r '.pnameOrAttrName') "hello";
}

@test "parse descriptor 'hello@1.2.3'" {
  query="hello@1.2.3";
  run "$PKGDB" parse descriptor --to manifest "$query";
  assert_success;
  assert_equal $(echo "$output" | jq -r '.name') "hello";
  assert_equal $(echo "$output" | jq -r '.version') "1.2.3";
  unset output;
  run "$PKGDB" parse descriptor --to query "$query";
  assert_success;
  assert_equal $(echo "$output" | jq -r '.pnameOrAttrName') "hello";
  assert_equal $(echo "$output" | jq -r '.version') "1.2.3";
}

@test "parse descriptor 'hello@1.2'" {
  query="hello@1.2";
  run "$PKGDB" parse descriptor --to manifest "$query";
  assert_success;
  assert_equal $(echo "$output" | jq -r '.name') "hello";
  assert_equal $(echo "$output" | jq -r '.semver') "1.2";
  unset output;
  run "$PKGDB" parse descriptor --to query "$query";
  assert_success;
  assert_equal $(echo "$output" | jq -r '.pnameOrAttrName') "hello";
  assert_equal $(echo "$output" | jq -r '.semver') "1.2";
}

@test "parse descriptor 'packageset.hello@1.2'" {
  query="packageset.hello@1.2";
  run "$PKGDB" parse descriptor --to manifest "$query";
  assert_success;
  assert_equal $(echo "$output" | jq -r -c '.path') '["packageset","hello"]';
  assert_equal $(echo "$output" | jq -r '.semver') "1.2";
  unset output;
  run "$PKGDB" parse descriptor --to query "$query";
  assert_success;
  assert_equal $(echo "$output" | jq -r -c '.relPath') '["packageset","hello"]';
  assert_equal $(echo "$output" | jq -r '.semver') "1.2";
}

@test "parse descriptor 'legacyPackages.aarch64-darwin.hello@1.2'" {
  query="legacyPackages.aarch64-darwin.hello@1.2";
  run "$PKGDB" parse descriptor --to manifest "$query";
  assert_success;
  assert_equal $(echo "$output" | jq -r -c '.path') '["hello"]';
  assert_equal $(echo "$output" | jq -r '.subtree') "legacyPackages";
  assert_equal $(echo "$output" | jq -r '.semver') "1.2";
  unset output;
  run "$PKGDB" parse descriptor --to query "$query";
  assert_success;
  assert_equal $(echo "$output" | jq -r -c '.relPath') '["hello"]';
  assert_equal $(echo "$output" | jq -r -c '.subtrees') '["legacyPackages"]';
  assert_equal $(echo "$output" | jq -r '.semver') "1.2";
}

@test "parse descriptor 'legacyPackages.*.hello@1.2'" {
  query="legacyPackages.*.hello@1.2";
  run "$PKGDB" parse descriptor --to manifest "$query";
  assert_success;
  assert_equal $(echo "$output" | jq -r -c '.path') '["hello"]';
  assert_equal $(echo "$output" | jq -r '.subtree') "legacyPackages";
  assert_equal $(echo "$output" | jq -r '.semver') "1.2";
  unset output;
  run "$PKGDB" parse descriptor --to query "$query";
  assert_success;
  assert_equal $(echo "$output" | jq -r -c '.relPath') '["hello"]';
  assert_equal $(echo "$output" | jq -r -c '.subtrees') '["legacyPackages"]';
  assert_equal $(echo "$output" | jq -r '.semver') "1.2";
}

@test "parse descriptor 'nixpkgs:hello'" {
  run "$PKGDB" parse descriptor --to manifest "nixpkgs:hello";
  assert_success;
  assert_equal $(echo "$output" | jq -r '.name') "hello";
  assert_equal $(echo "$output" | jq -r '.input.id') "nixpkgs";
  unset output;
  run "$PKGDB" parse descriptor --to query "nixpkgs:hello";
  assert_success;
  assert_equal $(echo "$output" | jq -r '.pnameOrAttrName') "hello";
}

@test "parse descriptor 'nixpkgs:hello@1.2.3'" {
  run "$PKGDB" parse descriptor --to manifest "nixpkgs:hello@1.2.3";
  assert_success;
  assert_equal $(echo "$output" | jq -r '.name') "hello";
  assert_equal $(echo "$output" | jq -r '.input.id') "nixpkgs";
  assert_equal $(echo "$output" | jq -r '.version') "1.2.3";
  unset output;
  run "$PKGDB" parse descriptor --to query "nixpkgs:hello@1.2.3";
  assert_success;
  assert_equal $(echo "$output" | jq -r '.pnameOrAttrName') "hello";
  assert_equal $(echo "$output" | jq -r '.version') "1.2.3";
}

@test "parse descriptor 'nixpkgs:hello@=1.2.3'" {
  run "$PKGDB" parse descriptor --to manifest "nixpkgs:hello@=1.2.3";
  assert_success;
  assert_equal $(echo "$output" | jq -r '.name') "hello";
  assert_equal $(echo "$output" | jq -r '.input.id') "nixpkgs";
  assert_equal $(echo "$output" | jq -r '.version') "1.2.3";
  unset output;
  run "$PKGDB" parse descriptor --to query "nixpkgs:hello@=1.2.3";
  assert_success;
  assert_equal $(echo "$output" | jq -r '.pnameOrAttrName') "hello";
  assert_equal $(echo "$output" | jq -r '.version') "1.2.3";
}

@test "parse descriptor 'nixpkgs:hello@=1.2'" {
  run "$PKGDB" parse descriptor --to manifest "nixpkgs:hello@=1.2";
  assert_success;
  assert_equal $(echo "$output" | jq -r '.name') "hello";
  assert_equal $(echo "$output" | jq -r '.input.id') "nixpkgs";
  assert_equal $(echo "$output" | jq -r '.version') "1.2";
  unset output;
  run "$PKGDB" parse descriptor --to query "nixpkgs:hello@=1.2";
  assert_success;
  assert_equal $(echo "$output" | jq -r '.pnameOrAttrName') "hello";
  assert_equal $(echo "$output" | jq -r '.version') "1.2";
}

@test "parse descriptor 'nixpkgs:hello@1.2'" {
  run "$PKGDB" parse descriptor --to manifest "nixpkgs:hello@1.2";
  assert_success;
  assert_equal $(echo "$output" | jq -r '.name') "hello";
  assert_equal $(echo "$output" | jq -r '.input.id') "nixpkgs";
  assert_equal $(echo "$output" | jq -r '.semver') "1.2";
  unset output;
  run "$PKGDB" parse descriptor --to query "nixpkgs:hello@1.2";
  assert_success;
  assert_equal $(echo "$output" | jq -r '.pnameOrAttrName') "hello";
  assert_equal $(echo "$output" | jq -r '.semver') "1.2";
}

@test "parse descriptor 'nixpkgs:hello@^1.2.3'" {
  run "$PKGDB" parse descriptor --to manifest "nixpkgs:hello@^1.2.3";
  assert_success;
  assert_equal $(echo "$output" | jq -r '.name') "hello";
  assert_equal $(echo "$output" | jq -r '.input.id') "nixpkgs";
  assert_equal $(echo "$output" | jq -r '.semver') "^1.2.3";
  unset output;
  run "$PKGDB" parse descriptor --to query "nixpkgs:hello@^1.2.3";
  assert_success;
  assert_equal $(echo "$output" | jq -r '.pnameOrAttrName') "hello";
  assert_equal $(echo "$output" | jq -r '.semver') "^1.2.3";
}

@test "parse descriptor 'nixpkgs:hello@>1.2.3'" {
  run "$PKGDB" parse descriptor --to manifest "nixpkgs:hello@>1.2.3";
  assert_success;
  assert_equal $(echo "$output" | jq -r '.name') "hello";
  assert_equal $(echo "$output" | jq -r '.input.id') "nixpkgs";
  assert_equal $(echo "$output" | jq -r '.semver') ">1.2.3";
  unset output;
  run "$PKGDB" parse descriptor --to query "nixpkgs:hello@>1.2.3";
  assert_success;
  assert_equal $(echo "$output" | jq -r '.pnameOrAttrName') "hello";
  assert_equal $(echo "$output" | jq -r '.semver') ">1.2.3";
}

@test "parse descriptor 'nixpkgs:packageset.hello@1.2.3'" {
  run "$PKGDB" parse descriptor --to manifest "nixpkgs:packageset.hello@1.2.3";
  assert_success;
  assert_equal $(echo "$output" | jq -r -c '.path') '["packageset","hello"]';
  assert_equal $(echo "$output" | jq -r '.input.id') "nixpkgs";
  assert_equal $(echo "$output" | jq -r '.version') "1.2.3";
  unset output;
  run "$PKGDB" parse descriptor --to query "nixpkgs:packageset.hello@1.2.3";
  assert_success;
  assert_equal $(echo "$output" | jq -r -c '.relPath') '["packageset","hello"]';
  assert_equal $(echo "$output" | jq -r '.version') "1.2.3";
}

@test "parse descriptor 'nixpkgs:packageset.hello@=1.2.3'" {
  run "$PKGDB" parse descriptor --to manifest "nixpkgs:packageset.hello@=1.2.3";
  assert_success;
  assert_equal $(echo "$output" | jq -r -c '.path') '["packageset","hello"]';
  assert_equal $(echo "$output" | jq -r '.input.id') "nixpkgs";
  assert_equal $(echo "$output" | jq -r '.version') "1.2.3";
  unset output;
  run "$PKGDB" parse descriptor --to query "nixpkgs:packageset.hello@=1.2.3";
  assert_success;
  assert_equal $(echo "$output" | jq -r -c '.relPath') '["packageset","hello"]';
  assert_equal $(echo "$output" | jq -r '.version') "1.2.3";
}

@test "parse descriptor 'nixpkgs:legacyPackages.*.hello@1.2.3'" {
  query="nixpkgs:legacyPackages.*.hello@1.2.3"
  run "$PKGDB" parse descriptor --to manifest "$query";
  assert_success;
  assert_equal $(echo "$output" | jq -r -c '.path') '["hello"]';
  assert_equal $(echo "$output" | jq -r -c '.subtree') "legacyPackages";
  assert_equal $(echo "$output" | jq -r '.input.id') "nixpkgs";
  assert_equal $(echo "$output" | jq -r '.version') "1.2.3";
  unset output;
  run "$PKGDB" parse descriptor --to query "$query";
  assert_success;
  assert_equal $(echo "$output" | jq -r -c '.relPath') '["hello"]';
  assert_equal $(echo "$output" | jq -r -c '.subtrees') '["legacyPackages"]';
  assert_equal $(echo "$output" | jq -r '.version') "1.2.3";
}

@test "parse descriptor 'nixpkgs:legacyPackages.aarch64-darwin.hello@1.2'" {
  query="nixpkgs:legacyPackages.aarch64-darwin.hello@1.2"
  run "$PKGDB" parse descriptor --to manifest "$query";
  assert_success;
  assert_equal $(echo "$output" | jq -r -c '.path') '["hello"]';
  assert_equal $(echo "$output" | jq -r -c '.subtree') "legacyPackages";
  assert_equal $(echo "$output" | jq -r '.input.id') "nixpkgs";
  assert_equal $(echo "$output" | jq -r '.semver') "1.2";
  unset output;
  run "$PKGDB" parse descriptor --to query "$query";
  assert_success;
  assert_equal $(echo "$output" | jq -r -c '.relPath') '["hello"]';
  assert_equal $(echo "$output" | jq -r -c '.subtrees') '["legacyPackages"]';
  assert_equal $(echo "$output" | jq -r '.semver') "1.2";
}

@test "parse descriptor 'nixpkgs:legacyPackages.*.hello@1.2'" {
  query="nixpkgs:legacyPackages.*.hello@1.2"
  run "$PKGDB" parse descriptor --to manifest "$query";
  assert_success;
  assert_equal $(echo "$output" | jq -r -c '.path') '["hello"]';
  assert_equal $(echo "$output" | jq -r -c '.subtree') "legacyPackages";
  assert_equal $(echo "$output" | jq -r '.input.id') "nixpkgs";
  assert_equal $(echo "$output" | jq -r '.semver') "1.2";
  unset output;
  run "$PKGDB" parse descriptor --to query "$query";
  assert_success;
  assert_equal $(echo "$output" | jq -r -c '.relPath') '["hello"]';
  assert_equal $(echo "$output" | jq -r -c '.subtrees') '["legacyPackages"]';
  assert_equal $(echo "$output" | jq -r '.semver') "1.2";
}

@test "parse descriptor 'myflake:packages.*.hello@1.2'" {
  query="myflake:packages.*.hello@1.2"
  run "$PKGDB" parse descriptor --to manifest "$query";
  assert_success;
  assert_equal $(echo "$output" | jq -r -c '.path') '["hello"]';
  assert_equal $(echo "$output" | jq -r -c '.subtree') "packages";
  assert_equal $(echo "$output" | jq -r '.input.id') "myflake";
  assert_equal $(echo "$output" | jq -r '.semver') "1.2";
  unset output;
  run "$PKGDB" parse descriptor --to query "$query";
  assert_success;
  assert_equal $(echo "$output" | jq -r -c '.relPath') '["hello"]';
  assert_equal $(echo "$output" | jq -r -c '.subtrees') '["packages"]';
  assert_equal $(echo "$output" | jq -r '.semver') "1.2";
}

@test "parse descriptor 'nixpkgs:legacyPackages.*.packageset.hello@1.2'" {
  query="nixpkgs:legacyPackages.*.packageset.hello@1.2"
  run "$PKGDB" parse descriptor --to manifest "$query";
  assert_success;
  assert_equal $(echo "$output" | jq -r -c '.path') '["packageset","hello"]';
  assert_equal $(echo "$output" | jq -r -c '.subtree') "legacyPackages";
  assert_equal $(echo "$output" | jq -r '.input.id') "nixpkgs";
  assert_equal $(echo "$output" | jq -r '.semver') "1.2";
  unset output;
  run "$PKGDB" parse descriptor --to query "$query";
  assert_success;
  assert_equal $(echo "$output" | jq -r -c '.relPath') '["packageset","hello"]';
  assert_equal $(echo "$output" | jq -r -c '.subtrees') '["legacyPackages"]';
  assert_equal $(echo "$output" | jq -r '.semver') "1.2";
}

@test "parse descriptor 'nixpkgs:legacyPackages.*.linuxKernel.packages.linux_4_19@1.2'" {
  # I know this package doesn't have this actual version, it's just an example of
  # a deeply nested package set
  query="nixpkgs:legacyPackages.*.linuxKernel.packages.linux_4_19@1.2"
  run "$PKGDB" parse descriptor --to manifest "$query";
  assert_success;
  assert_equal $(echo "$output" | jq -r -c '.path') '["linuxKernel","packages","linux_4_19"]';
  assert_equal $(echo "$output" | jq -r -c '.subtree') "legacyPackages";
  assert_equal $(echo "$output" | jq -r '.input.id') "nixpkgs";
  assert_equal $(echo "$output" | jq -r '.semver') "1.2";
  unset output;
  run "$PKGDB" parse descriptor --to query "$query";
  assert_success;
  assert_equal $(echo "$output" | jq -r -c '.relPath') '["linuxKernel","packages","linux_4_19"]';
  assert_equal $(echo "$output" | jq -r -c '.subtrees') '["legacyPackages"]';
  assert_equal $(echo "$output" | jq -r '.semver') "1.2";
}

@test "parse descriptor 'nixpkgs:'" {
  query="nixpkgs:";
  run "$PKGDB" parse descriptor --to manifest "$query";
  assert_failure;
  assert_output --partial "descriptor was missing a package name";
  unset output;
  run "$PKGDB" parse descriptor --to query "$query";
  assert_failure;
  assert_output --partial "descriptor was missing a package name";
}

@test "parse descriptor 'nixpkgs:.'" {
  query="nixpkgs:.";
  run "$PKGDB" parse descriptor --to manifest "$query";
  assert_failure;
  assert_output --partial "descriptor attribute name was malformed";
  unset output;
  run "$PKGDB" parse descriptor --to query "$query";
  assert_failure;
  assert_output --partial "descriptor attribute name was malformed";
}

@test "parse descriptor 'nixpkgs:..'" {
  query="nixpkgs:..";
  run "$PKGDB" parse descriptor --to manifest "$query";
  assert_failure;
  assert_output --partial "descriptor attribute name was malformed";
  unset output;
  run "$PKGDB" parse descriptor --to query "$query";
  assert_failure;
  assert_output --partial "descriptor attribute name was malformed";
}

@test "parse descriptor 'nixpkgs:*.foo'" {
  query="nixpkgs:*.foo";
  run "$PKGDB" parse descriptor --to manifest "$query";
  assert_failure;
  assert_output --partial "globs are only allowed to replace entire system names";
  unset output;
  run "$PKGDB" parse descriptor --to query "$query";
  assert_failure;
  assert_output --partial "globs are only allowed to replace entire system names";
}

@test "parse descriptor 'nixpkgs:*.foo.bar'" {
  query="nixpkgs:*.foo.bar";
  run "$PKGDB" parse descriptor --to manifest "$query";
  assert_failure;
  assert_output --partial "globs are only allowed to replace entire system names";
  unset output;
  run "$PKGDB" parse descriptor --to query "$query";
  assert_failure;
  assert_output --partial "globs are only allowed to replace entire system names";
}

@test "parse descriptor 'nixpkgs:packages.foo.*'" {
  query="nixpkgs:packages.foo.*";
  run "$PKGDB" parse descriptor --to manifest "$query";
  assert_failure;
  assert_output --partial "globs are only allowed to replace entire system names";
  unset output;
  run "$PKGDB" parse descriptor --to query "$query";
  assert_failure;
  assert_output --partial "globs are only allowed to replace entire system names";
}

@test "parse descriptor 'nixpkgs:packages.foo.b*ar'" {
  query='nixpkgs:packages.foo.b*ar';
  run "$PKGDB" parse descriptor --to manifest "$query";
  assert_failure;
  assert_output --partial "globs are only allowed to replace entire system names";
  unset output;
  run "$PKGDB" parse descriptor --to query "$query";
  assert_failure;
  assert_output --partial "globs are only allowed to replace entire system names";
}
